// VOLT desktop shell.
//
// Every line of the tracker still lives in beta2.html and runs here exactly as it does in a
// browser — same file, no copy, no build step. The shell exists for what a browser tab cannot do:
//
//   * a real origin instead of file://, so the File System Access folder pickers work and
//     AudioWorklet can load a module (both refuse to on file://)
//   * WebView2, i.e. Chromium, rather than whichever browser opens the file — measured about
//     three times faster than Firefox at the grid's DOM work
//   * songs as real files in a real folder (`store`), instead of an invisible per-origin
//     storage island only VOLT can back up
//   * the sample browser (`everything`), which needs to reach Everything's index
//   * a window with a name and an icon rather than a tab
//
// Audio latency is NOT one of them: the webview uses the same Web Audio path as Chrome, so this
// changes nothing there. See native/engine for what a native engine would actually buy, and
// native/README.md for the measured numbers.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod log;
mod cpu;
mod engine;
mod everything;
mod http;
mod plugins;
mod store;

/// The running app, for the threads that have something to tell the page but no handle to say it
/// through. The plugin thread is the one: it is started lazily, from whichever command first needs
/// a plugin, so it never receives an `AppHandle` the way a Tauri command does — and it is where a
/// plugin window's Ctrl+Delete is noticed. Set once, immediately after `build`.
pub static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// The way out, once the webview has said its goodbyes. See the close-requested handler below.
#[tauri::command]
fn app_quit() {
    log::write("app_quit — the webview finished its save-on-close; ending the process");
    // A DELIBERATE exit leaves its mark, so a plugin that merely happened to be loading at this
    // moment is not blamed for "killing the session" at the next launch — that is how Sytrus and
    // WaveShell ended up wrongly blacklisted on 2026-08-23 (the app was closed for a rebuild
    // while the song's plugins were still coming up). See plugins::blame_last_load.
    plugins::note_clean_exit();
    volt_vsthost::hard_exit(0);
}

/// When the user last asked the window to close, for the escape hatch below.
static CLOSE_ASKED: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

fn main() {
    // First thing, before anything can fail: a GUI process has no console, so without this every
    // error the native side produces is invisible and every fault has to be inferred. See log.rs.
    log::init();
    // The disposable scan worker — this same exe, re-run to open unknown plugin DLLs somewhere
    // they cannot take the app with them. MUST be decided before the Tauri builder: the
    // single-instance plugin would otherwise treat the worker as a second VOLT and neither
    // process would do what it should. See `engine::scan_in_worker`.
    if std::env::args().any(|a| a == "--scan-worker") {
        engine::scan_worker_main();
    }
    let app = tauri::Builder::default()
        .manage(engine::Engine::default())
        // One VOLT at a time, and it must be registered FIRST — the plugin decides whether this
        // process lives at all, and everything after it is setup for a process that may be about to
        // exit. A second launch raises and focuses the window you already have.
        //
        // This is not tidiness. An ASIO driver owns the hardware exclusively, so a second instance
        // cannot open the device the first holds; and when the first has hung — ASIO4ALL does not
        // always shut down cleanly — the second looks broken for reasons nothing on screen explains.
        // That is exactly what happened to Mickey: two processes, one not responding, "the volt is
        // stuck when i try to run it".
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            use tauri::Manager;
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        // **Alt+F4 / the ✕ asks the song first.** The close is intercepted, the webview gets a
        // `volt:close-req` event, and it decides: nothing unsaved → it calls `app_quit` at once;
        // something unsaved → it asks (save and close / close without saving / keep working) and
        // then calls `app_quit` or nothing. Two safety properties, both load-bearing:
        //   * the webview NEVER blocks the event loop — the dialog is the page's own, async;
        //   * a webview that cannot answer must not make the window unclosable, so a SECOND close
        //     within ten seconds bypasses the question entirely. Press Alt+F4 twice and you are
        //     out, whatever state the page is in — the husk history (2026-08-11) says never build
        //     a door that can only be opened from the inside.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let mut asked = CLOSE_ASKED.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(t) = *asked {
                    if t.elapsed() < std::time::Duration::from_secs(10) {
                        log::write("close: second ask within 10 s — closing without the question");
                        plugins::note_clean_exit(); // deliberate, same as app_quit
                        return; // do not prevent: RunEvent::ExitRequested ends the process
                    }
                }
                *asked = Some(std::time::Instant::now());
                api.prevent_close();
                use tauri::Emitter;
                let _ = window.emit("volt:close-req", ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            engine::eng_devices,
            engine::eng_open,
            engine::eng_close,
            engine::eng_load_sample,
            engine::eng_load_sample_raw,
            engine::eng_play,
            engine::eng_stop,
            engine::eng_cells,
            engine::eng_met,
            engine::eng_panic,
            engine::eng_meters,
            engine::eng_stats,
            engine::eng_tap,
            engine::vst_scan,
            engine::vst_rescan,
            engine::vst_load,
            engine::vst_unload,
            engine::eng_mix,
            engine::eng_render_vst,
            engine::eng_render_fx,
            engine::vst_editor_open,
            engine::vst_editor_close,
            engine::fxvst_load,
            engine::fxvst_unload,
            engine::fxvst_editor_open,
            engine::fxvst_editor_close,
            engine::vst_params,
            engine::fxvst_params,
            engine::vst_param_set,
            engine::fxvst_param_set,
            engine::vst_touched,
            engine::vst_patch_song,
            engine::vst_patch_flush,
            engine::fxvst_patch_removed,
            engine::fxvst_patch_swapped,
            app_quit,
            everything::ev_status,
            everything::ev_get,
            store::lib_default,
            store::lib_list,
            store::lib_read,
            store::lib_write,
            store::file_read,
        ])
        .build(tauri::generate_context!())
        .expect("VOLT failed to start");
    // Before anything can ask for it: the plugin thread emits through this.
    let _ = APP.set(app.handle().clone());
    app
        // **Die the instant the last window closes, before Tauri tears anything down.**
        //
        // `.run()` used to be given the context directly and `hard_exit` called after it returned —
        // which assumed it always does. It does not: with an ASIO device open and plugins loaded,
        // Tauri's own shutdown drops the audio context (ASIO4ALL is documented as not always
        // closing cleanly) and unwinds windows belonging to plugin DLLs, and it can wedge in there.
        // The result is a **husk**: a process with one thread left, no window, not responding,
        // still holding the audio device — and because the single-instance plugin sees it, the next
        // launch tries to focus a window that no longer exists instead of starting VOLT.
        // Caught on Mickey's machine 2026-08-11: PID 5096, one thread, zero CPU, 13 minutes dead.
        //
        // `ExitRequested` fires as soon as the last window goes, which is the earliest moment
        // nothing is needed any more. Everything durable is already on disk (the song folder mirror
        // writes on every autosave), so there is nothing to unwind for.
        .run(|_app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                log::write("window closed — ending the process now, without DLL teardown");
                volt_vsthost::hard_exit(0);
            }
        });

    // **Closing a DAW that has loaded plugins has to be done by force, and this is why.**
    //
    // Tauri's `run` returns when the last window closes, and normally that would be the end of it.
    // But once a VST3 plugin has been loaded, ending the process the ordinary way makes Windows
    // walk every loaded DLL's `DllMain(PROCESS_DETACH)` — which runs the plugin's shutdown code,
    // long after we have stopped caring. Measured on this machine: Jup-8000 V never returns from it
    // (the process hangs forever with no window, and only Task Manager clears it — exactly the
    // "the volt is stuck when i try to run it" that the single-instance plugin above exists to
    // prevent), and Vital, Diva and Kontakt come back with an access violation, which Windows
    // reports as VOLT having crashed on exit.
    //
    // `std::process::exit` runs the same teardown and does not help. So: everything durable is
    // already on disk by now — the song folder mirror writes on every autosave — and this ends the
    // process without asking anybody's DLL for permission.
    volt_vsthost::hard_exit(0);
}
