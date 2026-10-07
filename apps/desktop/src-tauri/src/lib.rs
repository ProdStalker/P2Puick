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
            commands::begin_receive,
            commands::cancel_transfer,
            commands::pick_files,
            commands::pick_folder,
            commands::app_info,
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
