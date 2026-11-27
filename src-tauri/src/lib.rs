mod alerts;
mod assets;
mod backup;
mod batch;
mod commands;
mod compose;
mod data_protection;
mod db;
mod discovery;
mod docker;
mod error;
mod files;
mod mcp;
mod metrics;
mod models;
mod notifications;
mod recording;
mod runbook;
mod scheduler;
mod security;
mod service_monitor;
mod ssh;
mod ssh_config;
mod systemd;
mod terminal;
mod transfers;
mod tunnel;
mod utils;

use alerts::AlertRegistry;
use commands::AppPaths;
use db::Database;
use models::McpCallRecord;
use runbook::RunbookExecutionRegistry;
use scheduler::RunbookScheduler;
use service_monitor::ServiceMonitorRegistry;
use std::sync::Arc;
use tauri::{Emitter, Manager};
use terminal::TerminalRegistry;
use transfers::TransferRegistry;
use tunnel::TunnelRegistry;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(TerminalRegistry::default())
        .manage(AlertRegistry::default())
        .manage(ServiceMonitorRegistry::default())
        .manage(TransferRegistry::default())
        .manage(TunnelRegistry::default())
        .manage(RunbookExecutionRegistry::default())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let database = Database::new(&data_dir.join("controlvps.sqlite3"))?;
            let _ = database.recover_interrupted_data_protection_runs();
            let paths = AppPaths {
                known_hosts: data_dir.join("known_hosts"),
                data_dir,
            };
            app.manage(RunbookScheduler::start(
                database.clone(),
                &paths,
                app.state::<RunbookExecutionRegistry>().inner().clone(),
            ));
            let mcp_settings = database.mcp_settings()?;
            let handle = app.handle().clone();
            let notify: Arc<dyn Fn(McpCallRecord) + Send + Sync> =
                Arc::new(move |record: McpCallRecord| {
                    let _ = handle.emit("mcp-call", record);
                });
            app.manage(mcp::McpServer::bootstrap(
                database.clone(),
                paths.known_hosts.clone(),
                app.state::<RunbookExecutionRegistry>().inner().clone(),
                &mcp_settings,
                Some(notify),
            ));
            app.manage(database);
            app.manage(paths);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            commands::get_mcp_status,
            commands::update_mcp_settings,
            commands::rotate_mcp_token,
            commands::list_mcp_calls,
            commands::clear_mcp_calls,
            commands::fetch_vps_deals,
            commands::create_encrypted_backup,
            commands::preview_encrypted_backup,
            commands::restore_encrypted_backup,
            commands::get_data_protection_overview,
            commands::create_data_protection_policy,
            commands::update_data_protection_policy,
            commands::delete_data_protection_policy,
            commands::initialize_data_protection,
            commands::check_data_protection,
            commands::run_data_protection_backup,
            commands::list_data_protection_snapshots,
            commands::restore_data_protection_snapshot,
            commands::list_data_protection_runs,
            commands::list_cloud_accounts,
            commands::create_cloud_account,
            commands::update_cloud_account,
            commands::delete_cloud_account,
            commands::preview_cloud_assets,
            commands::import_cloud_assets,
            commands::list_auth_profiles,
            commands::create_auth_profile,
            commands::update_auth_profile,
            commands::delete_auth_profile,
            commands::list_server_groups,
            commands::create_server_group,
            commands::update_server_group,
            commands::delete_server_group,
            commands::list_servers,
            commands::create_server,
            commands::import_ssh_config,
            commands::update_server,
            commands::delete_server,
            commands::test_connection,
            commands::diagnose_connection,
            commands::collect_metrics,
            commands::list_metric_history,
            commands::get_monitoring_settings,
            commands::update_monitoring_settings,
            commands::list_alert_events,
            commands::list_active_alerts,
            commands::list_service_monitors,
            commands::create_service_monitor,
            commands::update_service_monitor,
            commands::set_service_monitor_enabled,
            commands::delete_service_monitor,
            commands::check_service_monitor,
            commands::list_service_samples,
            commands::list_service_events,
            commands::get_service_notification_settings,
            commands::update_service_notification_settings,
            commands::test_service_webhook,
            commands::list_docker_containers,
            commands::run_docker_action,
            commands::get_docker_logs,
            commands::list_compose_projects,
            commands::run_compose_action,
            commands::get_compose_logs,
            commands::get_compose_application,
            commands::preview_compose_deployment,
            commands::deploy_compose_application,
            commands::list_compose_deployments,
            commands::rollback_compose_deployment,
            commands::list_systemd_services,
            commands::run_systemd_action,
            commands::get_systemd_logs,
            commands::collect_security_report,
            commands::run_quick_action,
            commands::list_operation_logs,
            commands::list_snippets,
            commands::create_snippet,
            commands::update_snippet,
            commands::delete_snippet,
            commands::list_runbooks,
            commands::create_runbook,
            commands::update_runbook,
            commands::delete_runbook,
            commands::run_runbook,
            commands::start_runbook_execution,
            commands::get_runbook_execution,
            commands::list_runbook_executions,
            commands::cancel_runbook_execution,
            commands::retry_runbook_execution,
            commands::list_runbook_schedules,
            commands::create_runbook_schedule,
            commands::update_runbook_schedule,
            commands::set_runbook_schedule_enabled,
            commands::delete_runbook_schedule,
            commands::run_runbook_schedule_now,
            commands::run_batch_command,
            commands::list_port_forwards,
            commands::create_port_forward,
            commands::delete_port_forward,
            commands::start_port_forward,
            commands::stop_port_forward,
            commands::list_tunnel_statuses,
            commands::open_terminal,
            commands::terminal_input,
            commands::terminal_resize,
            commands::terminal_close,
            commands::start_terminal_recording,
            commands::stop_terminal_recording,
            commands::list_terminal_recordings,
            commands::read_terminal_recording,
            commands::delete_terminal_recording,
            commands::list_remote_files,
            commands::create_remote_directory,
            commands::rename_remote_file,
            commands::delete_remote_file,
            commands::upload_remote_file,
            commands::download_remote_file,
            commands::enqueue_file_upload,
            commands::enqueue_file_download,
            commands::list_file_transfers,
            commands::cancel_file_transfer,
            commands::retry_file_transfer,
            commands::clear_finished_file_transfers,
            commands::list_notification_channels,
            commands::create_notification_channel,
            commands::update_notification_channel,
            commands::set_notification_channel_enabled,
            commands::delete_notification_channel,
            commands::test_notification_channel,
            commands::list_notification_deliveries,
            commands::clear_notification_deliveries,
            commands::get_action_center,
        ])
        .build(tauri::generate_context!())
        .expect("error while building ControlVPS")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(scheduler) = app.try_state::<RunbookScheduler>() {
                    scheduler.stop();
                }
                if let Some(mcp_server) = app.try_state::<mcp::McpServer>() {
                    mcp_server.shutdown();
                }
                if let Some(tunnels) = app.try_state::<TunnelRegistry>() {
                    tunnels.stop_all();
                }
                if let Some(terminals) = app.try_state::<TerminalRegistry>() {
                    terminals.close_all();
                }
            }
        });
}
