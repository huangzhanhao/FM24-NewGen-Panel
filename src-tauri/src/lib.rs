mod assets;
mod face_mapper;
mod json_store;
mod paths;
mod profile_manager;
mod replace_flow;
mod rtf_parser;
mod xml_parser;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
