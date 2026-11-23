mod commands;
mod db;
mod error;
mod files;
mod metrics;
mod models;
mod ssh;
mod terminal;

use commands::AppPaths;
use db::Database;
use tauri::Manager;
use terminal::TerminalRegistry;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(TerminalRegistry::default())
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
            commands::list_servers,
            commands::create_server,
            commands::update_server,
            commands::delete_server,
            commands::test_connection,
            commands::collect_metrics,
            commands::run_quick_action,
            commands::list_operation_logs,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running ControlVPS");
}
