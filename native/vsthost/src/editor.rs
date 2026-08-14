//! The plugin's own window.
//!
//! A plugin editor is somebody else's UI drawn into a window we supply. On Windows that means: make
//! a plain top-level window, hand the plugin its `HWND`, and let it take over the inside. VST3 does
//! this through `IPlugView`, VST2 through its `Editor::open(parent)` — different spellings of the
//! same handshake, which is why both live behind one [`Editor`] here.
//!
//! **This is what turns a plugin from "makes its default noise" into an instrument.** Presets,
//! knobs, Kontakt's library browser, Phase Plant's patch — none of them are reachable any other
//! way; those two are silent precisely because they start empty and need their own UI to be given
//! something to play.
//!
//! ## Everything here that is load-bearing, and why
//!
//! * **The window is a separate top-level window, not a child of VOLT's.** Tauri's message loop
//!   belongs to tao/winit, and a loop that runs `IsDialogMessage`/`TranslateAccelerator` over
//!   messages bound for a plugin's window subtree eats Tab, arrows, Enter and Escape — the plugin
//!   never sees them and `onKeyDown` cannot get them back. Typing a preset name would silently not
//!   work. Its own top-level window sidesteps the whole question.
//! * **It is still created on the MAIN thread**, because that is the thread whose message loop
//!   Tauri already runs, and a window's messages are only ever delivered to the thread that
//!   created it. A window made on the audio thread would simply never receive a message.
//! * **`createView` hands back a view with a refcount of 1 — do not add-ref it.** `ComPtr::from_raw`
//!   adopts that count rather than adding to it, which is exactly right; `ComRef` would not be.
//! * **Never create a second view for one plugin.** Some crash outright. One [`Editor`] per slot,
//!   and closing HIDES rather than destroys, so re-opening is instant and no second view is ever
//!   asked for.
//! * **Tear down as `removed()` → `setFrame(null)`.** The SDK sample does it the other way round;
//!   JUCE and Ardour both do it this way, and they are the ones with the field reports.
//! * **`removed()` is only legal if `attached()` succeeded** — hence `attached: bool` rather than
//!   assuming.
//! * **`kResultTrue == kResultOk == 0` on Windows**, so every check here is `== kResultOk`. Writing
//!   `if view.attached(..)` would read exactly backwards, which is the kind of bug that looks like
//!   a broken plugin.

#![allow(non_snake_case)]

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

use vst3::Steinberg::{
    kInvalidArgument, kResultOk, IPlugFrame, IPlugFrameTrait, IPlugView, IPlugViewTrait,
    ViewRect,
};
use vst3::Steinberg::Vst::{IEditController, IEditControllerTrait, ViewType};
use vst3::{Class, ComPtr, ComRef, ComWrapper};

use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HWND, RECT};
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::winuser::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect,
    GetWindowLongPtrW, GetWindowLongW, KillTimer, LoadCursorW, RegisterClassExW,
    SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow,
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, GWL_EXSTYLE, GWL_STYLE, IDC_ARROW,
    SWP_NOMOVE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WM_CLOSE, WM_TIMER, WNDCLASSEXW, WS_CAPTION,
    WS_CLIPCHILDREN, WS_OVERLAPPED, WS_SYSMENU, WS_THICKFRAME,
};

/// Our window class name, and the flag that says it has been registered. A class may only be
/// registered once per process; every plugin window after the first reuses it.
const CLASS_NAME: &str = "VOLTPluginWindow";
static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);

/// A NUL-terminated UTF-16 string, which is what every `...W` Win32 call wants.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The timer that drives `effEditIdle`, and how often. See [`Vst2Ctx`].
const IDLE_TIMER: usize = 1;
const IDLE_MS: u32 = 30;

/// A VST2 editor, on the heap at a fixed address so the window can reach it.
///
/// **VST2 editors need `effEditIdle` called or they sit there frozen.** Unlike VST3 — where the
/// plugin runs its own timers — VST2 was designed around the host poking it a few dozen times a
/// second, and without it meters do not move, animations stop, and some editors never finish
/// painting. The host is required to do this and nothing else will.
///
/// It lives in its own `Box` rather than inside [`Editor`] because the window needs a pointer to
/// it that survives [`Editor`] being moved (it is returned by value from `open`, and then moved
/// into a registry). A `Box`'s contents do not move when the `Box` does.
struct Vst2Ctx {
    editor: Box<dyn vst::editor::Editor>,
}

/// The window procedure.
///
/// **`WM_CLOSE` hides the window instead of destroying it.** That is not laziness — it is what
/// makes "close" safe. Destroying would mean tearing the view down from inside a message handler
/// that the plugin's own code is above on the stack, and it would mean a second `createView` on
/// re-open, which some plugins do not survive. Hiding keeps one view alive for the life of the
/// plugin and makes re-opening instant. Reaper does the same thing with FX windows.
///
/// Everything else goes to `DefWindowProc`: the plugin makes its own child window inside ours and
/// handles its own painting, mouse and keys there.
unsafe extern "system" fn wndproc(hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_CLOSE {
        ShowWindow(hwnd, SW_HIDE);
        return 0;
    }
    if msg == WM_TIMER && w == IDLE_TIMER {
        // The VST2 idle tick. Null until a VST2 editor has been attached, and cleared before the
        // context is freed, so this is either a live pointer or nothing.
        let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Vst2Ctx;
        if !ctx.is_null() {
            (*ctx).editor.idle();
        }
        return 0;
    }
    DefWindowProcW(hwnd, msg, w, l)
}

/// Run one pass of this thread's Win32 message queue.
///
/// **A thread that owns plugin windows must call this regularly or they are dead glass**, and the
/// VST2 idle timer that keeps an editor's meters moving never fires. The main thread gets this for
/// free from Tauri; any other thread hosting plugins has to do it itself, which is exactly what
/// `native/desktop/src/plugins.rs` uses this for.
///
/// Returns how many messages were dispatched, which is only useful for proving the pump is alive.
pub fn pump_messages() -> u32 {
    use winapi::um::winuser::{
        DispatchMessageW, GetAncestor, GetClassNameW, PeekMessageW, TranslateMessage, GA_ROOT,
        MSG, PM_REMOVE, VK_ESCAPE, WM_KEYDOWN,
    };
    let mut n = 0;
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            // **Esc closes (hides) the plugin window it was typed into** — the DAW convention
            // (FL does exactly this), and the fix for "the plugin window has no keyboard way
            // out". Intercepted here, before dispatch, because the keystroke lands on whichever
            // of the plugin's own child windows has focus and would otherwise never reach our
            // frame's wndproc. The root-window class check keeps this from touching any window
            // that is not ours. The trade — a plugin cannot use Esc itself (e.g. to cancel
            // typing in its own text field) — is the same trade every host makes.
            if msg.message == WM_KEYDOWN && msg.wParam == VK_ESCAPE as usize {
                let root = GetAncestor(msg.hwnd, GA_ROOT);
                if !root.is_null() {
                    let mut buf = [0u16; 64];
                    let len = GetClassNameW(root, buf.as_mut_ptr(), buf.len() as i32);
                    let class = String::from_utf16_lossy(&buf[..len.max(0) as usize]);
                    if class == CLASS_NAME {
                        ShowWindow(root, SW_HIDE);
                        continue; // swallowed: hidden is the whole answer
                    }
                }
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
            n += 1;
        }
    }
    n
}

/// OLE-initialise this thread, once. Public so a thread that will host plugin editors can do it
/// before anything else — see `pump_messages`.
pub fn init_thread() {
    ensure_ole();
}

/// OLE-initialise this thread, once.
///
/// **Not optional, and the failure is a crash rather than a message.** Plugin editors draw with
/// Windows controls that assume an OLE-initialised (STA) thread — drag-and-drop, the common
/// controls, the clipboard. Sylenth1 access-violates the moment its editor opens on a thread that
/// is not. It was invisible at first because a *sweep* of every plugin works fine: some earlier
/// plugin initialises OLE for us, and everything after it inherits a thread that happens to be
/// ready. Sylenth1 alone, first in a fresh process, segfaults every time. Measured 2026-08-11.
///
/// The result is deliberately ignored. `S_FALSE` means already initialised, and
/// `RPC_E_CHANGED_MODE` means somebody (Tauri's window layer, say) already chose a different
/// apartment for this thread — in which case the thread is initialised and we are no worse off.
/// Never uninitialised: the threads that open editors live as long as the process.
fn ensure_ole() {
    thread_local! {
        static DONE: Cell<bool> = const { Cell::new(false) };
    }
    DONE.with(|d| {
        if !d.get() {
            unsafe { winapi::um::ole2::OleInitialize(std::ptr::null_mut()) };
            d.set(true);
        }
    });
}

/// Register the window class once.
fn ensure_class() -> Result<(), String> {
    if CLASS_REGISTERED.load(Ordering::Acquire) {
        return Ok(());
    }
    unsafe {
        let name = wide(CLASS_NAME);
        let mut wc: WNDCLASSEXW = std::mem::zeroed();
        wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
        // Redraw on resize: plugin editors resize themselves and a stale edge looks like a bug.
        wc.style = CS_HREDRAW | CS_VREDRAW;
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = GetModuleHandleW(std::ptr::null());
        wc.lpszClassName = name.as_ptr();
        // Without a class cursor the window inherits whatever the last one set, so the pointer
        // keeps a resize arrow (or worse) over the plugin's edges. The plugin sets its own cursor
        // over its own child window; this is only for the sliver around it.
        wc.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
        // No background brush on purpose: the plugin paints every pixel of the client area, and a
        // brush here means one frame of grey flashing through on every resize.
        if RegisterClassExW(&wc) == 0 {
            // Another thread winning the race is the only benign reason this fails, and the flag
            // below makes that a non-event. Anything else is fatal to opening a window at all.
            let err = std::io::Error::last_os_error();
            if CLASS_REGISTERED.load(Ordering::Acquire) {
                return Ok(());
            }
            return Err(format!("could not register the plugin window class: {err}"));
        }
    }
    CLASS_REGISTERED.store(true, Ordering::Release);
    Ok(())
}

/// Resize a window so its *client* area — the part the plugin draws in — is exactly `w` × `h`.
///
/// The plugin asks for the size of its own canvas and knows nothing about our title bar and
/// borders, so handing its numbers straight to `SetWindowPos` makes every editor a title-bar
/// shorter than it asked for, with its bottom row of controls cut off.
unsafe fn size_client(hwnd: HWND, w: i32, h: i32) {
    let mut r = RECT { left: 0, top: 0, right: w, bottom: h };
    let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    AdjustWindowRectEx(&mut r, style, 0, ex);
    SetWindowPos(
        hwnd,
        std::ptr::null_mut(),
        0,
        0,
        r.right - r.left,
        r.bottom - r.top,
        SWP_NOMOVE | SWP_NOZORDER,
    );
}

/// `IPlugFrame` — the small COM object a plugin calls when it wants to change its own size.
///
/// Resizable editors require this; a view given a null frame is entitled to refuse to resize, and
/// several plugins simply render wrong instead of saying so.
///
/// **The re-entrancy guard is required, not defensive.** The plugin calls `resizeView` from
/// *inside* its own `onSize`, so without the flag the sequence is `resizeView → onSize →
/// resizeView → …` until the stack runs out.
struct PlugFrame {
    hwnd: Cell<HWND>,
    resizing: Cell<bool>,
}

impl Class for PlugFrame {
    type Interfaces = (IPlugFrame,);
}

impl IPlugFrameTrait for PlugFrame {
    unsafe fn resizeView(&self, view: *mut IPlugView, newSize: *mut ViewRect) -> i32 {
        if view.is_null() || newSize.is_null() {
            return kInvalidArgument;
        }
        if self.resizing.get() {
            // Already inside a resize: the plugin is calling us back from its own onSize. Say yes
            // and unwind rather than recursing.
            return kResultOk;
        }
        self.resizing.set(true);

        let r = *newSize;
        size_client(self.hwnd.get(), r.right - r.left, r.bottom - r.top);
        // The window is the size it asked for; now tell the view to lay itself out in it. The
        // order is fixed by the spec — onSize before the window moves means the plugin lays out
        // against the old size.
        if let Some(v) = ComRef::from_raw(view) {
            v.onSize(newSize);
        }

        self.resizing.set(false);
        kResultOk
    }
}

/// Everything needed to open a plugin's window later, held on the UI thread.
///
/// This is the whole point of the controller/component split: the audio thread owns the half that
/// makes sound, and this owns the half that draws. It is created when the plugin loads and sits
/// idle until somebody asks for the window.
pub struct EditorHandle {
    inner: EditorSource,
    /// For the window's title bar.
    pub name: String,
}

enum EditorSource {
    /// The controller, add-ref'd. The audio side keeps its own reference and remains responsible
    /// for `terminate()` in the documented order — this is a second owner of the same object, not
    /// a transfer, so nothing about teardown changes.
    Vst3(ComPtr<IEditController>),
    /// The `vst` crate's editor object. It holds an `Arc` to the shared plugin-parameters handle
    /// (which the crate marks `Send + Sync`), not to the instance itself.
    Vst2(Box<dyn vst::editor::Editor>),
}

// SAFETY: a plugin's UI half is created during load (on the audio thread, where loading happens)
// and then moved once, to the main thread, where it stays for the rest of its life. One move, one
// owner — the same contract as `PluginBox` in the engine, and the same one every DAW relies on.
unsafe impl Send for EditorHandle {}

impl EditorHandle {
    pub fn vst3(controller: ComPtr<IEditController>, name: String) -> EditorHandle {
        EditorHandle { inner: EditorSource::Vst3(controller), name }
    }

    pub fn vst2(editor: Box<dyn vst::editor::Editor>, name: String) -> EditorHandle {
        EditorHandle { inner: EditorSource::Vst2(editor), name }
    }
}

/// An open plugin window.
///
/// Created once per plugin and kept: [`show`](Editor::show) and [`hide`](Editor::hide) are the
/// visible operations, and the view underneath stays attached the whole time.
pub struct Editor {
    hwnd: HWND,
    name: String,
    backend: OpenBackend,
    /// `removed()` is only legal after a successful `attached()`. Tracked rather than assumed.
    attached: bool,
}

enum OpenBackend {
    Vst3 {
        view: Option<ComPtr<IPlugView>>,
        /// Held for as long as the view is attached: the plugin keeps the pointer it was handed in
        /// `setFrame` and may call it at any time until `setFrame(null)`.
        _frame: Option<ComWrapper<PlugFrame>>,
        /// The controller this view came out of. Held so the object cannot be released while its
        /// own view is still alive.
        _controller: ComPtr<IEditController>,
    },
    /// Owned raw so its address survives this struct being moved — the window's timer holds the
    /// same pointer. Freed in `Drop`, after the timer is killed.
    Vst2(*mut Vst2Ctx),
}

// SAFETY: an `Editor` owns a window and a plugin view, and both are only ever touched from the
// main thread — every entry point in `engine.rs` goes through `run_on_main_thread` before it can
// reach one. The declaration exists so the registry that holds them can live in a `static`.
unsafe impl Send for Editor {}

impl Editor {
    /// Build the window and get the plugin drawing into it, **without showing it**.
    ///
    /// **Main thread only** — a window's messages are delivered to the thread that created it, and
    /// only the main thread has a loop pumping them.
    ///
    /// Showing is a separate step ([`show`](Editor::show)) rather than part of this, so the whole
    /// attach path — `createView`, the platform check, the frame, `attached`, `getSize` — can be
    /// exercised against a real plugin without a window appearing on anybody's screen. That is what
    /// `examples/editwin.rs` does, and it is the only way this code gets tested at all.
    pub fn open(handle: EditorHandle) -> Result<Editor, String> {
        ensure_ole();
        ensure_class()?;

        let hwnd = unsafe {
            let class = wide(CLASS_NAME);
            let title = wide(&handle.name);
            // WS_CLIPCHILDREN keeps us from painting over the plugin's own child windows, which
            // otherwise flicker on every resize.
            let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_CLIPCHILDREN;
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                640,
                480,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null_mut(),
            );
            if hwnd.is_null() {
                return Err(format!("could not create a window for {}: {}",
                                   handle.name, std::io::Error::last_os_error()));
            }
            hwnd
        };

        let name = handle.name.clone();
        let attached = match attach(hwnd, handle) {
            Ok(b) => b,
            Err(e) => {
                // The window is useless without the plugin in it, and leaving an empty one around
                // would be worse than the error message.
                unsafe { DestroyWindow(hwnd) };
                return Err(e);
            }
        };
        Ok(Editor { hwnd, name, backend: attached, attached: true })
    }

    /// The size the plugin asked for, once attached. Reported so a test can prove the editor is
    /// really there rather than merely not erroring.
    pub fn size(&self) -> (i32, i32) {
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { GetClientRect(self.hwnd, &mut r) };
        (r.right - r.left, r.bottom - r.top)
    }

    /// The plugin's name, for a caller that only has the editor.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Bring the window up, and to the front — re-opening an already-open editor should raise it,
    /// not do nothing and look broken.
    pub fn show(&self) {
        unsafe {
            ShowWindow(self.hwnd, SW_SHOW);
            SetForegroundWindow(self.hwnd);
        }
    }

    pub fn hide(&self) {
        unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }

    /// Rename the title bar. The plugin's own name is known before its window is; this keeps them
    /// in step when the instrument is renamed.
    pub fn set_title(&self, title: &str) {
        unsafe { SetWindowTextW(self.hwnd, wide(title).as_ptr()) };
    }
}

/// Make the plugin draw into `hwnd`. The format-specific half of opening.
///
/// A free function rather than a method because it consumes the handle and produces the backend:
/// there is no half-built `Editor` for it to borrow, which is what keeps the failure paths above
/// from having to unpick one.
fn attach(hwnd: HWND, handle: EditorHandle) -> Result<OpenBackend, String> {
    let name = handle.name;
    match handle.inner {
        EditorSource::Vst3(controller) => unsafe {
            let raw = controller.createView(ViewType::kEditor);
            // A null view is the plugin saying it has no editor at all. Common for utility
            // plugins, and not an error worth a stack trace — just nothing to show.
            let view = ComPtr::from_raw(raw)
                .ok_or_else(|| format!("{name} has no editor window"))?;

            if view.isPlatformTypeSupported(vst3::Steinberg::kPlatformTypeHWND) != kResultOk {
                return Err(format!("{name} will not draw into a Windows window"));
            }

            // The frame goes on BEFORE attaching: a plugin is entitled to ask its frame to resize
            // during `attached()`, and several do.
            let frame = ComWrapper::new(PlugFrame {
                hwnd: Cell::new(hwnd),
                resizing: Cell::new(false),
            });
            if let Some(fp) = frame.to_com_ptr::<IPlugFrame>() {
                view.setFrame(fp.as_ptr());
            }

            if view.attached(hwnd as *mut std::ffi::c_void, vst3::Steinberg::kPlatformTypeHWND)
                != kResultOk
            {
                view.setFrame(std::ptr::null_mut());
                return Err(format!("{name} refused to attach to the window"));
            }

            // Size the window to whatever the editor says it wants. Do this AFTER attaching:
            // plugins with a remembered size only report it once they have their window.
            let mut r: ViewRect = std::mem::zeroed();
            if view.getSize(&mut r) == kResultOk {
                let (w, h) = (r.right - r.left, r.bottom - r.top);
                if w > 0 && h > 0 {
                    size_client(hwnd, w, h);
                }
            }

            Ok(OpenBackend::Vst3 {
                view: Some(view),
                _frame: Some(frame),
                _controller: controller,
            })
        },
        EditorSource::Vst2(editor) => unsafe {
            // On the heap and staying there: the idle timer below holds this pointer, and the
            // `Editor` that owns it gets moved into a registry straight after this returns.
            let ctx = Box::into_raw(Box::new(Vst2Ctx { editor }));

            // Size first — a VST2 editor reports its size *before* it opens, the opposite order
            // from VST3, and several lay themselves out against whatever the window already is.
            let (w, h) = (*ctx).editor.size();
            if w > 0 && h > 0 {
                size_client(hwnd, w, h);
            }

            if !(*ctx).editor.open(hwnd as *mut std::ffi::c_void) {
                drop(Box::from_raw(ctx));
                return Err(format!("{name} refused to open its window"));
            }

            // Some report a useful size only once open. Ask again; ignore a repeat.
            let (w2, h2) = (*ctx).editor.size();
            if w2 > 0 && h2 > 0 && (w2, h2) != (w, h) {
                size_client(hwnd, w2, h2);
            }

            // The idle tick. **A VST2 editor without one is frozen** — see `Vst2Ctx`. The pointer
            // goes into the window before the timer starts, so the first tick cannot find null.
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, ctx as isize);
            SetTimer(hwnd, IDLE_TIMER, IDLE_MS, None);

            Ok(OpenBackend::Vst2(ctx))
        },
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        unsafe {
            match &mut self.backend {
                OpenBackend::Vst3 { view, .. } => {
                    if let Some(v) = view.as_ref() {
                        // Order matters and is NOT the SDK sample's: removed() first, then the
                        // frame goes. JUCE and Ardour both do it this way round.
                        if self.attached {
                            v.removed();
                        }
                        v.setFrame(std::ptr::null_mut());
                    }
                    // Release the view (ComPtr's Drop) while the window it drew into still
                    // exists — destroying the window first would pull the ground out from under a
                    // plugin that is still holding the HWND.
                    *view = None;
                }
                OpenBackend::Vst2(ctx) => {
                    // Stop the idle timer and unhook the pointer BEFORE freeing what it points at.
                    // Both happen on the main thread, so once the timer is dead no tick can be in
                    // flight — but leaving a dangling pointer in the window would be a crash the
                    // next time any message arrived.
                    KillTimer(self.hwnd, IDLE_TIMER);
                    SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
                    if !ctx.is_null() {
                        let mut owned = Box::from_raw(*ctx);
                        if self.attached {
                            owned.editor.close();
                        }
                        drop(owned);
                        *ctx = std::ptr::null_mut();
                    }
                }
            }
            if !self.hwnd.is_null() {
                DestroyWindow(self.hwnd);
            }
        }
    }
}
