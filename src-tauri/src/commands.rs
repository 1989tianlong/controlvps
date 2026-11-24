use std::path::PathBuf;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::alerts::{self, AlertRegistry};
use crate::batch;
use crate::db::Database;
use crate::docker;
use crate::error::{AppError, AppResult, CommandError, CommandResult};
use crate::files;
use crate::metrics;
use crate::models::{
    ActionResult, AlertEvent, AuthMethod, BatchResult, ConnectionReport, ConnectionTest,
    DockerAction, DockerActionResult, DockerLogs, DockerOverview, FileListing, FileTransfer,
    FileTransferKind, FileTransferTask, MetricSample, MonitoringSettings, OperationLog,
    PortForwardInput, PortForwardRule, QuickAction, Server, ServerInput, ServerMetrics, Snippet,
    SnippetInput, SshConfigImport, SystemdAction, SystemdActionResult, SystemdLogs, SystemdOverview,
    TerminalSession, TunnelStatus,
};
use crate::ssh;
use crate::ssh_config;
use crate::systemd;
use crate::terminal::{self, TerminalCommand, TerminalRegistry};
use crate::transfers::TransferRegistry;
use crate::tunnel::TunnelRegistry;

pub struct AppPaths {
    pub data_dir: PathBuf,
    pub known_hosts: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
    pub known_hosts_path: String,
    pub credential_store: String,
}

#[tauri::command]
pub fn get_app_info(paths: State<'_, AppPaths>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        data_dir: paths.data_dir.display().to_string(),
        known_hosts_path: paths.known_hosts.display().to_string(),
        credential_store: "macOS Keychain".into(),
    }
}

#[tauri::command]
pub fn list_servers(db: State<'_, Database>) -> CommandResult<Vec<Server>> {
    db.list_servers().map_err(Into::into)
}

#[tauri::command]
pub fn create_server(db: State<'_, Database>, input: ServerInput) -> CommandResult<Server> {
    let server = db.create_server(&input).map_err(CommandError::from)?;
    if let Err(error) = ssh::save_credentials(&server, &input) {
        let _ = db.delete_server(&server.id);
        return Err(error.into());
    }
    Ok(server)
}

#[tauri::command]
pub fn import_ssh_config(db: State<'_, Database>, path: String) -> CommandResult<SshConfigImport> {
    let content = std::fs::read_to_string(&path)
        .map_err(AppError::from)
        .map_err(CommandError::from)?;
    let (entries, mut warnings) = ssh_config::parse(&content).map_err(CommandError::from)?;
    let default_username = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let mut seen = db
        .list_servers()
        .map_err(CommandError::from)?
        .into_iter()
        .map(|server| (server.host, server.port, server.username))
        .collect::<std::collections::HashSet<_>>();
    let mut imported = Vec::new();
    let mut skipped = 0;
    for entry in entries {
        let label = entry.alias.clone();
        if let Some(reason) = entry.unsupported {
            skipped += 1;
            warnings.push(format!("已跳过 {label}：{reason}"));
            continue;
        }
        let host = entry.hostname.unwrap_or_else(|| entry.alias.clone());
        let username = entry.username.unwrap_or_else(|| default_username.clone());
        let port = entry.port.unwrap_or(22);
        if host.trim().is_empty() || username.trim().is_empty() {
            skipped += 1;
            warnings.push(format!("已跳过 {label}：主机或用户名为空"));
            continue;
        }
        if !seen.insert((host.clone(), port, username.clone())) {
            skipped += 1;
            warnings.push(format!("已跳过 {label}：相同主机、端口和用户已存在"));
            continue;
        }
        let auth_method = if entry.identity_file.is_some() {
            AuthMethod::Key
        } else {
            AuthMethod::Agent
        };
        let input = ServerInput {
            name: entry.alias,
            host,
            port,
            username,
            auth_method,
            private_key_path: entry.identity_file,
            password: None,
            key_passphrase: None,
            group_name: Some("SSH Config 导入".into()),
            tags: vec!["ssh-config".into()],
            provider: None,
            region: None,
            notes: None,
            cost: None,
            currency: None,
            expires_at: None,
            jump_server_id: None,
        };
        match db.create_server(&input) {
            Ok(server) => {
                if let Err(error) = ssh::save_credentials(&server, &input) {
                    let _ = db.delete_server(&server.id);
                    return Err(error.into());
                }
                imported.push(server);
            }
            Err(error) => {
                skipped += 1;
                warnings.push(format!("无法导入 {label}：{error}"));
            }
        }
    }
    Ok(SshConfigImport {
        imported,
        skipped,
        warnings,
    })
}

#[tauri::command]
pub fn update_server(
    db: State<'_, Database>,
    id: String,
    input: ServerInput,
) -> CommandResult<Server> {
    let server = db.update_server(&id, &input).map_err(CommandError::from)?;
    ssh::save_credentials(&server, &input).map_err(CommandError::from)?;
    Ok(server)
}

#[tauri::command]
pub fn delete_server(
    db: State<'_, Database>,
    transfers: State<'_, TransferRegistry>,
    tunnels: State<'_, TunnelRegistry>,
    id: String,
) -> CommandResult<()> {
    db.ensure_server_deletable(&id)
        .map_err(CommandError::from)?;
    transfers
        .cancel_by_server(&id)
        .map_err(CommandError::from)?;
    tunnels.stop_by_server(&id).map_err(CommandError::from)?;
    ssh::delete_credentials(&id).map_err(CommandError::from)?;
    db.delete_server(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn test_connection(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    allow_unknown_host: bool,
) -> CommandResult<ConnectionTest> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        let started = Instant::now();
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )?;
        ssh::test(&job.server, &connection, started)
    })
    .await
}

#[tauri::command]
pub async fn diagnose_connection(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    allow_unknown_host: bool,
) -> CommandResult<ConnectionReport> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        ssh::diagnose(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )
    })
    .await
}

#[tauri::command]
pub async fn collect_metrics(
    app: AppHandle,
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    alerts: State<'_, AlertRegistry>,
    id: String,
    allow_unknown_host: bool,
) -> CommandResult<ServerMetrics> {
    let job = connection_job(&db, &id)?;
    let server = job.server.clone();
    let known_hosts = paths.known_hosts.clone();
    let result = run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )?;
        metrics::collect(&job.server.id, &connection)
    })
    .await;
    match &result {
        Ok(metrics) => {
            db.insert_metric_sample(metrics).map_err(CommandError::from)?;
            let _ = alerts::record_success(&app, &db, &alerts, &server, metrics);
        }
        Err(error) => {
            let _ = alerts::record_failure(&app, &db, &alerts, &server, &error.message);
        }
    }
    result
}

#[tauri::command]
pub fn list_metric_history(
    db: State<'_, Database>,
    server_id: String,
    hours: u32,
) -> CommandResult<Vec<MetricSample>> {
    db.list_metric_history(&server_id, hours).map_err(Into::into)
}

#[tauri::command]
pub fn get_monitoring_settings(db: State<'_, Database>) -> CommandResult<MonitoringSettings> {
    db.monitoring_settings().map_err(Into::into)
}

#[tauri::command]
pub fn update_monitoring_settings(
    db: State<'_, Database>,
    settings: MonitoringSettings,
) -> CommandResult<MonitoringSettings> {
    db.update_monitoring_settings(&settings).map_err(Into::into)
}

#[tauri::command]
pub fn list_alert_events(
    db: State<'_, Database>,
    server_id: Option<String>,
) -> CommandResult<Vec<AlertEvent>> {
    db.list_alert_events(server_id.as_deref()).map_err(Into::into)
}

#[tauri::command]
pub fn list_active_alerts(db: State<'_, Database>) -> CommandResult<Vec<AlertEvent>> {
    db.list_active_alerts().map_err(Into::into)
}

#[tauri::command]
pub async fn list_docker_containers(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    allow_unknown_host: bool,
) -> CommandResult<DockerOverview> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )?;
        docker::list(&connection.session)
    })
    .await
}

#[tauri::command]
pub async fn run_docker_action(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    container_id: String,
    action: DockerAction,
    allow_unknown_host: bool,
) -> CommandResult<DockerActionResult> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    let result = run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )?;
        docker::action(&connection.session, &container_id, action)
    })
    .await;
    log_operation_result(&db, &id, "Docker 容器操作", &result, |output| {
        format!("{:?}，退出码 {}", output.action, output.exit_code)
    });
    result
}

#[tauri::command]
pub async fn get_docker_logs(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    container_id: String,
    allow_unknown_host: bool,
) -> CommandResult<DockerLogs> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )?;
        docker::logs(&connection.session, &container_id)
    })
    .await
}

#[tauri::command]
pub async fn list_systemd_services(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    allow_unknown_host: bool,
) -> CommandResult<SystemdOverview> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )?;
        systemd::list(&connection.session)
    })
    .await
}

#[tauri::command]
pub async fn run_systemd_action(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    unit: String,
    action: SystemdAction,
    allow_unknown_host: bool,
) -> CommandResult<SystemdActionResult> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    let result = run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )?;
        systemd::action(&connection.session, &unit, action)
    })
    .await;
    log_operation_result(&db, &id, "systemd 服务操作", &result, |output| {
        format!(
            "{:?} {}，退出码 {}",
            output.action, output.unit, output.exit_code
        )
    });
    result
}

#[tauri::command]
pub async fn get_systemd_logs(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    unit: String,
    allow_unknown_host: bool,
) -> CommandResult<SystemdLogs> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )?;
        systemd::logs(&connection.session, &unit)
    })
    .await
}

#[tauri::command]
pub async fn run_quick_action(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    action: QuickAction,
    allow_unknown_host: bool,
) -> CommandResult<ActionResult> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    let action_name = format!("{action:?}");
    let command = quick_action_command(action).to_string();
    let command_for_task = command.clone();
    let result = run_blocking(move || {
        let started = Instant::now();
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )?;
        let output = ssh::execute(&connection.session, &command_for_task)?;
        let mut text = output.stdout;
        if !output.stderr.trim().is_empty() {
            text.push_str(&output.stderr);
        }
        Ok(ActionResult {
            action,
            command,
            output: text,
            exit_code: output.exit_code,
            duration_ms: started.elapsed().as_millis() as u64,
        })
    })
    .await;
    let (success, summary) = match &result {
        Ok(output) => (
            output.exit_code == 0,
            format!(
                "命令退出码 {}，耗时 {} ms",
                output.exit_code, output.duration_ms
            ),
        ),
        Err(error) => (false, error.message.clone()),
    };
    let _ = db.log_operation(&id, &action_name, success, &summary);
    result
}

#[tauri::command]
pub fn list_operation_logs(
    db: State<'_, Database>,
    server_id: Option<String>,
) -> CommandResult<Vec<OperationLog>> {
    db.list_operation_logs(server_id.as_deref())
        .map_err(Into::into)
}

#[tauri::command]
pub fn list_port_forwards(db: State<'_, Database>) -> CommandResult<Vec<PortForwardRule>> {
    db.list_port_forwards().map_err(Into::into)
}

#[tauri::command]
pub fn create_port_forward(
    db: State<'_, Database>,
    input: PortForwardInput,
) -> CommandResult<PortForwardRule> {
    db.create_port_forward(&input).map_err(Into::into)
}

#[tauri::command]
pub fn delete_port_forward(db: State<'_, Database>, id: String) -> CommandResult<()> {
    db.delete_port_forward(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn start_port_forward(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    tunnels: State<'_, TunnelRegistry>,
    id: String,
    allow_unknown_host: bool,
) -> CommandResult<TunnelStatus> {
    let rule = db.get_port_forward(&id).map_err(CommandError::from)?;
    let job = connection_job(&db, &rule.server_id)?;
    let known_hosts = paths.known_hosts.clone();
    let registry = tunnels.inner().clone();
    let server_id = rule.server_id.clone();
    let result = run_blocking(move || {
        registry.start(
            &rule,
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )
    })
    .await;
    log_operation_result(&db, &server_id, "隧道启动", &result, |status| {
        format!("本地监听 {}", status.local_endpoint)
    });
    result
}

#[tauri::command]
pub fn stop_port_forward(
    db: State<'_, Database>,
    tunnels: State<'_, TunnelRegistry>,
    id: String,
) -> CommandResult<()> {
    let rule = db.get_port_forward(&id).map_err(CommandError::from)?;
    let result = tunnels.stop(&id).map_err(CommandError::from);
    log_operation_result(&db, &rule.server_id, "隧道停止", &result, |_| {
        format!("已停止 {}", rule.name)
    });
    result
}

#[tauri::command]
pub fn list_tunnel_statuses(
    tunnels: State<'_, TunnelRegistry>,
) -> CommandResult<Vec<TunnelStatus>> {
    tunnels.list().map_err(Into::into)
}

#[tauri::command]
pub fn list_snippets(db: State<'_, Database>) -> CommandResult<Vec<Snippet>> {
    db.list_snippets().map_err(Into::into)
}

#[tauri::command]
pub fn create_snippet(db: State<'_, Database>, input: SnippetInput) -> CommandResult<Snippet> {
    db.create_snippet(&input).map_err(Into::into)
}

#[tauri::command]
pub fn update_snippet(
    db: State<'_, Database>,
    id: String,
    input: SnippetInput,
) -> CommandResult<Snippet> {
    db.update_snippet(&id, &input).map_err(Into::into)
}

#[tauri::command]
pub fn delete_snippet(db: State<'_, Database>, id: String) -> CommandResult<()> {
    db.delete_snippet(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn run_batch_command(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    ids: Vec<String>,
    command: String,
    concurrency: usize,
) -> CommandResult<BatchResult> {
    if ids.is_empty() || ids.len() > 50 {
        return Err(CommandError::from(AppError::Message(
            "每次批量执行需要选择 1 到 50 台服务器".into(),
        )));
    }
    if command.trim().is_empty() || command.len() > 100_000 {
        return Err(CommandError::from(AppError::Message(
            "批量命令不能为空且不能超过 100000 字节".into(),
        )));
    }
    if !(1..=10).contains(&concurrency) {
        return Err(CommandError::from(AppError::Message(
            "并发数必须在 1 到 10 之间".into(),
        )));
    }
    let mut unique_ids = Vec::new();
    for id in ids {
        if !unique_ids.contains(&id) {
            unique_ids.push(id);
        }
    }
    let mut jobs = Vec::with_capacity(unique_ids.len());
    for id in unique_ids {
        jobs.push(connection_job(&db, &id)?);
    }
    let known_hosts = paths.known_hosts.clone();
    let result =
        run_blocking(move || batch::execute(jobs, command, known_hosts, concurrency)).await?;
    for item in &result.results {
        let summary = match item.exit_code {
            Some(code) => format!("命令退出码 {code}，耗时 {} ms", item.duration_ms),
            None => item.error.clone().unwrap_or_else(|| "连接失败".into()),
        };
        let _ = db.log_operation(&item.server_id, "批量命令", item.success, &summary);
    }
    Ok(result)
}

#[tauri::command]
pub async fn open_terminal(
    app: AppHandle,
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    allow_unknown_host: bool,
) -> CommandResult<TerminalSession> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        let registry = app.state::<TerminalRegistry>();
        terminal::open(
            app.clone(),
            &registry,
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )
    })
    .await
}

#[tauri::command]
pub fn terminal_input(
    registry: State<'_, TerminalRegistry>,
    terminal_id: String,
    data: String,
) -> CommandResult<()> {
    registry
        .send(&terminal_id, TerminalCommand::Input(data.into_bytes()))
        .map_err(Into::into)
}

#[tauri::command]
pub fn terminal_resize(
    registry: State<'_, TerminalRegistry>,
    terminal_id: String,
    columns: u32,
    rows: u32,
) -> CommandResult<()> {
    registry
        .send(&terminal_id, TerminalCommand::Resize(columns, rows))
        .map_err(Into::into)
}

#[tauri::command]
pub fn terminal_close(
    registry: State<'_, TerminalRegistry>,
    terminal_id: String,
) -> CommandResult<()> {
    registry
        .send(&terminal_id, TerminalCommand::Close)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_remote_files(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    path: Option<String>,
    allow_unknown_host: bool,
) -> CommandResult<FileListing> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )?;
        files::list(&connection.session, path.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn create_remote_directory(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    parent: String,
    name: String,
    allow_unknown_host: bool,
) -> CommandResult<String> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    let result = run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )?;
        files::create_directory(&connection.session, &parent, &name)
    })
    .await;
    log_operation_result(&db, &id, "文件新建目录", &result, |path| {
        format!("已创建 {path}")
    });
    result
}

#[tauri::command]
pub async fn rename_remote_file(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    path: String,
    new_name: String,
    allow_unknown_host: bool,
) -> CommandResult<String> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    let result = run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )?;
        files::rename(&connection.session, &path, &new_name)
    })
    .await;
    log_operation_result(&db, &id, "文件重命名", &result, |path| {
        format!("新路径 {path}")
    });
    result
}

#[tauri::command]
pub async fn delete_remote_file(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    path: String,
    recursive: bool,
    allow_unknown_host: bool,
) -> CommandResult<()> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    let summary_path = path.clone();
    let result = run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )?;
        files::delete(&connection.session, &path, recursive)
    })
    .await;
    log_operation_result(&db, &id, "文件删除", &result, |_| {
        format!("已删除 {summary_path}")
    });
    result
}

#[tauri::command]
pub async fn upload_remote_file(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    local_path: String,
    remote_directory: String,
    file_name: String,
    allow_unknown_host: bool,
) -> CommandResult<FileTransfer> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    let result = run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )?;
        files::upload(
            &connection.session,
            &local_path,
            &remote_directory,
            &file_name,
        )
    })
    .await;
    log_operation_result(&db, &id, "文件上传", &result, |transfer| {
        format!("已上传 {}，{} 字节", transfer.remote_path, transfer.bytes)
    });
    result
}

#[tauri::command]
pub async fn download_remote_file(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    remote_path: String,
    local_path: String,
    allow_unknown_host: bool,
) -> CommandResult<FileTransfer> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    let result = run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )?;
        files::download(&connection.session, &remote_path, &local_path)
    })
    .await;
    log_operation_result(&db, &id, "文件下载", &result, |transfer| {
        format!("已下载 {}，{} 字节", transfer.remote_path, transfer.bytes)
    });
    result
}

#[tauri::command]
pub fn enqueue_file_upload(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    transfers: State<'_, TransferRegistry>,
    id: String,
    local_path: String,
    remote_directory: String,
    file_name: String,
    allow_unknown_host: bool,
) -> CommandResult<FileTransferTask> {
    let job = connection_job(&db, &id)?;
    let remote_path = PathBuf::from(&remote_directory)
        .join(&file_name)
        .to_string_lossy()
        .into_owned();
    transfers
        .enqueue(
            db.inner().clone(),
            FileTransferKind::Upload,
            job,
            local_path,
            remote_path,
            file_name,
            paths.known_hosts.clone(),
            allow_unknown_host,
        )
        .map_err(Into::into)
}

#[tauri::command]
pub fn enqueue_file_download(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    transfers: State<'_, TransferRegistry>,
    id: String,
    remote_path: String,
    local_path: String,
    allow_unknown_host: bool,
) -> CommandResult<FileTransferTask> {
    let job = connection_job(&db, &id)?;
    let file_name = PathBuf::from(&remote_path)
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AppError::Message("远程文件路径无效".into()))
        .map_err(CommandError::from)?
        .to_string();
    transfers
        .enqueue(
            db.inner().clone(),
            FileTransferKind::Download,
            job,
            local_path,
            remote_path,
            file_name,
            paths.known_hosts.clone(),
            allow_unknown_host,
        )
        .map_err(Into::into)
}

#[tauri::command]
pub fn list_file_transfers(
    transfers: State<'_, TransferRegistry>,
) -> CommandResult<Vec<FileTransferTask>> {
    transfers.list().map_err(Into::into)
}

#[tauri::command]
pub fn cancel_file_transfer(
    transfers: State<'_, TransferRegistry>,
    id: String,
) -> CommandResult<FileTransferTask> {
    transfers.cancel(&id).map_err(Into::into)
}

#[tauri::command]
pub fn retry_file_transfer(
    transfers: State<'_, TransferRegistry>,
    id: String,
) -> CommandResult<FileTransferTask> {
    transfers.retry(&id).map_err(Into::into)
}

#[tauri::command]
pub fn clear_finished_file_transfers(
    transfers: State<'_, TransferRegistry>,
    server_id: String,
) -> CommandResult<()> {
    transfers.clear_finished(&server_id).map_err(Into::into)
}

fn connection_job(db: &Database, id: &str) -> CommandResult<ssh::ConnectionJob> {
    let server = db.get_server(id).map_err(CommandError::from)?;
    let credentials = ssh::load_credentials(&server).map_err(CommandError::from)?;
    let jump_host = match server.jump_server_id.as_deref() {
        Some(jump_id) => {
            let jump_server = db.get_server(jump_id).map_err(CommandError::from)?;
            let jump_credentials =
                ssh::load_credentials(&jump_server).map_err(CommandError::from)?;
            Some(ssh::JumpHost {
                server: jump_server,
                credentials: jump_credentials,
            })
        }
        None => None,
    };
    Ok(ssh::ConnectionJob {
        server,
        credentials,
        jump_host,
    })
}

fn log_operation_result<T>(
    db: &Database,
    server_id: &str,
    action: &str,
    result: &CommandResult<T>,
    success_summary: impl FnOnce(&T) -> String,
) {
    let (success, summary) = match result {
        Ok(value) => (true, success_summary(value)),
        Err(error) => (false, error.message.clone()),
    };
    let _ = db.log_operation(server_id, action, success, &summary);
}

fn quick_action_command(action: QuickAction) -> &'static str {
    match action {
        QuickAction::SystemInfo => "uname -a; printf '\\nOperating system\\n'; cat /etc/os-release 2>/dev/null; printf '\\nCPU\\n'; lscpu 2>/dev/null; printf '\\nMemory\\n'; free -h 2>/dev/null",
        QuickAction::Processes => "ps aux --sort=-%cpu 2>/dev/null | head -n 16",
        QuickAction::DiskUsage => "df -h; printf '\\nInodes\\n'; df -ih",
        QuickAction::SystemLogs => "journalctl -n 80 --no-pager 2>&1 || tail -n 80 /var/log/syslog 2>&1",
        QuickAction::CleanCache => "if command -v apt-get >/dev/null 2>&1; then apt-get clean; elif command -v dnf >/dev/null 2>&1; then dnf clean all; elif command -v yum >/dev/null 2>&1; then yum clean all; elif command -v apk >/dev/null 2>&1; then apk cache clean; else printf 'No supported package manager found\\n'; fi",
        QuickAction::Reboot => "shutdown -r now",
    }
}

async fn run_blocking<T: Send + 'static>(
    task: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> CommandResult<T> {
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| CommandError::from(AppError::Message(error.to_string())))?
        .map_err(Into::into)
}
