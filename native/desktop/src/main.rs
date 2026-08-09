// VOLT desktop shell.
//
// Deliberately almost empty. Every line of the tracker lives in beta2.html and runs here exactly
// as it does in a browser — same file, no copy, no build step. The shell exists for three things
// a browser cannot give us:
//
//   * a real origin instead of file://, so the File System Access folder pickers work and
//     AudioWorklet can load a module (both refuse to on file://)
//   * WebView2, i.e. Chromium, rather than whichever browser opens the file — measured about
//     three times faster than Firefox at the grid's DOM work
//   * a window with a name and an icon rather than a tab
//
// Audio latency is NOT one of them: the webview uses the same Web Audio path as Chrome, so this
// changes nothing there. See native/probe for what a native engine would actually buy.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .run(tauri::generate_context!())
        .expect("VOLT failed to start");
}
