mod commands;
mod state;

use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::generate_code,
            commands::start_host,
            commands::stop_host,
            commands::join_session,
            commands::discover_peers,
            commands::begin_send,
            commands::begin_send_retry,
            commands::begin_receive,
            commands::cancel_transfer,
            commands::pick_files,
            commands::pick_folder,
            commands::app_info,
            commands::lan_info,
            commands::default_excludes,
            commands::list_retry_queue,
            commands::clear_retry_queue,
            commands::remove_retry_entry,
            commands::retry_queue_file_path,
        ])
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title("P2Puick");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running P2Puick");
}
