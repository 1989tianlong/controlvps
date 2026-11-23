use std::path::PathBuf;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::db::Database;
use crate::error::{AppError, AppResult, CommandError, CommandResult};
use crate::metrics;
use crate::models::{
    ActionResult, ConnectionTest, OperationLog, QuickAction, Server, ServerInput, ServerMetrics,
    TerminalSession,
};
use crate::ssh;
use crate::terminal::{self, TerminalCommand, TerminalRegistry};

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
pub fn delete_server(db: State<'_, Database>, id: String) -> CommandResult<()> {
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
    let server = db.get_server(&id).map_err(CommandError::from)?;
    let credentials = ssh::load_credentials(&server).map_err(CommandError::from)?;
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        let started = Instant::now();
        let connection = ssh::connect(&server, &credentials, &known_hosts, allow_unknown_host)?;
        ssh::test(&server, &connection, started)
    })
    .await
}

#[tauri::command]
pub async fn collect_metrics(
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    allow_unknown_host: bool,
) -> CommandResult<ServerMetrics> {
    let server = db.get_server(&id).map_err(CommandError::from)?;
    let credentials = ssh::load_credentials(&server).map_err(CommandError::from)?;
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        let connection = ssh::connect(&server, &credentials, &known_hosts, allow_unknown_host)?;
        metrics::collect(&server.id, &connection)
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
    let server = db.get_server(&id).map_err(CommandError::from)?;
    let credentials = ssh::load_credentials(&server).map_err(CommandError::from)?;
    let known_hosts = paths.known_hosts.clone();
    let action_name = format!("{action:?}");
    let command = quick_action_command(action).to_string();
    let command_for_task = command.clone();
    let result = run_blocking(move || {
        let started = Instant::now();
        let connection = ssh::connect(&server, &credentials, &known_hosts, allow_unknown_host)?;
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
pub async fn open_terminal(
    app: AppHandle,
    db: State<'_, Database>,
    paths: State<'_, AppPaths>,
    id: String,
    allow_unknown_host: bool,
) -> CommandResult<TerminalSession> {
    let server = db.get_server(&id).map_err(CommandError::from)?;
    let credentials = ssh::load_credentials(&server).map_err(CommandError::from)?;
    let known_hosts = paths.known_hosts.clone();
    run_blocking(move || {
        let registry = app.state::<TerminalRegistry>();
        terminal::open(
            app.clone(),
            &registry,
            &server,
            &credentials,
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
