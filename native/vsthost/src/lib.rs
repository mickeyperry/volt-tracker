//! VOLT's plugin host — VST3 and VST2 behind one front door.
//!
//! A VST3 plugin is a DLL that exports one useful function, `GetPluginFactory`, which hands back a
//! COM object listing the classes inside it. Instantiate one of those classes, ask it for
//! `IAudioProcessor`, tell it the sample rate and block size, and then hand it buffers and note
//! events a block at a time. That is the whole shape of it; everything below is that sequence with
//! the error handling written down.
//!
//! VST2 (in `vst2.rs`) is the same job over an older, smaller API. The point of this file's
//! [`Module`] and [`Plugin`] types is that **nothing upstream knows which format it is holding**:
//! `Module::open` picks the loader from the file itself (`.dll` is VST2, everything else VST3),
//! and both formats process through the same event list and the same buffer shape. The engine's
//! slot map, `VstNode`, the desktop commands and the song's `ins.vst` all stay format-blind.
//!
//! **Why this crate is separate from `volt-engine`.** The engine is a transliteration of VOLT's own
//! instruments and is graded by null tests against the browser. A plugin is somebody else's code
//! running in our process — it can be slow, it can be wrong, and it can crash. Keeping it behind
//! its own crate boundary means the scanning tool that finds out *which* of Mickey's 129 plugins
//! misbehave is a separate binary from the thing playing his songs.
//!
//! **On safety.** Almost everything here is `unsafe`: it is COM, over FFI, into code we did not
//! write. The rule followed throughout is that every raw pointer coming out of a plugin is checked
//! against null and every `tresult` is checked against `kResultOk` before anything is done with it,
//! because the alternative is a crash in someone's session with no way to tell which plugin did it.

use std::ffi::{c_void, OsStr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use vst3::Steinberg::Vst::{
    AudioBusBuffers, BusDirections_, BusInfo, IAudioProcessor, IAudioProcessorTrait, IComponent,
    IComponentTrait, IConnectionPoint, IConnectionPointTrait, IEditController, IEditControllerTrait,
    MediaTypes_, ProcessContext, ProcessContext_::StatesAndFlags_, ProcessData, ProcessModes_,
    ProcessSetup, SpeakerArr, SpeakerArrangement, SymbolicSampleSizes_,
};
use vst3::Steinberg::{
    kResultOk, FUnknown, IPluginBaseTrait, IPluginFactory, IPluginFactory2, IPluginFactory2Trait,
    IPluginFactoryTrait, PClassInfo, PClassInfo2, TUID,
};
use vst3::{ComPtr, Interface};

pub mod editor;
pub mod events;
pub mod host;
pub mod params;
pub mod stream;
pub mod vst2;
pub use editor::{Editor, EditorHandle};
pub use events::HostEvents;
pub use params::HostParams;
pub use host::Host;
pub use stream::Stream;
pub use vst2::{scan_vst2, Vst2Scan, VST2_DIRS};

/// Where Windows keeps VST3 plugins. The spec fixes this path, which is why a scan needs no
/// configuration — and why Mickey's 129 plugins were countable before any of this was written.
pub const VST3_DIR: &str = r"C:\Program Files\Common Files\VST3";

/// Which API a plugin speaks. Carried on [`PluginInfo`] so a caller *can* ask, and shaped so it
/// never has to: load by path, play through the same calls, and the difference stays in this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginFormat {
    Vst3,
    Vst2,
}

/// One plugin class found by a scan. A single `.vst3` file can contain several — an instrument and
/// its own effect, say — so this is per class, not per file. (A VST2 DLL is always one plugin,
/// `index` 0.)
#[derive(Debug, Clone)]
pub struct PluginInfo {
    /// The DLL to load. For a bundle this is the binary *inside* it, not the folder.
    pub path: PathBuf,
    /// Class index within the factory, which is how `load` finds it again.
    pub index: i32,
    pub name: String,
    /// `Audio Module Class` for anything that processes audio. Effects and instruments both.
    pub category: String,
    /// Where instrument-vs-effect actually lives: a VSTi says `Instrument` in here.
    pub subcategories: String,
    pub vendor: String,
    pub cid: [i8; 16],
    pub format: PluginFormat,
}

impl PluginInfo {
    /// A VSTi — something that turns notes into audio, which is what VOLT wants first.
    ///
    /// The VST3 spec has no "is an instrument" flag; it has a free-text subcategory string, by
    /// convention containing `Instrument` for a VSTi (`Instrument|Synth`, `Instrument|Sampler`).
    /// So this is a substring match, because that is genuinely what the format offers.
    pub fn is_instrument(&self) -> bool {
        self.subcategories.contains("Instrument")
    }
}

/// A loaded plugin DLL.
///
/// **The library is deliberately never unloaded.** Measured on this machine: releasing the plugin
/// object works perfectly for every plugin tried, and then `FreeLibrary` on the DLL crashes the
/// process with an illegal instruction — Vital, Diva and Kontakt all do it. This is not a VOLT bug
/// and not a plugin bug so much as a fact about the format: plugins spin up their own threads,
/// register their own TLS destructors and hand out pointers into their own code, and unmapping that
/// while any of it is still live is undefined. Real hosts do not unload plugin binaries either.
///
/// The cost of not unloading is bounded and small: Windows reference-counts a loaded DLL, so it is
/// one mapping per distinct plugin *file* used in a session, however many instances are made of it.
/// The cost of unloading is a crash in the middle of somebody's song.
pub struct Vst3Module {
    // Field order IS the drop order in Rust, and here it is load-bearing rather than stylistic.
    factory: ComPtr<IPluginFactory>,
    /// `Option` only so `Drop` can take it out and forget it. Never `None` while in use.
    lib: Option<libloading::Library>,
    path: PathBuf,
}

impl Drop for Vst3Module {
    fn drop(&mut self) {
        if let Some(lib) = self.lib.take() {
            // Leak the library on purpose. See the note on `Vst3Module`.
            //
            // **And do NOT call `ExitDll` here**, tempting as it looks. `ExitDll` is the module's
            // own teardown and it pairs with actually unloading; calling it while leaving the DLL
            // mapped leaves the module half dead, and anything touching it afterwards is reading a
            // torn-down runtime. Measured: with `ExitDll` called as each scanned module was
            // dropped, Diva and Phase Plant — which had just been fixed and were playing — went
            // back to crashing, this time before they had even been reached. Without it, every
            // plugin tried exits cleanly.
            std::mem::forget(lib);
        }
    }
}

/// The two exports a VST3 DLL offers that this host uses. `GetPluginFactory` is the one that
/// matters; `InitDll` is the module's setup and is skipped at your peril — some plugins allocate
/// their globals in it. (`ExitDll` is deliberately never called — see `Drop for Module`.)
type GetFactoryFn = unsafe extern "system" fn() -> *mut IPluginFactory;
type InitDllFn = unsafe extern "system" fn() -> bool;

impl Vst3Module {
    /// Open a plugin DLL and take its factory.
    ///
    /// Returns an `Arc` rather than a bare module, and that is not a convenience: every plugin
    /// keeps a handle to the module it came from, so the DLL cannot be unloaded while an instance
    /// of it is still alive. Getting that wrong is not a leak, it is a crash — the plugin's own
    /// destructor lives in the code being unmapped. It cost a hung process to find.
    pub fn open(path: &Path) -> Result<Arc<Vst3Module>, String> {
        let dll = binary_in(path).ok_or_else(|| format!("no VST3 binary in {}", path.display()))?;

        // SAFETY: this loads and runs someone else's code, and there is no version of this that is
        // safe in the Rust sense. It is why the scanner is its own process.
        let lib = unsafe { libloading::Library::new(&dll) }
            .map_err(|e| format!("{}: {e}", dll.display()))?;

        unsafe {
            if let Ok(init) = lib.get::<InitDllFn>(b"InitDll\0") {
                if !init() {
                    return Err(format!("{}: InitDll refused", dll.display()));
                }
            }

            let get_factory = lib
                .get::<GetFactoryFn>(b"GetPluginFactory\0")
                .map_err(|_| format!("{}: not a VST3 plugin (no GetPluginFactory)", dll.display()))?;

            let raw = get_factory();
            let factory = ComPtr::from_raw(raw)
                .ok_or_else(|| format!("{}: GetPluginFactory returned null", dll.display()))?;

            Ok(Arc::new(Vst3Module { factory, lib: Some(lib), path: dll }))
        }
    }

    /// Every class this module offers.
    pub fn classes(&self) -> Vec<PluginInfo> {
        let mut out = Vec::new();
        unsafe {
            // IPluginFactory2 carries the subcategories — the only place "is this an instrument"
            // is written down. Every plugin worth hosting implements it; the fallback to plain
            // PClassInfo exists so an ancient one still shows up in a scan rather than vanishing.
            let f2 = self.factory.cast::<IPluginFactory2>();
            let count = self.factory.countClasses();
            for index in 0..count {
                let info = match &f2 {
                    Some(f2) => {
                        let mut ci: PClassInfo2 = std::mem::zeroed();
                        if f2.getClassInfo2(index, &mut ci) != kResultOk {
                            continue;
                        }
                        PluginInfo {
                            path: self.path.clone(),
                            index,
                            name: cstr(&ci.name),
                            category: cstr(&ci.category),
                            subcategories: cstr(&ci.subCategories),
                            vendor: cstr(&ci.vendor),
                            cid: ci.cid,
                            format: PluginFormat::Vst3,
                        }
                    }
                    None => {
                        let mut ci: PClassInfo = std::mem::zeroed();
                        if self.factory.getClassInfo(index, &mut ci) != kResultOk {
                            continue;
                        }
                        PluginInfo {
                            path: self.path.clone(),
                            index,
                            name: cstr(&ci.name),
                            category: cstr(&ci.category),
                            subcategories: String::new(),
                            vendor: String::new(),
                            cid: ci.cid,
                            format: PluginFormat::Vst3,
                        }
                    }
                };
                out.push(info);
            }
        }
        out
    }

    /// Instantiate one class and get it ready to process at `sample_rate` in blocks of at most
    /// `max_block`.
    ///
    /// The order of these calls is fixed by the spec and plugins are entitled to crash if it is
    /// wrong: create → initialize → activate the buses → setupProcessing → setActive(true) →
    /// setProcessing(true). Getting `setActive` before `setupProcessing` is the classic way to
    /// make a synth produce silence and no error.
    pub fn instantiate(
        self: &Arc<Self>,
        info: &PluginInfo,
        sample_rate: f64,
        max_block: i32,
    ) -> Result<Vst3Plugin, String> {
        unsafe {
            let mut obj: *mut c_void = std::ptr::null_mut();
            let res = self.factory.createInstance(
                info.cid.as_ptr(),
                IComponent::IID.as_ptr() as *const i8,
                &mut obj,
            );
            if res != kResultOk || obj.is_null() {
                return Err(format!("{}: createInstance failed ({res})", info.name));
            }
            let component = ComPtr::<IComponent>::from_raw(obj as *mut IComponent)
                .ok_or_else(|| format!("{}: null component", info.name))?;

            // The host context. This used to be null — the spec allows it — and it crashed u-he
            // Diva and Phase Plant outright: both ask the context for IHostApplication the moment
            // they start and use the answer without checking it. See host.rs.
            let host = Host::new();
            if component.initialize(host.as_ptr() as *mut FUnknown) != kResultOk {
                return Err(format!("{}: initialize failed", info.name));
            }

            let processor = component
                .cast::<IAudioProcessor>()
                .ok_or_else(|| format!("{}: no IAudioProcessor", info.name))?;

            if processor.canProcessSampleSize(SymbolicSampleSizes_::kSample32 as i32) != kResultOk {
                return Err(format!("{}: will not process 32-bit float", info.name));
            }

            // Tell the plugin what its buses are wired to, before activating them. Stereo in and
            // out for everything: VOLT's mixer is stereo and a plugin that cannot do stereo is not
            // one we can host anyway. Plugins are allowed to refuse an arrangement, and several do
            // for their side-chain or aux buses — that is why the result is not checked. What
            // matters is that they were ASKED: a strict plugin that was never told its arrangement
            // is entitled to sit there outputting nothing.
            let mut ins = vec![SpeakerArr::kStereo as SpeakerArrangement;
                               component.getBusCount(MediaTypes_::kAudio as i32, BusDirections_::kInput as i32).max(0) as usize];
            let mut outs = vec![SpeakerArr::kStereo as SpeakerArrangement;
                                component.getBusCount(MediaTypes_::kAudio as i32, BusDirections_::kOutput as i32).max(0) as usize];
            processor.setBusArrangements(
                ins.as_mut_ptr(), ins.len() as i32,
                outs.as_mut_ptr(), outs.len() as i32,
            );

            // Turn on every audio and event bus the plugin has. A synth with its event input left
            // inactive is within its rights to ignore every note it is sent, which looks exactly
            // like a broken host.
            for (media, dir) in [
                (MediaTypes_::kAudio, BusDirections_::kInput),
                (MediaTypes_::kAudio, BusDirections_::kOutput),
                (MediaTypes_::kEvent, BusDirections_::kInput),
                (MediaTypes_::kEvent, BusDirections_::kOutput),
            ] {
                let n = component.getBusCount(media as i32, dir as i32);
                for i in 0..n {
                    component.activateBus(media as i32, dir as i32, i, 1);
                }
            }

            let mut setup = ProcessSetup {
                processMode: ProcessModes_::kRealtime as i32,
                symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
                maxSamplesPerBlock: max_block,
                sampleRate: sample_rate,
            };
            if processor.setupProcessing(&mut setup) != kResultOk {
                return Err(format!("{}: setupProcessing failed", info.name));
            }
            if component.setActive(1) != kResultOk {
                return Err(format!("{}: setActive failed", info.name));
            }
            processor.setProcessing(1);

            // The controller. Two halves of a plugin are created separately and do not know about
            // each other until the host introduces them, and a plugin whose controller never saw
            // the component's state is a plugin sitting on default-of-nothing — which is why so
            // many of them load, process, and output silence.
            let controller = make_controller(&self.factory, &component, &host, &info.name);

            // The touch log's ear: the controller reports every knob its editor moves through
            // this handler. Set AFTER the controller is initialized (make_controller), kept alive
            // in the plugin struct for as long as the controller might call it.
            let tag = next_tag();
            let edit_hooks = host::EditHooks::new(tag);
            if let Some(c) = &controller {
                c.setComponentHandler(edit_hooks.as_ptr());
            }

            // The automatable parameters, captured NOW — the controller lives with the audio side
            // and nothing on the UI thread can ask it later. Capped: Kontakt-sized plugins expose
            // thousands, and a list nobody can scroll is not a feature.
            let param_list = controller
                .as_ref()
                .map(|c| {
                    use vst3::Steinberg::Vst::ParameterInfo_::ParameterFlags_ as PF;
                    let n = c.getParameterCount().clamp(0, 4096);
                    let mut out = Vec::new();
                    for i in 0..n {
                        let mut pi: vst3::Steinberg::Vst::ParameterInfo = std::mem::zeroed();
                        if c.getParameterInfo(i, &mut pi) != kResultOk {
                            continue;
                        }
                        if pi.flags & PF::kCanAutomate == 0
                            || pi.flags & (PF::kIsReadOnly | PF::kIsHidden) != 0
                        {
                            continue;
                        }
                        out.push(ParamDesc { id: pi.id, name: utf16z(&pi.title) });
                    }
                    out
                })
                .unwrap_or_default();

            let out_channels = bus_channels(&component, BusDirections_::kOutput as i32);
            let in_channels = bus_channels(&component, BusDirections_::kInput as i32);
            let in_bus_chans = all_bus_channels(&component, BusDirections_::kInput as i32);
            let out_bus_chans = all_bus_channels(&component, BusDirections_::kOutput as i32);
            let event_in_buses =
                component.getBusCount(MediaTypes_::kEvent as i32, BusDirections_::kInput as i32).max(0) as usize;
            let parameters = controller.as_ref().map(|c| c.getParameterCount().max(0) as usize).unwrap_or(0);
            let latency = processor.getLatencySamples() as u32;

            Ok(Vst3Plugin {
                component,
                processor,
                name: info.name.clone(),
                out_channels,
                in_channels,
                in_bus_chans,
                out_bus_chans,
                sc_in: Vec::new(),
                sc_in_ptrs: Vec::new(),
                sc_aux_out: Vec::new(),
                sc_aux_ptrs: Vec::new(),
                sc_main_ptrs: Vec::new(),
                sc_bus_in: Vec::new(),
                sc_bus_out: Vec::new(),
                latency,
                max_block,
                event_in_buses,
                parameters,
                param_list,
                pending_params: Vec::new(),
                host_params: HostParams::new(),
                tag,
                _edit_hooks: edit_hooks,
                ctx: {
                    let mut c: ProcessContext = std::mem::zeroed();
                    c.state = (StatesAndFlags_::kPlaying
                        | StatesAndFlags_::kTempoValid
                        | StatesAndFlags_::kProjectTimeMusicValid) as u32;
                    c.sampleRate = sample_rate;
                    c.tempo = 120.0;
                    c.timeSigNumerator = 4;
                    c.timeSigDenominator = 4;
                    c
                },
                controller,
                _host: host,
                _module: Arc::clone(self),
            })
        }
    }
}

/// A live VST3 instance, set up and processing.
pub struct Vst3Plugin {
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    pub name: String,
    /// Channels on the FIRST output bus. Multi-bus plugins exist; VOLT feeds one strip, so the
    /// main bus is the one that matters and the rest are ignored until something needs them.
    pub out_channels: usize,
    pub in_channels: usize,
    /// Channel count of EVERY audio bus, in bus order — `ProcessData` must present them ALL
    /// (sidechains included, as silence) or a strict plugin refuses to process. See `process_io`.
    in_bus_chans: Vec<usize>,
    out_bus_chans: Vec<usize>,
    /// Per-block scratch, allocated on the first block and REUSED — the render thread must not
    /// touch the allocator (see the underrun note in `process_io`). Raw pointers in here are
    /// refreshed every block before use and never live past the `process` call.
    sc_in: Vec<Vec<Vec<f32>>>,
    sc_in_ptrs: Vec<Vec<*mut f32>>,
    sc_aux_out: Vec<Vec<Vec<f32>>>,
    sc_aux_ptrs: Vec<Vec<*mut f32>>,
    sc_main_ptrs: Vec<*mut f32>,
    sc_bus_in: Vec<AudioBusBuffers>,
    sc_bus_out: Vec<AudioBusBuffers>,
    /// How many event (note) input buses the plugin has. **Zero means it cannot be played** — no
    /// amount of correct note scheduling reaches a plugin with nowhere to put notes, and the
    /// symptom is silence that looks exactly like a broken host.
    pub event_in_buses: usize,
    /// How many parameters its controller exposes. Zero usually means the controller never
    /// initialized, which is worth knowing before blaming the audio path.
    pub parameters: usize,
    /// The automatable ones, id + display name, captured at instantiate (see there for why).
    pub param_list: Vec<ParamDesc>,
    /// Changes waiting for the next block — see [`Vst3Plugin::set_param`].
    pending_params: Vec<(u32, f64)>,
    /// The COM object those changes ride in. Pooled; allocates only as distinct ids appear.
    host_params: HostParams,
    /// This instance's address in the touch log — see `next_tag`.
    pub tag: u64,
    /// The component handler the controller was given; alive as long as it might be called.
    _edit_hooks: host::EditHooks,
    /// What the plugin says it delays by. Has to be compensated or everything it touches drifts —
    /// see stage E in ROADMAP. Reported here, not yet acted on.
    pub latency: u32,
    max_block: i32,
    /// Where the transport is, as far as the plugin is concerned. Advanced by `process`.
    ///
    /// Handing a plugin a null `processContext` is legal and is what this host did first. It is
    /// also a good way to get silence out of an arpeggiator, a tempo-synced LFO or anything that
    /// asks "is the host playing?" before it does anything — and there is no way to tell that kind
    /// of silence from a broken host. So VOLT always presents a running transport.
    ctx: ProcessContext,
    /// The plugin's controller half, when it has one. Holds the parameters, knows how to describe
    /// a preset, and — via `createView` — is what draws the plugin's own window.
    ///
    /// **Stays owned here even though `take_editor` hands a reference to the UI thread.** COM is
    /// refcounted, so both may hold it; what must not be duplicated is the *teardown*, and that
    /// remains this struct's `Drop` in the documented order (disconnect, terminate controller,
    /// terminate component). The UI side only ever drops a reference.
    controller: Option<ComPtr<IEditController>>,
    /// The host context this instance was initialized with. Plugins keep the pointer they were
    /// given and may call back into it at any time before `terminate`, so it has to outlive them.
    _host: Host,
    /// The module this instance came out of, kept alive for exactly as long as the instance is.
    ///
    /// **Field order is drop order in Rust, and it is load-bearing here.** The COM pointers above
    /// are released first — including the `Drop` impl below, which calls back into the plugin —
    /// and only then can this last reference go and the DLL unload. The other way round unmaps the
    /// code that the plugin's own destructor lives in. Never read; holding it IS the job.
    _module: Arc<Vst3Module>,
}

impl Vst3Plugin {
    /// Render one block. `events` carries the notes for this block; `input` is what the plugin's
    /// main input bus hears (planar; empty for an instrument); `out` is filled with
    /// `out_channels` planar buffers of `frames` samples.
    ///
    /// Deliberately takes the output buffers rather than allocating: this is called from the audio
    /// callback in the end, where an allocation is a dropout.
    pub fn process_io(
        &mut self,
        events: &HostEvents,
        input: &[&[f32]],
        out: &mut [Vec<f32>],
        frames: usize,
    ) -> Result<(), String> {
        if frames as i32 > self.max_block {
            return Err(format!("{}: block of {frames} exceeds the {} it was set up for",
                               self.name, self.max_block));
        }
        for ch in out.iter_mut() {
            ch.resize(frames, 0.0);
            ch.fill(0.0);
        }

        unsafe {
            // **Every declared bus is presented, not just the main one.** `ProcessData`'s
            // numInputs/numOutputs are the COMPONENT's audio bus counts; the arrays must have an
            // `AudioBusBuffers` per bus, sidechains and aux outs included. A strict plugin handed
            // fewer is entitled to refuse the block — apQualizr2 (apulSoft) declares a sidechain
            // input, was handed `numInputs: 1`, and answered every block with silence: "apq on
            // drums not showing and mutes the channel" (2026-08-13, proven by fxprobe before and
            // after). Aux buses carry honest silence with the silence flag set; WaveShell and
            // friends never cared, apQualizr2 does, and the spec sides with apQualizr2.
            //
            // **Everything below reuses scratch allocated on the FIRST block.** The first version
            // of the bus arrays allocated half a dozen Vecs per 128-sample block on the render
            // thread — hundreds of allocations a second per plugin — and an allocator stall in
            // the callback is an underrun, audible only where the music is: "extreme distortion
            // ... only where the note is" on a sparse bass line (2026-08-13, after every offline
            // layer measured clean). Sizes never change after instantiate, so after warmup this
            // path allocates NOTHING.
            //
            // Main input bus: the caller's audio, mono repeated across a wider bus the same way
            // the graph up-mixes (a mono strip into a stereo Pro-Q must be both ears, not the
            // left one). Instruments (no input at all) still get their event path below.
            let in_buses_n = self.in_bus_chans.len();
            if self.sc_in.len() != in_buses_n {
                self.sc_in = self
                    .in_bus_chans
                    .iter()
                    .map(|&chans| vec![Vec::with_capacity(self.max_block as usize); chans])
                    .collect();
            }
            for (bus, store) in self.sc_in.iter_mut().enumerate() {
                let chans = store.len();
                for (c, chan) in store.iter_mut().enumerate() {
                    chan.resize(frames, 0.0);
                    if bus == 0 && !input.is_empty() {
                        // **A plugin with FEWER inputs than the bus carries gets a DOWNMIX, not
                        // channel 0.** Taking `input[0]` threw the right channel in the bin: a
                        // MONO plugin anywhere in the path collapsed everything to the left, and
                        // anything panned right vanished outright. On the MASTER insert that is
                        // the entire mix — which is exactly what Mickey hit (2026-08-15: "still
                        // mono with vsts, it gives only left, if moving to right it becomes
                        // muted", with L2 Mono sitting on his master). Speakers downmix, as the
                        // Web Audio spec defines it: mono = (L+R)/2.
                        if chans == 1 && input.len() > 1 {
                            let n = input.iter().map(|s| s.len()).min().unwrap_or(0).min(frames);
                            let k = 1.0 / input.len() as f32;
                            for i in 0..n {
                                chan[i] = input.iter().map(|s| s[i]).sum::<f32>() * k;
                            }
                            chan[n..].fill(0.0);
                        } else {
                            // wider than the input (mono strip into a stereo plugin): repeat, so
                            // both ears hear it rather than only the left one
                            let src = input[c.min(input.len() - 1)];
                            let n = src.len().min(frames);
                            chan[..n].copy_from_slice(&src[..n]);
                            chan[n..].fill(0.0);
                        }
                    } else {
                        chan.fill(0.0);
                    }
                }
            }
            while self.sc_in_ptrs.len() < self.sc_in.len() {
                self.sc_in_ptrs.push(Vec::new());
            }
            for (store, ptrs) in self.sc_in.iter_mut().zip(self.sc_in_ptrs.iter_mut()) {
                ptrs.clear();
                ptrs.extend(store.iter_mut().map(|c| c.as_mut_ptr()));
            }
            self.sc_bus_in.clear();
            for (bus, ptrs) in self.sc_in_ptrs.iter_mut().enumerate() {
                self.sc_bus_in.push(AudioBusBuffers {
                    numChannels: ptrs.len() as i32,
                    // **The MAIN bus never claims silence, even when it is silent.** The flag is
                    // an invitation to skip the block, and what a skipping plugin skips includes
                    // ADVANCING ITS OWN STATE — lookahead delay lines, oversampler history,
                    // envelope followers. An insert on a note-based channel goes silent between
                    // every pair of notes, so the flag flapped audio→silence→audio in note
                    // rhythm, and every plugin that took the shortcut resumed each note with a
                    // stale delay line: a burp per note start. That was THE distortion of
                    // 2026-08-14 — on every device, with the plugin bypassed (bypass still
                    // processes), never offline (a pre-built voice keeps the input active), and
                    // never on the Sytrus channel (an instrument node renders every block, so
                    // its insert never saw the flag flip). A DAW insert runs CONTINUOUSLY;
                    // zeroed buffers marked as audio cost the plugin its shortcut and nothing
                    // else. Aux buses (sidechains we have nothing to feed) keep the honest flag.
                    silenceFlags: if bus == 0 { 0 } else { u64::MAX },
                    __field0: vst3::Steinberg::Vst::AudioBusBuffers__type0 {
                        channelBuffers32: ptrs.as_mut_ptr(),
                    },
                });
            }

            // Output buses: bus 0 is the caller's, aux buses render into scratch nobody reads.
            if self.sc_aux_out.len() + 1 != self.out_bus_chans.len().max(1) {
                self.sc_aux_out = self
                    .out_bus_chans
                    .iter()
                    .skip(1)
                    .map(|&chans| vec![Vec::with_capacity(self.max_block as usize); chans])
                    .collect();
            }
            for store in self.sc_aux_out.iter_mut() {
                for chan in store.iter_mut() {
                    chan.resize(frames, 0.0);
                }
            }
            self.sc_main_ptrs.clear();
            self.sc_main_ptrs.extend(out.iter_mut().map(|c| c.as_mut_ptr()));
            while self.sc_aux_ptrs.len() < self.sc_aux_out.len() {
                self.sc_aux_ptrs.push(Vec::new());
            }
            for (store, ptrs) in self.sc_aux_out.iter_mut().zip(self.sc_aux_ptrs.iter_mut()) {
                ptrs.clear();
                ptrs.extend(store.iter_mut().map(|c| c.as_mut_ptr()));
            }
            self.sc_bus_out.clear();
            self.sc_bus_out.push(AudioBusBuffers {
                numChannels: self.sc_main_ptrs.len() as i32,
                silenceFlags: 0,
                __field0: vst3::Steinberg::Vst::AudioBusBuffers__type0 {
                    channelBuffers32: self.sc_main_ptrs.as_mut_ptr(),
                },
            });
            for ptrs in self.sc_aux_ptrs.iter_mut() {
                self.sc_bus_out.push(AudioBusBuffers {
                    numChannels: ptrs.len() as i32,
                    silenceFlags: 0,
                    __field0: vst3::Steinberg::Vst::AudioBusBuffers__type0 {
                        channelBuffers32: ptrs.as_mut_ptr(),
                    },
                });
            }

            let list = events.com_ptr();
            // Automation: whatever `set_param` queued since the last block rides in as input
            // parameter changes — the ONLY door to a VST3 processor's parameters. An empty list
            // is passed as null, the shape every plugin was tested against before this existed.
            self.host_params.clear();
            for (id, v) in self.pending_params.drain(..) {
                self.host_params.set(id, v);
            }
            let param_list = if self.host_params.is_empty() {
                std::ptr::null_mut()
            } else {
                self.host_params.com_ptr()
            };
            let ctx = &mut self.ctx as *mut ProcessContext;
            let mut data = ProcessData {
                processMode: ProcessModes_::kRealtime as i32,
                symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
                numSamples: frames as i32,
                numInputs: in_buses_n as i32,
                numOutputs: self.sc_bus_out.len() as i32,
                inputs: if self.sc_bus_in.is_empty() { std::ptr::null_mut() } else { self.sc_bus_in.as_mut_ptr() },
                outputs: self.sc_bus_out.as_mut_ptr(),
                inputParameterChanges: param_list,
                outputParameterChanges: std::ptr::null_mut(),
                inputEvents: list,
                outputEvents: std::ptr::null_mut(),
                processContext: ctx,
            };

            let res = self.processor.process(&mut data);
            if res != kResultOk {
                return Err(format!("{}: process returned {res}", self.name));
            }

            // Move the transport on. A plugin that syncs to the host reads this every block, and a
            // context that never advances looks like a permanently paused transport.
            self.ctx.projectTimeSamples += frames as i64;
            self.ctx.continousTimeSamples += frames as i64;
            self.ctx.projectTimeMusic =
                self.ctx.projectTimeSamples as f64 / self.ctx.sampleRate * (self.ctx.tempo / 60.0);
        }
        Ok(())
    }

    /// Queue one normalised parameter change for the next block. Audio-thread safe: no COM call
    /// happens here, only a push; `process_io` turns the queue into `inputParameterChanges`.
    pub fn set_param(&mut self, id: u32, value: f64) {
        self.pending_params.push((id, value));
    }

    /// Drop whatever the plugin is holding internally — ringing voices, delay lines, reverb
    /// tails. VST3's convention is the `setProcessing` toggle, which the spec explicitly allows
    /// from the processing thread; plugins flush their buffers on the off→on edge.
    pub fn flush_state(&mut self) {
        unsafe {
            self.processor.setProcessing(0);
            self.processor.setProcessing(1);
        }
    }

    /// The component's state — see [`Plugin::state`].
    fn state(&self) -> Option<Vec<u8>> {
        let s = Stream::new();
        unsafe {
            if self.component.getState(s.as_ptr()) != kResultOk {
                return None;
            }
        }
        if s.is_empty() {
            return None;
        }
        Some(s.bytes())
    }

    /// Restore a component state — see [`Plugin::set_state`].
    ///
    /// The controller is told as well, and that second call is not optional: the component owns
    /// the sound but the WINDOW is drawn by the controller, so skipping it gives you a plugin
    /// that sounds like the saved patch above an editor still showing the factory one.
    /// Both streams rewind before being handed over — a stream sitting at its end reads as empty
    /// and reports no error at all, which is the classic silent failure of this API.
    fn set_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        let s = Stream::from_bytes(bytes.to_vec());
        s.rewind();
        unsafe {
            if self.component.setState(s.as_ptr()) != kResultOk {
                return Err("the plugin refused its saved patch".into());
            }
            if let Some(ctrl) = &self.controller {
                s.rewind();
                ctrl.setComponentState(s.as_ptr());
            }
        }
        Ok(())
    }
}

impl Drop for Vst3Plugin {
    fn drop(&mut self) {
        // Teardown in the reverse of setup, and every step of it matters to somebody's plugin.
        unsafe {
            self.processor.setProcessing(0);
            self.component.setActive(0);
            if let Some(ctrl) = &self.controller {
                // Disconnect before terminating, or each half is left holding a pointer to
                // something that has torn itself down and may still be called during shutdown.
                if let (Some(a), Some(b)) = (
                    self.component.cast::<IConnectionPoint>(),
                    ctrl.cast::<IConnectionPoint>(),
                ) {
                    a.disconnect(b.as_ptr());
                    b.disconnect(a.as_ptr());
                }
                ctrl.terminate();
            }
            self.component.terminate();
        }
    }
}

/// A loaded plugin of either format. This is the type everything upstream holds, and the whole
/// point of it is what it hides: `engine.rs` loads by path, `vst.rs` schedules notes, and neither
/// contains the string "Vst2" anywhere. The roadmap's rule, enforced by shape: one path through
/// the UI and the engine, whatever the plugin speaks.
pub enum Module {
    Vst3(Arc<Vst3Module>),
    Vst2(vst2::Vst2Module),
}

impl Module {
    /// Open a plugin file, picking the loader from the file itself: `.dll` is VST2, everything
    /// else (a `.vst3` file or bundle folder) is VST3. The two never overlap on disk — a `.vst3`
    /// sitting in a VST2 folder is still a VST3 — so the extension is genuinely sufficient.
    pub fn open(path: &Path) -> Result<Arc<Module>, String> {
        let is_vst2 = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("dll"))
            .unwrap_or(false);
        Ok(Arc::new(if is_vst2 {
            Module::Vst2(vst2::Vst2Module::open(path)?)
        } else {
            Module::Vst3(Vst3Module::open(path)?)
        }))
    }

    /// Every class this module offers. One entry for a VST2; possibly several for a VST3.
    pub fn classes(&self) -> Vec<PluginInfo> {
        match self {
            Module::Vst3(m) => m.classes(),
            Module::Vst2(m) => m.classes(),
        }
    }

    /// Instantiate one class, set up and processing, behind the one `Plugin` type.
    pub fn instantiate(
        self: &Arc<Self>,
        info: &PluginInfo,
        sample_rate: f64,
        max_block: i32,
    ) -> Result<Plugin, String> {
        match self.as_ref() {
            Module::Vst3(m) => {
                let mut p = m.instantiate(info, sample_rate, max_block)?;
                Ok(Plugin {
                    name: p.name.clone(),
                    out_channels: p.out_channels,
                    in_channels: p.in_channels,
                    event_in_buses: p.event_in_buses,
                    parameters: p.parameters,
                    param_list: std::mem::take(&mut p.param_list),
                    tag: p.tag,
                    latency: p.latency,
                    format: PluginFormat::Vst3,
                    backend: Backend::Vst3(p),
                })
            }
            Module::Vst2(m) => {
                let mut p = m.instantiate(sample_rate, max_block)?;
                Ok(Plugin {
                    name: p.name.clone(),
                    out_channels: p.outputs.max(1),
                    in_channels: p.inputs,
                    // VST2 has no event buses to count: `effProcessEvents` is part of the ABI for
                    // every plugin, so notes always have somewhere to go.
                    event_in_buses: 1,
                    parameters: p.parameters,
                    param_list: std::mem::take(&mut p.param_list),
                    tag: p.tag,
                    latency: p.latency,
                    format: PluginFormat::Vst2,
                    backend: Backend::Vst2(p),
                })
            }
        }
    }
}

/// A live plugin instance of either format. The fields VOLT reasons about are hoisted here so
/// callers keep field access; the format-specific machinery sits behind `backend`.
pub struct Plugin {
    pub name: String,
    /// Channels on the main output bus — what the graph node is built with.
    pub out_channels: usize,
    pub in_channels: usize,
    /// Zero means it cannot be played — see `Vst3Plugin`. Always 1 for a VST2.
    pub event_in_buses: usize,
    pub parameters: usize,
    /// This instance's address in the touch log — `last_touch()` reports it back.
    pub tag: u64,
    /// The automatable parameters, id + name, whichever format is behind. For VST3 the id is the
    /// plugin's own ParamID; for VST2 it is the parameter index. Both go back into `set_param`.
    pub param_list: Vec<ParamDesc>,
    /// What the plugin says it delays by. Reported, not yet compensated — stage E in ROADMAP.
    pub latency: u32,
    pub format: PluginFormat,
    backend: Backend,
}

enum Backend {
    Vst3(Vst3Plugin),
    Vst2(vst2::Vst2Plugin),
}

// SAFETY: a plugin is created on one thread and then moved, once, to the thread that will call
// `process` on it. One move, one owner — the same contract the engine's `PluginBox` relies on, and
// the one every DAW is built around (the format specs are written for exactly this split).
//
// **The move now goes main-thread → audio-thread, and that direction is load-bearing for VST2.**
// A VST2 plugin is a single object with no component/controller split, and its editor is opened
// through the same dispatcher its audio goes through. Plugins therefore treat *the thread that
// created them* as their UI thread — FL Studio's (Sytrus, Slicex) block inside `effEditOpen`
// waiting on it. Created on the audio thread, which has no message pump, that wait never ends:
// measured 2026-08-11, `examples/editthread.rs`, a hard hang with no CPU and no crash.
unsafe impl Send for Plugin {}

impl Plugin {
    /// Render one block: same contract whichever format is behind it. `events` is the one list the
    /// engine fills; `out` is resized and filled with `out_channels` planar buffers of `frames`.
    pub fn process(
        &mut self,
        events: &HostEvents,
        out: &mut [Vec<f32>],
        frames: usize,
    ) -> Result<(), String> {
        self.process_io(events, &[], out, frames)
    }

    /// As [`process`](Plugin::process), with audio into the plugin's input bus — what makes an
    /// EFFECT an effect. Instruments keep calling `process`; the empty input means silence of the
    /// right shape, exactly as before.
    pub fn process_io(
        &mut self,
        events: &HostEvents,
        input: &[&[f32]],
        out: &mut [Vec<f32>],
        frames: usize,
    ) -> Result<(), String> {
        match &mut self.backend {
            Backend::Vst3(p) => p.process_io(events, input, out, frames),
            Backend::Vst2(p) => p.process_io(events, input, out, frames),
        }
    }

    /// One normalised (0..1) parameter change, applied at the next block. Audio-thread safe in
    /// both formats: VST3 queues it into `inputParameterChanges`; VST2 calls the effect's own
    /// `setParameter` function pointer, which is how every host automates a VST2.
    pub fn set_param(&mut self, id: u32, value: f64) {
        match &mut self.backend {
            Backend::Vst3(p) => p.set_param(id, value),
            Backend::Vst2(p) => p.set_param(id, value),
        }
    }

    /// Forget everything ringing inside the plugin — voices, delay lines, reverb tails — so the
    /// next block starts from silence. Each format's own convention: VST3 `setProcessing` off/on,
    /// VST2 mains off/on. Called from the processing context on a fresh Play and on panic.
    pub fn flush_state(&mut self) {
        match &mut self.backend {
            Backend::Vst3(p) => p.flush_state(),
            Backend::Vst2(p) => p.flush_state(),
        }
    }

    /// **The patch.** Everything the plugin has been set to — the preset, every knob, the sample
    /// it loaded — as an opaque blob only that plugin can read back.
    ///
    /// This is what makes a plugin part of a SONG rather than a thing you set up again every time.
    /// Without it VOLT reloaded plugins at factory defaults on every device reopen, so a Sytrus
    /// patch survived until the next time the audio device changed underneath it and then quietly
    /// became the default preset again (reported 2026-08-13: *"i put some preset there and then
    /// after some time it reset"*).
    ///
    /// Both formats answer the same question in their own words:
    ///   * VST3 — `IComponent::getState`, the processor half's state. (The controller's own
    ///     `getState` is a separate, optional blob for UI-only things like which tab is open; a
    ///     host that saves just the component state is a host that restores the sound, which is
    ///     the part worth being strict about.)
    ///   * VST2 — the bank chunk (`effGetChunk`), via the crate's parameters handle.
    ///
    /// `None` means the plugin has nothing to say, which is a legitimate answer: a plugin that
    /// does not implement chunks keeps its state in its parameters, and those are already saved.
    pub fn state(&self) -> Option<Vec<u8>> {
        match &self.backend {
            Backend::Vst3(p) => p.state(),
            Backend::Vst2(p) => p.state(),
        }
    }

    /// Put a patch from [`state`](Plugin::state) back into the plugin.
    ///
    /// Called right after a plugin is created and before it makes a sound, so a reload is
    /// invisible. A blob from a DIFFERENT plugin is not our problem to detect — plugins check
    /// their own magic and refuse — but it is the caller's job not to cross them: VOLT keys the
    /// saved state by the plugin's path, so a song that lost a plugin cannot feed its patch to
    /// whatever took the slot.
    pub fn set_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        match &mut self.backend {
            Backend::Vst3(p) => p.set_state(bytes),
            Backend::Vst2(p) => p.set_state(bytes),
        }
    }

    /// Take the half of this plugin that draws its window, to be moved to the UI thread.
    ///
    /// **This is the controller/component split, and it is why a plugin window is possible at
    /// all.** Everything else about a loaded plugin belongs to the audio thread and is moved
    /// there; the editor half has to stay somewhere the main thread can reach, because that is
    /// the only thread whose message loop can run a window.
    ///
    /// Returns `None` the second time, and for a plugin with no editor. The two formats differ in
    /// what is being taken:
    ///   * VST3 — a second, add-ref'd reference to the *same* controller the audio side holds.
    ///     Teardown is unchanged and still the audio side's job in the documented order; this is
    ///     an extra owner keeping the object alive, not a transfer.
    ///   * VST2 — the crate's editor object, which hangs off the shared parameters handle rather
    ///     than off the instance, and which the crate will only hand out once.
    pub fn take_editor(&mut self) -> Option<EditorHandle> {
        match &mut self.backend {
            Backend::Vst3(p) => p
                .controller
                .clone()
                .map(|c| EditorHandle::vst3(c, self.name.clone())),
            Backend::Vst2(p) => p.take_editor().map(|e| EditorHandle::vst2(e, self.name.clone())),
        }
    }
}

/// Create the plugin's controller, connect it to the component, and hand it the component's state.
///
/// All three steps matter and the third is the one that is easy to skip:
///   1. `getControllerClassId` then `createInstance` — some plugins put both halves in one class,
///      in which case the component *is* the controller and asking for a separate one fails. That
///      is not an error; it is one of the two shapes the spec allows.
///   2. Connect the two `IConnectionPoint`s to each other. A proper host puts a proxy in between so
///      it can marshal messages between threads; direct is what every simple host does and what
///      plugins expect to work.
///   3. **Copy the component's state into the controller.** `getState` writes it, the stream gets
///      rewound — forget that and the reader sees nothing, with no error — and `setComponentState`
///      reads it back. This is the step that turns "loads but silent" into "plays".
///
/// Any of it may fail on a given plugin, and none of it is fatal: a plugin with no controller still
/// makes sound. So this returns an `Option` rather than an error.
unsafe fn make_controller(
    factory: &ComPtr<IPluginFactory>,
    component: &ComPtr<IComponent>,
    host: &Host,
    name: &str,
) -> Option<ComPtr<IEditController>> {
    // Shape one: the component and controller are the same object.
    if let Some(ctrl) = component.cast::<IEditController>() {
        return Some(ctrl);
    }

    // Shape two: a separate class, named by the component.
    let mut cid: TUID = [0; 16];
    if component.getControllerClassId(&mut cid) != kResultOk {
        return None;
    }
    let mut obj: *mut c_void = std::ptr::null_mut();
    if factory.createInstance(cid.as_ptr(), IEditController::IID.as_ptr() as *const i8, &mut obj)
        != kResultOk
        || obj.is_null()
    {
        return None;
    }
    let controller = ComPtr::<IEditController>::from_raw(obj as *mut IEditController)?;

    if controller.initialize(host.as_ptr() as *mut FUnknown) != kResultOk {
        eprintln!("  {name}: controller refused to initialize — parameters will be unavailable");
        return None;
    }

    if let (Some(a), Some(b)) = (
        component.cast::<IConnectionPoint>(),
        controller.cast::<IConnectionPoint>(),
    ) {
        a.connect(b.as_ptr());
        b.connect(a.as_ptr());
    }

    let state = Stream::new();
    if component.getState(state.as_ptr()) == kResultOk {
        state.rewind();
        controller.setComponentState(state.as_ptr());
    }

    Some(controller)
}

/// Channels on the first bus in `dir`, or 0 if there is no such bus.
unsafe fn bus_channels(component: &ComPtr<IComponent>, dir: i32) -> usize {
    if component.getBusCount(MediaTypes_::kAudio as i32, dir) < 1 {
        return 0;
    }
    let mut info: BusInfo = std::mem::zeroed();
    if component.getBusInfo(MediaTypes_::kAudio as i32, dir, 0, &mut info) != kResultOk {
        return 0;
    }
    info.channelCount.max(0) as usize
}

/// The channel count of EVERY audio bus in `dir`, in bus order — because `ProcessData` is a
/// per-bus contract, not a per-channel one. See the sidechain note on `process_io`.
unsafe fn all_bus_channels(component: &ComPtr<IComponent>, dir: i32) -> Vec<usize> {
    let n = component.getBusCount(MediaTypes_::kAudio as i32, dir).max(0);
    (0..n)
        .map(|i| {
            let mut info: BusInfo = std::mem::zeroed();
            if component.getBusInfo(MediaTypes_::kAudio as i32, dir, i, &mut info) == kResultOk {
                info.channelCount.max(0) as usize
            } else {
                2
            }
        })
        .collect()
}

/// End the process now, without letting Windows unload the plugin DLLs.
///
/// **This is how a plugin host exits, and it is not a hack.** Once the plugins have been released
/// there is nothing left worth doing tidily, and what remains — Windows walking every loaded DLL's
/// `DllMain(PROCESS_DETACH)` — is code we did not write, running after we no longer care, with a
/// measured habit of falling over. On this machine: Jup-8000 V never returned from it at all (the
/// program hung after printing its results, and had to be killed), and Vital, Diva and Kontakt came
/// back with an access violation. Every one of them had already played correctly and released
/// cleanly. `std::process::exit` does not help: it runs the same teardown.
///
/// So: flush what we have written, then stop. Anything that must be durable has to be on disk
/// before this is called, which is a real constraint and the reason this is one obvious function
/// with a name that says what it does rather than something buried in a `Drop`.
pub fn hard_exit(code: u32) -> ! {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();

    // kernel32, linked into every Windows process already — declaring the two calls here is
    // smaller and clearer than taking on windows-sys for them.
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn TerminateProcess(handle: *mut c_void, exit_code: u32) -> i32;
    }
    unsafe {
        TerminateProcess(GetCurrentProcess(), code);
    }
    // TerminateProcess on the current process does not return, but the compiler does not know that.
    unreachable!("TerminateProcess returned");
}

/// Where the plugin blacklist lives: `%LOCALAPPDATA%\VOLT\vst-blacklist.txt`, one full path per
/// line, hand-editable (delete a line to give a plugin another chance after an update).
///
/// A file lands here when opening it took a scan down — crashed it, or hung it forever. Both are
/// real: the desktop app's scan journal converts a died-mid-file scan into an entry here, and
/// `Bionic Delay v1_2.64.dll` earned the first entry by hanging inside `LoadLibrary` without
/// return (measured 2026-08-10, in isolation, 20 s and counting). Format-neutral on purpose —
/// nothing says a VST3 cannot hang too.
pub fn blacklist_path() -> Option<PathBuf> {
    let dir = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    Some(dir.join("VOLT").join("vst-blacklist.txt"))
}

/// The blacklist as it stands, empty if there is none.
pub fn read_blacklist() -> Vec<String> {
    blacklist_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
        .unwrap_or_default()
}

/// Whether this file is on the blacklist. Compared as text, because that is what the file holds
/// and what a human editing it will have typed.
pub fn is_blacklisted(path: &Path) -> bool {
    let p = path.to_string_lossy();
    read_blacklist().iter().any(|b| b.as_str() == p)
}

/// Add a plugin to the blacklist, once. Returns whether it was newly added.
///
/// Called when a plugin has demonstrably taken the process down — during a scan, or while being
/// loaded into a song. The alternative to remembering is an app that dies the same way every time
/// it is started, which is what `Bionic Delay` did before any of this existed.
pub fn blacklist(path: &Path) -> bool {
    let p = path.to_string_lossy().to_string();
    let mut list = read_blacklist();
    if list.iter().any(|b| *b == p) {
        return false;
    }
    list.push(p);
    if let Some(bp) = blacklist_path() {
        if let Some(dir) = bp.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&bp, list.join("\n"));
    }
    true
}

/// Every plugin on this machine.
///
/// Three layouts live in the VST3 folder and all three are in use on Mickey's machine, so a scanner
/// that knows only the documented one finds about half of them:
///   * a plain DLL named `Thing.vst3` (FabFilter, Vital, Kontakt),
///   * a bundle folder `Thing.vst3/Contents/x86_64-win/Thing.vst3` (Amigo),
///   * a vendor folder containing either of the above (`u-he/Diva(x64).vst3`, `Valhalla DSP/...`).
pub fn scan(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(dir, 0, &mut out);
    out.sort();
    out
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    // Two levels is enough for vendor/Thing.vst3 and stops a bundle's insides being walked as if
    // they were more plugins.
    if depth > 2 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension() == Some(OsStr::new("vst3")) {
            out.push(p);
        } else if p.is_dir() {
            collect(&p, depth + 1, out);
        }
    }
}

/// The actual DLL for a path that may be a bundle folder or may be the DLL itself.
fn binary_in(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    let arch = path.join("Contents").join("x86_64-win");
    let entries = std::fs::read_dir(arch).ok()?;
    entries
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension() == Some(OsStr::new("vst3")))
}

/// A fixed-size C string field, as Rust. Plugins are not always tidy about the terminator, so the
/// length is taken from the first NUL or the whole field, whichever comes first.
fn cstr(buf: &[i8]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

/// A `String128` (NUL-terminated UTF-16) into a `String` — parameter titles come this way.
fn utf16z(buf: &[u16]) -> String {
    let units: Vec<u16> = buf.iter().take_while(|&&c| c != 0).copied().collect();
    String::from_utf16_lossy(&units).trim().to_string()
}

/// One automatable parameter: its id (a VST3 `ParamID`, or a VST2 parameter index) and the name
/// the plugin gives it. What the automation picker lists and what `Plugin::set_param` takes.
#[derive(Debug, Clone)]
pub struct ParamDesc {
    pub id: u32,
    pub name: String,
}

// ---- the touch log -----------------------------------------------------------------------------
//
// Plugins TELL their host when a knob in their editor is moved — VST3 through
// `IComponentHandler::performEdit`, VST2 through the `audioMasterAutomate` callback. Every DAW's
// "automate the knob you just touched" is built on exactly this, and VOLT simply never listened.
// One host-wide slot holding the latest touch is all the feature needs: the user turns a knob,
// then asks VOLT to automate "that one".

/// Where a touch came from: each plugin instance gets a tag at instantiate, and the desktop side
/// keeps a tag → slot/track registry. A tag rather than a name because two Sytrus instances are
/// two different targets with one name.
static NEXT_TAG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(crate) fn next_tag() -> u64 {
    NEXT_TAG.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

struct Touch {
    tag: u64,
    id: u32,
    value: f64,
    at: std::time::Instant,
}

static LAST_TOUCH: Mutex<Option<Touch>> = Mutex::new(None);

pub(crate) fn record_touch(tag: u64, id: u32, value: f64) {
    if tag == 0 {
        return; // a scan-path instance nobody can address
    }
    *LAST_TOUCH.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Touch { tag, id, value, at: std::time::Instant::now() });
}

/// The most recent plugin-knob touch: (instance tag, param id, normalised value, ms ago).
pub fn last_touch() -> Option<(u64, u32, f64, u64)> {
    LAST_TOUCH
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|t| (t.tag, t.id, t.value, t.at.elapsed().as_millis() as u64))
}

/// Re-exported so a caller can build a note list without depending on `vst3` itself.
pub use vst3::Steinberg::Vst::Event;
