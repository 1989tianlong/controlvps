mod alerts;
mod backup;
mod batch;
mod commands;
mod compose;
mod db;
mod docker;
mod error;
mod files;
mod metrics;
mod models;
mod runbook;
mod security;
mod service_monitor;
mod ssh;
mod ssh_config;
mod systemd;
mod terminal;
mod transfers;
mod tunnel;

use alerts::AlertRegistry;
use commands::AppPaths;
use db::Database;
use service_monitor::ServiceMonitorRegistry;
use tauri::Manager;
use terminal::TerminalRegistry;
use transfers::TransferRegistry;
use tunnel::TunnelRegistry;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(TerminalRegistry::default())
        .manage(AlertRegistry::default())
        .manage(ServiceMonitorRegistry::default())
        .manage(TransferRegistry::default())
        .manage(TunnelRegistry::default())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let database = Database::new(&data_dir.join("controlvps.sqlite3"))?;
            app.manage(database);
            app.manage(AppPaths {
                known_hosts: data_dir.join("known_hosts"),
                data_dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            commands::create_encrypted_backup,
            commands::preview_encrypted_backup,
            commands::restore_encrypted_backup,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running ControlVPS");
}
