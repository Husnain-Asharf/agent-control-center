mod bridge;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let bridge = bridge::Bridge::default();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(bridge.clone())
        .invoke_handler(tauri::generate_handler![bridge::daemon_request, bridge::daemon_connected])
        .setup(move |app| {
            let connection = bridge.run(app.handle().clone(), bridge::accd_path(), acc_protocol::data_dir());
            tauri::async_runtime::spawn(connection);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
