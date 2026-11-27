use std::path::PathBuf;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::alerts::{self, AlertRegistry};
use crate::batch;
use crate::compose;
use crate::db::Database;
use crate::discovery::{self, DealFeed};
use crate::docker;
use crate::error::{AppError, AppResult, CommandError, CommandResult};
use crate::files;
use crate::metrics;
use crate::recording::{RecordingDetail, RecordingSummary};
use crate::models::{
    ActionCenterSummary, ActionResult, AlertEvent, AuthMethod, AuthProfile, AuthProfileInput,
    BackupPreview, BackupRestoreResult, BackupSummary, BatchResult, CloudAccount,
    CloudAccountInput, CloudImportRequest, CloudImportResult, CloudPreview, ComposeAction,
    ComposeActionResult, ComposeApplication, ComposeDeploymentHistory, ComposeDeploymentPlan,
    ComposeDeploymentResult, ComposeEditInput, ComposeLogs, ComposeOverview, ConnectionReport,
    ConnectionTest, DataProtectionInput, DataProtectionOverview, DataProtectionPolicy,
    DataProtectionRestoreResult, DataProtectionRun, DockerAction, DockerActionResult, DockerLogs,
    DockerOverview, FileListing, FileTransfer, FileTransferKind, FileTransferTask, McpCallRecord,
    McpSettingsInput, MetricSample, MonitoringSettings, NotificationChannel,
    NotificationChannelInput, NotificationDelivery, OperationLog, PortForwardInput,
    PortForwardRule, QuickAction, Runbook, RunbookExecution, RunbookExecutionRequest, RunbookInput,
    RunbookResult, RunbookSchedule, RunbookScheduleInput, RunbookTrigger, SecurityReport, Server,
    ServerGroup, ServerGroupInput, ServerInput, ServerMetrics, ServiceCheckResult, ServiceEvent,
    ServiceMonitorInput, ServiceMonitorView, ServiceNotificationInput, ServiceNotificationSettings,
    ServiceSample, Snippet, SnippetInput, SshConfigImport, SystemdAction, SystemdActionResult,
    SystemdLogs, SystemdOverview, TerminalSession, TunnelStatus,
};
use crate::mcp::{McpServer, McpStatus};
use crate::notifications;
use crate::runbook::RunbookExecutionRegistry;
use crate::security;
use crate::service_monitor::{self, ServiceMonitorRegistry};
use crate::ssh;
use crate::ssh_config;
use crate::systemd;
use crate::terminal::{self, TerminalCommand, TerminalRegistry};
use crate::transfers::TransferRegistry;
use crate::tunnel::TunnelRegistry;

#[tauri::command]
pub fn get_mcp_status(server: State<'_, McpServer>) -> McpStatus {
    server.status()
}

#[tauri::command]
pub async fn update_mcp_settings(
    db: State<'_, Database>,
    server: State<'_, McpServer>,
    input: McpSettingsInput,
) -> CommandResult<McpStatus> {
    let settings = db.update_mcp_settings(&input)?;
    let server = server.inner().clone();
    run_blocking(move || Ok(server.apply(&settings))).await
}

#[tauri::command]
pub fn rotate_mcp_token(
    db: State<'_, Database>,
    server: State<'_, McpServer>,
) -> CommandResult<McpStatus> {
    let settings = db.rotate_mcp_token()?;
    Ok(server.rotate_token(&settings))
}

#[tauri::command]
pub fn list_mcp_calls(
    db: State<'_, Database>,
    limit: Option<usize>,
) -> CommandResult<Vec<McpCallRecord>> {
    db.list_mcp_calls(limit.unwrap_or(50)).map_err(Into::into)
}

#[tauri::command]
pub fn clear_mcp_calls(db: State<'_, Database>) -> CommandResult<()> {
    db.clear_mcp_calls().map_err(Into::into)
}

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
        credential_store: crate::utils::credential_store_label().into(),
    }
}

#[tauri::command]
pub async fn fetch_vps_deals() -> CommandResult<DealFeed> {
    run_blocking(discovery::fetch_deals).await
}

#[tauri::command]
pub async fn create_encrypted_backup(
    db: State<'_, Database>,
    path: String,
    password: String,
    include_credentials: bool,
) -> CommandResult<BackupSummary> {
    let database = db.inner().clone();
    run_blocking(move || {
        crate::backup::create(
            &database,
            std::path::Path::new(&path),
            &password,
            include_credentials,
        )
    })
    .await
}

#[tauri::command]
pub async fn preview_encrypted_backup(
    db: State<'_, Database>,
    path: String,
    password: String,
) -> CommandResult<BackupPreview> {
    let database = db.inner().clone();
    run_blocking(move || crate::backup::preview(&database, std::path::Path::new(&path), &password))
        .await
}

#[tauri::command]
pub async fn restore_encrypted_backup(
    db: State<'_, Database>,
    path: String,
    password: String,
) -> CommandResult<BackupRestoreResult> {
    let database = db.inner().clone();
    run_blocking(move || crate::backup::restore(&database, std::path::Path::new(&path), &password))
        .await
}

#[tauri::command]
pub async fn get_data_protection_overview(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    allow_unknown_host: bool,
) -> CommandResult<DataProtectionOverview> {
    let database = db.inner().clone();
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        crate::data_protection::overview(&database, &id, &known_hosts, allow_unknown_host)
    })
    .await
}

#[tauri::command]
pub fn create_data_protection_policy(
    db: State<'_, Database>,
    id: String,
    input: DataProtectionInput,
) -> CommandResult<DataProtectionPolicy> {
    crate::data_protection::create_policy(&db, &id, &input).map_err(Into::into)
}

#[tauri::command]
pub fn update_data_protection_policy(
    db: State<'_, Database>,
    id: String,
    input: DataProtectionInput,
) -> CommandResult<DataProtectionPolicy> {
    crate::data_protection::update_policy(&db, &id, &input).map_err(Into::into)
}

#[tauri::command]
pub fn delete_data_protection_policy(db: State<'_, Database>, id: String) -> CommandResult<()> {
    crate::data_protection::delete_policy(&db, &id).map_err(Into::into)
}

#[tauri::command]
pub async fn initialize_data_protection(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    allow_unknown_host: bool,
) -> CommandResult<DataProtectionRun> {
    let database = db.inner().clone();
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        crate::data_protection::initialize(&database, &id, &known_hosts, allow_unknown_host)
    })
    .await
}

#[tauri::command]
pub async fn check_data_protection(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    allow_unknown_host: bool,
) -> CommandResult<DataProtectionRun> {
    let database = db.inner().clone();
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        crate::data_protection::check(&database, &id, &known_hosts, allow_unknown_host)
    })
    .await
}

#[tauri::command]
pub async fn run_data_protection_backup(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    allow_unknown_host: bool,
) -> CommandResult<DataProtectionRun> {
    let database = db.inner().clone();
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        crate::data_protection::backup(&database, &id, &known_hosts, allow_unknown_host)
    })
    .await
}

#[tauri::command]
pub async fn list_data_protection_snapshots(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    allow_unknown_host: bool,
) -> CommandResult<Vec<crate::models::ResticSnapshot>> {
    let database = db.inner().clone();
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        crate::data_protection::snapshots(&database, &id, &known_hosts, allow_unknown_host)
    })
    .await
}

#[tauri::command]
pub async fn restore_data_protection_snapshot(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    snapshot_id: String,
    target: Option<String>,
    allow_unknown_host: bool,
) -> CommandResult<DataProtectionRestoreResult> {
    let database = db.inner().clone();
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        crate::data_protection::restore(
            &database,
            &id,
            &snapshot_id,
            target.as_deref(),
            &known_hosts,
            allow_unknown_host,
        )
    })
    .await
}

#[tauri::command]
pub fn list_data_protection_runs(
    db: State<'_, Database>,
    id: String,
) -> CommandResult<Vec<DataProtectionRun>> {
    db.list_data_protection_runs(&id).map_err(Into::into)
}

#[tauri::command]
pub fn list_cloud_accounts(db: State<'_, Database>) -> CommandResult<Vec<CloudAccount>> {
    db.list_cloud_accounts().map_err(Into::into)
}

#[tauri::command]
pub fn create_cloud_account(
    db: State<'_, Database>,
    input: CloudAccountInput,
) -> CommandResult<CloudAccount> {
    crate::assets::create_account(&db, &input).map_err(Into::into)
}

#[tauri::command]
pub fn update_cloud_account(
    db: State<'_, Database>,
    id: String,
    input: CloudAccountInput,
) -> CommandResult<CloudAccount> {
    crate::assets::update_account(&db, &id, &input).map_err(Into::into)
}

#[tauri::command]
pub fn delete_cloud_account(db: State<'_, Database>, id: String) -> CommandResult<()> {
    crate::assets::delete_account(&db, &id).map_err(Into::into)
}

#[tauri::command]
pub async fn preview_cloud_assets(
    db: State<'_, Database>,
    id: String,
) -> CommandResult<CloudPreview> {
    let database = db.inner().clone();
    run_blocking(move || crate::assets::preview(&database, &id)).await
}

#[tauri::command]
pub async fn import_cloud_assets(
    db: State<'_, Database>,
    request: CloudImportRequest,
) -> CommandResult<CloudImportResult> {
    let database = db.inner().clone();
    run_blocking(move || crate::assets::import(&database, &request)).await
}

#[tauri::command]
pub fn list_auth_profiles(db: State<'_, Database>) -> CommandResult<Vec<AuthProfile>> {
    db.list_auth_profiles().map_err(Into::into)
}

#[tauri::command]
pub fn create_auth_profile(
    db: State<'_, Database>,
    input: AuthProfileInput,
) -> CommandResult<AuthProfile> {
    crate::assets::create_profile(&db, &input).map_err(Into::into)
}

#[tauri::command]
pub fn update_auth_profile(
    db: State<'_, Database>,
    id: String,
    input: AuthProfileInput,
) -> CommandResult<AuthProfile> {
    crate::assets::update_profile(&db, &id, &input).map_err(Into::into)
}

#[tauri::command]
pub fn delete_auth_profile(db: State<'_, Database>, id: String) -> CommandResult<()> {
    crate::assets::delete_profile(&db, &id).map_err(Into::into)
}

#[tauri::command]
pub fn list_server_groups(db: State<'_, Database>) -> CommandResult<Vec<ServerGroup>> {
    db.list_server_groups().map_err(Into::into)
}

#[tauri::command]
pub fn create_server_group(
    db: State<'_, Database>,
    input: ServerGroupInput,
) -> CommandResult<ServerGroup> {
    crate::assets::create_group(&db, &input).map_err(Into::into)
}

#[tauri::command]
pub fn update_server_group(
    db: State<'_, Database>,
    id: String,
    input: ServerGroupInput,
) -> CommandResult<ServerGroup> {
    crate::assets::update_group(&db, &id, &input).map_err(Into::into)
}

#[tauri::command]
pub fn delete_server_group(db: State<'_, Database>, id: String) -> CommandResult<()> {
    db.delete_server_group(&id).map_err(Into::into)
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
            jump_chain: Vec::new(),
            auth_profile_id: None,
            cloud_account_id: None,
            cloud_resource_id: None,
            inherit_port: false,
            inherit_username: false,
            inherit_jump: false,
            inherit_auth: false,
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
    terminals: State<'_, TerminalRegistry>,
    transfers: State<'_, TransferRegistry>,
    tunnels: State<'_, TunnelRegistry>,
    id: String,
) -> CommandResult<()> {
    db.ensure_server_deletable(&id)
        .map_err(CommandError::from)?;
    // Capture policy IDs before DB cascade delete, but only wipe Keychain secrets
    // AFTER the database deletion succeeds so a failed delete never destroys credentials.
    let policy_ids: Vec<String> = db
        .list_data_protection_policies(&id)
        .unwrap_or_default()
        .into_iter()
        .map(|policy| policy.id)
        .collect();
    db.delete_server(&id).map_err(CommandError::from)?;
    terminals.close_by_server(&id);
    let _ = transfers.cancel_by_server(&id);
    let _ = tunnels.stop_by_server(&id);
    let _ = ssh::delete_credentials(&id);
    for policy_id in policy_ids {
        crate::data_protection::delete_policy_secrets_by_id(&policy_id);
    }
    Ok(())
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
            &job.jumps,
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
            &job.jumps,
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
            &job.jumps,
            &known_hosts,
            allow_unknown_host,
        )?;
        metrics::collect(&job.server.id, &connection)
    })
    .await;
    match &result {
        Ok(metrics) => {
            db.insert_metric_sample(metrics)
                .map_err(CommandError::from)?;
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
    db.list_metric_history(&server_id, hours)
        .map_err(Into::into)
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
    db.list_alert_events(server_id.as_deref())
        .map_err(Into::into)
}

#[tauri::command]
pub fn list_active_alerts(db: State<'_, Database>) -> CommandResult<Vec<AlertEvent>> {
    db.list_active_alerts().map_err(Into::into)
}

#[tauri::command]
pub fn list_service_monitors(db: State<'_, Database>) -> CommandResult<Vec<ServiceMonitorView>> {
    db.list_service_monitors().map_err(Into::into)
}

#[tauri::command]
pub fn create_service_monitor(
    db: State<'_, Database>,
    input: ServiceMonitorInput,
) -> CommandResult<ServiceMonitorView> {
    db.create_service_monitor(&input).map_err(Into::into)
}

#[tauri::command]
pub fn update_service_monitor(
    db: State<'_, Database>,
    id: String,
    input: ServiceMonitorInput,
) -> CommandResult<ServiceMonitorView> {
    db.update_service_monitor(&id, &input).map_err(Into::into)
}

#[tauri::command]
pub fn set_service_monitor_enabled(
    db: State<'_, Database>,
    id: String,
    enabled: bool,
) -> CommandResult<ServiceMonitorView> {
    db.set_service_monitor_enabled(&id, enabled)
        .map_err(Into::into)
}

#[tauri::command]
pub fn delete_service_monitor(db: State<'_, Database>, id: String) -> CommandResult<()> {
    db.delete_service_monitor(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn check_service_monitor(
    app: AppHandle,
    db: State<'_, Database>,
    registry: State<'_, ServiceMonitorRegistry>,
    id: String,
) -> CommandResult<ServiceCheckResult> {
    let database = db.inner().clone();
    let registry = registry.inner().clone();
    run_blocking(move || service_monitor::check(&app, &database, &registry, &id)).await
}

#[tauri::command]
pub fn list_service_samples(
    db: State<'_, Database>,
    id: String,
    hours: u32,
) -> CommandResult<Vec<ServiceSample>> {
    db.list_service_samples(&id, hours).map_err(Into::into)
}

#[tauri::command]
pub fn list_service_events(
    db: State<'_, Database>,
    id: Option<String>,
) -> CommandResult<Vec<ServiceEvent>> {
    db.list_service_events(id.as_deref()).map_err(Into::into)
}

#[tauri::command]
pub fn get_service_notification_settings(
    db: State<'_, Database>,
) -> CommandResult<ServiceNotificationSettings> {
    service_monitor::notification_settings(&db).map_err(Into::into)
}

#[tauri::command]
pub fn update_service_notification_settings(
    db: State<'_, Database>,
    input: ServiceNotificationInput,
) -> CommandResult<ServiceNotificationSettings> {
    service_monitor::save_notification_settings(&db, &input).map_err(Into::into)
}

#[tauri::command]
pub async fn test_service_webhook(db: State<'_, Database>) -> CommandResult<()> {
    let database = db.inner().clone();
    run_blocking(move || service_monitor::test_webhook(&database)).await
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
            &job.jumps,
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
            &job.jumps,
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
            &job.jumps,
            &known_hosts,
            allow_unknown_host,
        )?;
        docker::logs(&connection.session, &container_id)
    })
    .await
}

#[tauri::command]
pub async fn list_compose_projects(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    allow_unknown_host: bool,
) -> CommandResult<ComposeOverview> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            &job.jumps,
            &known_hosts,
            allow_unknown_host,
        )?;
        docker::compose_list(&connection.session)
    })
    .await
}

#[tauri::command]
pub async fn run_compose_action(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    project: String,
    action: ComposeAction,
    allow_unknown_host: bool,
) -> CommandResult<ComposeActionResult> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    let result = run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            &job.jumps,
            &known_hosts,
            allow_unknown_host,
        )?;
        docker::compose_action(&connection.session, &project, action)
    })
    .await;
    log_operation_result(&db, &id, "Docker Compose 项目操作", &result, |output| {
        format!(
            "{:?} {}，退出码 {}",
            output.action, output.project, output.exit_code
        )
    });
    result
}

#[tauri::command]
pub async fn get_compose_logs(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    project: String,
    allow_unknown_host: bool,
) -> CommandResult<ComposeLogs> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            &job.jumps,
            &known_hosts,
            allow_unknown_host,
        )?;
        let mut logs = docker::compose_logs(&connection.session, &project)?;
        logs.output =
            compose::redact_project_output(&connection.session, &id, &project, &logs.output)?;
        Ok(logs)
    })
    .await
}

#[tauri::command]
pub async fn get_compose_application(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    project: String,
    allow_unknown_host: bool,
) -> CommandResult<ComposeApplication> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            &job.jumps,
            &known_hosts,
            allow_unknown_host,
        )?;
        compose::read(&connection.session, &id, &project)
    })
    .await
}

#[tauri::command]
pub async fn preview_compose_deployment(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    input: ComposeEditInput,
    allow_unknown_host: bool,
) -> CommandResult<ComposeDeploymentPlan> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            &job.jumps,
            &known_hosts,
            allow_unknown_host,
        )?;
        compose::preview(&connection.session, input)
    })
    .await
}

#[tauri::command]
pub async fn deploy_compose_application(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    input: ComposeEditInput,
    allow_unknown_host: bool,
) -> CommandResult<ComposeDeploymentResult> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    let operation_id = id.clone();
    let result = run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            &job.jumps,
            &known_hosts,
            allow_unknown_host,
        )?;
        compose::deploy(&connection.session, &operation_id, input)
    })
    .await;
    match &result {
        Ok(output) => {
            let _ = db.log_operation(
                &id,
                "Compose 应用部署",
                output.record.status == crate::models::ComposeDeploymentStatus::Succeeded,
                &format!(
                    "{} · {} · {:?}",
                    output.record.project, output.record.id, output.record.status
                ),
            );
        }
        Err(error) => {
            let _ = db.log_operation(&id, "Compose 应用部署", false, &error.message);
        }
    }
    result
}

#[tauri::command]
pub async fn list_compose_deployments(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    project: String,
    allow_unknown_host: bool,
) -> CommandResult<ComposeDeploymentHistory> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            &job.jumps,
            &known_hosts,
            allow_unknown_host,
        )?;
        compose::history(&connection.session, &project)
    })
    .await
}

#[tauri::command]
pub async fn rollback_compose_deployment(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    project: String,
    deployment_id: String,
    expected_revision: String,
    allow_unknown_host: bool,
) -> CommandResult<ComposeDeploymentResult> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    let operation_id = id.clone();
    let result = run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            &job.jumps,
            &known_hosts,
            allow_unknown_host,
        )?;
        compose::rollback(
            &connection.session,
            &operation_id,
            &project,
            &deployment_id,
            &expected_revision,
        )
    })
    .await;
    match &result {
        Ok(output) => {
            let _ = db.log_operation(
                &id,
                "Compose 应用回滚",
                output.record.status == crate::models::ComposeDeploymentStatus::Succeeded,
                &format!(
                    "{} · {} · {:?}",
                    output.record.project, output.record.id, output.record.status
                ),
            );
        }
        Err(error) => {
            let _ = db.log_operation(&id, "Compose 应用回滚", false, &error.message);
        }
    }
    result
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
            &job.jumps,
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
            &job.jumps,
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
            &job.jumps,
            &known_hosts,
            allow_unknown_host,
        )?;
        systemd::logs(&connection.session, &unit)
    })
    .await
}

#[tauri::command]
pub async fn collect_security_report(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    allow_unknown_host: bool,
) -> CommandResult<SecurityReport> {
    let job = connection_job(&db, &id)?;
    let known_hosts = paths.known_hosts.clone();
    let result = run_blocking(move || {
        let connection = ssh::connect(
            &job.server,
            &job.credentials,
            &job.jumps,
            &known_hosts,
            allow_unknown_host,
        )?;
        security::collect(&job.server.id, &connection.session)
    })
    .await;
    log_operation_result(&db, &id, "补丁与安全只读审计", &result, |report| {
        format!(
            "{} 项更新，{} 个监听端口，{} 条提示",
            report.updates.len(),
            report.listening_ports.len(),
            report.warnings.len()
        )
    });
    result
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
            &job.jumps,
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
pub async fn delete_port_forward(
    db: State<'_, Database>,
    tunnels: State<'_, TunnelRegistry>,
    id: String,
) -> CommandResult<()> {
    let registry = tunnels.inner().clone();
    let stop_id = id.clone();
    let _ = run_blocking(move || registry.stop(&stop_id)).await;
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
            &job.jumps,
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
pub async fn stop_port_forward(
    db: State<'_, Database>,
    tunnels: State<'_, TunnelRegistry>,
    id: String,
) -> CommandResult<()> {
    let rule = db.get_port_forward(&id).map_err(CommandError::from)?;
    let registry = tunnels.inner().clone();
    let stop_id = id.clone();
    let result = run_blocking(move || registry.stop(&stop_id)).await;
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
pub fn list_runbooks(db: State<'_, Database>) -> CommandResult<Vec<Runbook>> {
    db.list_runbooks().map_err(Into::into)
}

#[tauri::command]
pub fn create_runbook(db: State<'_, Database>, input: RunbookInput) -> CommandResult<Runbook> {
    db.create_runbook(&input).map_err(Into::into)
}

#[tauri::command]
pub fn update_runbook(
    db: State<'_, Database>,
    id: String,
    input: RunbookInput,
) -> CommandResult<Runbook> {
    db.update_runbook(&id, &input).map_err(Into::into)
}

#[tauri::command]
pub fn delete_runbook(db: State<'_, Database>, id: String) -> CommandResult<()> {
    db.delete_runbook(&id).map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn start_runbook_execution(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    registry: State<'_, RunbookExecutionRegistry>,
    id: String,
    ids: Vec<String>,
    concurrency: usize,
    canary_count: usize,
    timeout_seconds: u64,
) -> CommandResult<RunbookExecution> {
    let runbook = db.get_runbook(&id).map_err(CommandError::from)?;
    let request = RunbookExecutionRequest {
        runbook,
        target_ids: ids,
        concurrency,
        canary_count,
        timeout_seconds,
        trigger: RunbookTrigger::Manual,
        schedule_id: None,
        retry_of: None,
    };
    crate::runbook::start_execution(
        db.inner().clone(),
        paths.known_hosts.clone(),
        registry.inner().clone(),
        request,
    )
    .map_err(Into::into)
}

#[tauri::command]
pub fn get_runbook_execution(
    db: State<'_, Database>,
    id: String,
) -> CommandResult<RunbookExecution> {
    db.get_runbook_execution(&id).map_err(Into::into)
}

#[tauri::command]
pub fn list_runbook_executions(
    db: State<'_, Database>,
    runbook_id: Option<String>,
) -> CommandResult<Vec<RunbookExecution>> {
    db.list_runbook_executions(runbook_id.as_deref())
        .map_err(Into::into)
}

#[tauri::command]
pub fn cancel_runbook_execution(
    db: State<'_, Database>,
    registry: State<'_, RunbookExecutionRegistry>,
    id: String,
) -> CommandResult<RunbookExecution> {
    registry.cancel(&id).map_err(CommandError::from)?;
    db.get_runbook_execution(&id).map_err(Into::into)
}

#[tauri::command]
pub fn retry_runbook_execution(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    registry: State<'_, RunbookExecutionRegistry>,
    id: String,
) -> CommandResult<RunbookExecution> {
    let request = db.runbook_retry_request(&id).map_err(CommandError::from)?;
    crate::runbook::start_execution(
        db.inner().clone(),
        paths.known_hosts.clone(),
        registry.inner().clone(),
        request,
    )
    .map_err(Into::into)
}

#[tauri::command]
pub fn list_runbook_schedules(db: State<'_, Database>) -> CommandResult<Vec<RunbookSchedule>> {
    db.list_runbook_schedules().map_err(Into::into)
}

#[tauri::command]
pub fn create_runbook_schedule(
    db: State<'_, Database>,
    input: RunbookScheduleInput,
) -> CommandResult<RunbookSchedule> {
    db.create_runbook_schedule(&input).map_err(Into::into)
}

#[tauri::command]
pub fn update_runbook_schedule(
    db: State<'_, Database>,
    id: String,
    input: RunbookScheduleInput,
) -> CommandResult<RunbookSchedule> {
    db.update_runbook_schedule(&id, &input).map_err(Into::into)
}

#[tauri::command]
pub fn set_runbook_schedule_enabled(
    db: State<'_, Database>,
    id: String,
    enabled: bool,
) -> CommandResult<RunbookSchedule> {
    db.set_runbook_schedule_enabled(&id, enabled)
        .map_err(Into::into)
}

#[tauri::command]
pub fn delete_runbook_schedule(db: State<'_, Database>, id: String) -> CommandResult<()> {
    db.delete_runbook_schedule(&id).map_err(Into::into)
}

#[tauri::command]
pub fn run_runbook_schedule_now(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    registry: State<'_, RunbookExecutionRegistry>,
    id: String,
) -> CommandResult<RunbookExecution> {
    let schedule = db.get_runbook_schedule(&id).map_err(CommandError::from)?;
    if schedule
        .last_execution_id
        .as_deref()
        .is_some_and(|execution_id| registry.contains(execution_id))
    {
        return Err(AppError::Message("上一次执行仍未结束，不能重复启动".into()).into());
    }
    let request = RunbookExecutionRequest {
        runbook: db
            .get_runbook(&schedule.runbook_id)
            .map_err(CommandError::from)?,
        target_ids: schedule.server_ids,
        concurrency: schedule.concurrency,
        canary_count: schedule.canary_count,
        timeout_seconds: schedule.timeout_seconds,
        trigger: RunbookTrigger::Scheduled,
        schedule_id: Some(schedule.id.clone()),
        retry_of: None,
    };
    let execution = crate::runbook::start_execution(
        db.inner().clone(),
        paths.known_hosts.clone(),
        registry.inner().clone(),
        request,
    )
    .map_err(CommandError::from)?;
    db.mark_runbook_schedule_started(&id, &execution.id)
        .map_err(CommandError::from)?;
    Ok(execution)
}

#[tauri::command]
pub async fn run_runbook(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    ids: Vec<String>,
    concurrency: usize,
    canary_count: usize,
) -> CommandResult<RunbookResult> {
    if ids.is_empty() || ids.len() > 50 {
        return Err(CommandError::from(AppError::Message(
            "每次 Runbook 执行需要选择 1 到 50 台服务器".into(),
        )));
    }
    if !(1..=10).contains(&concurrency) {
        return Err(CommandError::from(AppError::Message(
            "并发数必须在 1 到 10 之间".into(),
        )));
    }
    let runbook = db.get_runbook(&id).map_err(CommandError::from)?;
    let mut unique_ids = Vec::new();
    for server_id in ids {
        if !unique_ids.contains(&server_id) {
            unique_ids.push(server_id);
        }
    }
    if canary_count > 10 || canary_count >= unique_ids.len() {
        return Err(CommandError::from(AppError::Message(
            "灰度首批必须为 0 到 10 台，且小于目标总数".into(),
        )));
    }
    let mut jobs = Vec::with_capacity(unique_ids.len());
    for server_id in unique_ids {
        jobs.push(connection_job(&db, &server_id)?);
    }
    let known_hosts = paths.known_hosts.clone();
    let result = run_blocking(move || {
        crate::runbook::execute(jobs, runbook, known_hosts, concurrency, canary_count)
    })
    .await?;
    for server in &result.results {
        if let Some(error) = &server.error {
            let _ = db.log_operation(
                &server.server_id,
                &format!("Runbook · {}", result.runbook_name),
                false,
                error,
            );
        }
        for step in &server.steps {
            let summary = match step.exit_code {
                Some(code) => format!("退出码 {code}，耗时 {} ms", step.duration_ms),
                None => step.error.clone().unwrap_or_else(|| "执行失败".into()),
            };
            let _ = db.log_operation(
                &server.server_id,
                &format!("Runbook · {} · {}", result.runbook_name, step.step_name),
                step.success,
                &summary,
            );
        }
    }
    Ok(result)
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
            &job.jumps,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingHandle {
    pub id: String,
    pub started_at: String,
}

#[tauri::command]
pub fn start_terminal_recording(
    paths: State<'_, AppPaths>,
    registry: State<'_, TerminalRegistry>,
    terminal_id: String,
) -> CommandResult<RecordingHandle> {
    let info = registry.session(&terminal_id).map_err(CommandError::from)?;
    let (recorder, id, started_at) = crate::recording::Recorder::create(
        &paths.data_dir,
        &info.server_id,
        &info.server_name,
        info.columns,
        info.rows,
    )
    .map_err(CommandError::from)?;
    if let Err(error) = registry.send(
        &terminal_id,
        TerminalCommand::Record(Some(Box::new(recorder))),
    ) {
        let _ = crate::recording::delete(&paths.data_dir, &id);
        return Err(CommandError::from(error));
    }
    Ok(RecordingHandle { id, started_at })
}

#[tauri::command]
pub fn stop_terminal_recording(
    registry: State<'_, TerminalRegistry>,
    terminal_id: String,
) -> CommandResult<()> {
    registry
        .send(&terminal_id, TerminalCommand::Record(None))
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_terminal_recordings(
    paths: State<'_, AppPaths>,
) -> CommandResult<Vec<RecordingSummary>> {
    let data_dir = paths.data_dir.clone();
    run_blocking(move || crate::recording::list(&data_dir)).await
}

#[tauri::command]
pub async fn read_terminal_recording(
    paths: State<'_, AppPaths>,
    id: String,
) -> CommandResult<RecordingDetail> {
    let data_dir = paths.data_dir.clone();
    run_blocking(move || crate::recording::read(&data_dir, &id)).await
}

#[tauri::command]
pub fn delete_terminal_recording(
    paths: State<'_, AppPaths>,
    id: String,
) -> CommandResult<()> {
    crate::recording::delete(&paths.data_dir, &id).map_err(Into::into)
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
            &job.jumps,
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
            &job.jumps,
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
            &job.jumps,
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
            &job.jumps,
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
            &job.jumps,
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
            &job.jumps,
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

#[allow(clippy::too_many_arguments)]
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
        .enqueue(crate::transfers::TransferEnqueueRequest {
            db: db.inner().clone(),
            kind: FileTransferKind::Upload,
            job,
            local_path,
            remote_path,
            file_name,
            known_hosts: paths.known_hosts.clone(),
            allow_unknown_host,
        })
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
        .enqueue(crate::transfers::TransferEnqueueRequest {
            db: db.inner().clone(),
            kind: FileTransferKind::Download,
            job,
            local_path,
            remote_path,
            file_name,
            known_hosts: paths.known_hosts.clone(),
            allow_unknown_host,
        })
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

#[tauri::command]
pub fn list_notification_channels(
    db: State<'_, Database>,
) -> CommandResult<Vec<NotificationChannel>> {
    db.list_notification_channels().map_err(Into::into)
}

#[tauri::command]
pub fn create_notification_channel(
    db: State<'_, Database>,
    input: NotificationChannelInput,
) -> CommandResult<NotificationChannel> {
    notifications::create_channel(&db, &input).map_err(Into::into)
}

#[tauri::command]
pub fn update_notification_channel(
    db: State<'_, Database>,
    id: String,
    input: NotificationChannelInput,
) -> CommandResult<NotificationChannel> {
    notifications::update_channel(&db, &id, &input).map_err(Into::into)
}

#[tauri::command]
pub fn set_notification_channel_enabled(
    db: State<'_, Database>,
    id: String,
    enabled: bool,
) -> CommandResult<NotificationChannel> {
    db.set_notification_channel_enabled(&id, enabled)
        .map_err(Into::into)
}

#[tauri::command]
pub fn delete_notification_channel(db: State<'_, Database>, id: String) -> CommandResult<()> {
    notifications::delete_channel(&db, &id).map_err(Into::into)
}

#[tauri::command]
pub async fn test_notification_channel(
    db: State<'_, Database>,
    id: String,
) -> CommandResult<NotificationDelivery> {
    let database = db.inner().clone();
    run_blocking(move || notifications::send_test(&database, &id)).await
}

#[tauri::command]
pub fn list_notification_deliveries(
    db: State<'_, Database>,
    limit: Option<usize>,
) -> CommandResult<Vec<NotificationDelivery>> {
    db.list_notification_deliveries(limit.unwrap_or(50))
        .map_err(Into::into)
}

#[tauri::command]
pub fn clear_notification_deliveries(db: State<'_, Database>) -> CommandResult<()> {
    db.clear_notification_deliveries().map_err(Into::into)
}

#[tauri::command]
pub fn get_action_center(db: State<'_, Database>) -> CommandResult<ActionCenterSummary> {
    db.action_center_summary().map_err(Into::into)
}

fn connection_job(db: &Database, id: &str) -> CommandResult<ssh::ConnectionJob> {
    crate::runbook::connection_job(db, id).map_err(CommandError::from)
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
