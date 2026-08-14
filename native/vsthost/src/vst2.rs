//! The VST2 half of the host.
//!
//! VST2 is the older, smaller API: one exported `VSTPluginMain`, a dispatcher full of opcodes, and
//! `processReplacing`. The `vst` crate (rustaudio/vst-rs, MIT) implements all of that for hosts —
//! `PluginLoader` finds the entry point, `PluginInstance` wraps the dispatcher — so this file is
//! the VOLT-specific parts only: the scan, the PE filter, the host callback, and the translation
//! from the engine's one event list into `VstMidiEvent`s.
//!
//! **Why VST2 at all, in 2026:** Mickey asked for it directly, and 232 of his plugins exist only in
//! this format. The Steinberg licence history is why none of this is distributed — it is a personal
//! build.
//!
//! **The 32-bit fact that will not move.** A 64-bit process cannot load a 32-bit DLL, full stop.
//! Counted on this machine 2026-08-10 by PE header: 232 of the VST2 DLLs are 64-bit and load;
//! 114 are 32-bit (Absynth 3, the early Bionic Delay, old apQualizr — the genuinely early-2000s
//! ones) and need a bridge process, which is a separate project. The scan filters by PE machine
//! type and **counts what it skipped**, because a scan that silently drops half the folder looks
//! broken rather than honest.
//!
//! **Every VST3 lesson applies again, translated:**
//! * The null host context that crashed Diva is VST2's `audioMasterCallback` — plugins call it
//!   before `VSTPluginMain` even returns. The `vst` crate always supplies one; [`VoltVst2Host`]
//!   fills in the answers plugins actually use, `get_time_info` above all, because a tempo-synced
//!   plugin that is told "no transport" goes silent in a way indistinguishable from a host bug.
//! * The DLL is never unloaded. [`Vst2Module::open`] leaks one loader (holding the library) per
//!   plugin file per session, exactly the `Module` policy next door.
//! * Teardown order matters: `suspend` (mains off) before the instance closes, and the process
//!   still ends with `hard_exit` — DLL teardown at exit runs plugin code we do not control.

// The `vst` crate marks its whole API deprecated ("this crate is unmaintained") — which is fine:
// VST2 stopped changing in 2006, the crate is the only published host implementation of it, and
// unmaintained is what an API that cannot change looks like. Decided in ROADMAP; not a warning
// worth reading 12 times per build.
#![allow(deprecated)]

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use vst::api::{TimeInfo, TimeInfoFlags};
use vst::buffer::SendEventBuffer;
use vst::event::MidiEvent;
use vst::host::{Host as Vst2HostCallbacks, HostBuffer, PluginInstance, PluginLoader};
use vst::plugin::{Category, Plugin as _};

use crate::{HostEvents, PluginFormat, PluginInfo};

/// Where VST2 plugins live on this machine. Unlike VST3 the spec fixes nothing — these are the
/// conventional folders, and the three that exist here. `Program Files (x86)` is deliberately
/// absent: it is 32-bit by definition and none of it can load.
pub const VST2_DIRS: &[&str] = &[
    r"C:\Program Files\VSTPlugins",
    r"C:\Program Files\Steinberg\VSTPlugins",
    r"C:\Program Files\Common Files\VST2",
];

/// What a VST2 scan of the folders found, with the skipped count carried alongside rather than
/// buried in a log — the UI has to be able to say "114 skipped (32-bit)" or the missing half of
/// the folder reads as a broken scanner.
pub struct Vst2Scan {
    /// 64-bit DLLs, loadable in principle. Whether each is *actually* a VST2 is only knowable by
    /// loading it — plenty of stray DLLs live in these folders — so `Vst2Module::open` stays the
    /// arbiter and callers must expect failures here.
    pub dlls: Vec<PathBuf>,
    /// 32-bit DLLs seen and set aside. Not an error; a fact about the folder.
    pub skipped_32bit: usize,
}

/// Walk the conventional folders for candidate DLLs, sorted, with the 32-bit count.
pub fn scan_vst2() -> Vst2Scan {
    let mut scan = Vst2Scan { dlls: Vec::new(), skipped_32bit: 0 };
    for dir in VST2_DIRS {
        collect_dlls(Path::new(dir), 0, &mut scan);
    }
    scan.dlls.sort();
    scan
}

fn collect_dlls(dir: &Path, depth: usize, scan: &mut Vst2Scan) {
    // Vendor subfolders are one level down (Eventide/, Waves/, u-he/); two levels covers every
    // layout seen on this machine without walking whole install trees.
    if depth > 2 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let ext = p.extension().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase());
        // `.vst3` files sit in these folders too (Jup-8000 V ships both side by side). They are
        // the VST3 scanner's business; listing them here would show every such plugin twice.
        if ext.as_deref() == Some("dll") && p.is_file() {
            match pe_machine(&p) {
                Some(IMAGE_FILE_MACHINE_AMD64) => scan.dlls.push(p),
                Some(_) => scan.skipped_32bit += 1,
                // Not a PE file at all — data masquerading as .dll. Neither loadable nor worth
                // counting as skipped; it was never a plugin.
                None => {}
            }
        } else if p.is_dir() {
            collect_dlls(&p, depth + 1, scan);
        }
    }
}

/// x64, as the PE header spells it.
pub const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;

/// The machine type from a PE header, without loading the file as code.
///
/// This is the whole 32-bit filter: DOS header at 0, `e_lfanew` at 0x3C pointing at the `PE\0\0`
/// signature, machine type two bytes after it. Reading four dozen bytes per file makes the filter
/// effectively free, which matters at 300+ DLLs per scan.
pub fn pe_machine(path: &Path) -> Option<u16> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut dos = [0u8; 0x40];
    f.read_exact(&mut dos).ok()?;
    if &dos[0..2] != b"MZ" {
        return None;
    }
    let e_lfanew = u32::from_le_bytes([dos[0x3C], dos[0x3D], dos[0x3E], dos[0x3F]]);
    f.seek(SeekFrom::Start(e_lfanew as u64)).ok()?;
    let mut pe = [0u8; 6];
    f.read_exact(&mut pe).ok()?;
    if &pe[0..4] != b"PE\0\0" {
        return None;
    }
    Some(u16::from_le_bytes([pe[4], pe[5]]))
}

/// The transport as one VST2 instance sees it. Shared between the host callback (which answers
/// `audioMasterGetTime` with it) and the plugin wrapper (which advances it every block) — the VST2
/// spelling of the `ProcessContext` that is always presented and always running, for the same
/// reason: a plugin that asks "is the host playing?" and hears "no transport" goes silent, and
/// that silence is indistinguishable from a broken host.
struct Transport {
    /// f64 bits in an atomic: the audio thread writes, the callback reads, no lock on either side.
    sample_pos: std::sync::atomic::AtomicU64,
    sample_rate: f64,
    tempo: f64,
}

/// What VOLT says when a VST2 plugin calls home.
struct VoltVst2Host {
    transport: Arc<Transport>,
    block_size: isize,
    /// The instance's address in the touch log; 0 for scan-path hosts nobody can address.
    tag: u64,
}

impl Vst2HostCallbacks for VoltVst2Host {
    fn get_info(&self) -> (isize, String, String) {
        // (version, vendor, product). Some plugins key workarounds off the product string; an
        // honest unknown name gets the default behaviour, which is what a new host wants.
        (1, "VOLT".to_owned(), "VOLT".to_owned())
    }

    /// `audioMasterAutomate` — a knob in the plugin's own editor moved. The VST2 half of the
    /// touch log; the VST3 half is `EditHandler::performEdit` in host.rs.
    fn automate(&self, index: i32, value: f32) {
        if index >= 0 {
            crate::record_touch(self.tag, index as u32, value as f64);
        }
    }

    fn get_time_info(&self, _mask: i32) -> Option<TimeInfo> {
        let pos = f64::from_bits(self.transport.sample_pos.load(std::sync::atomic::Ordering::Relaxed));
        Some(TimeInfo {
            sample_pos: pos,
            sample_rate: self.transport.sample_rate,
            ppq_pos: pos / self.transport.sample_rate * (self.transport.tempo / 60.0),
            tempo: self.transport.tempo,
            time_sig_numerator: 4,
            time_sig_denominator: 4,
            flags: (TimeInfoFlags::TRANSPORT_PLAYING
                | TimeInfoFlags::TEMPO_VALID
                | TimeInfoFlags::PPQ_POS_VALID
                | TimeInfoFlags::TIME_SIG_VALID)
                .bits(),
            ..TimeInfo::default()
        })
    }

    fn get_block_size(&self) -> isize {
        self.block_size
    }
}

/// The `vst` crate routes `VSTPluginMain`'s callback through one global (`LOAD_POINTER`) between
/// `load` and `instance`, so two threads loading VST2s at once is a data race inside the crate.
/// VOLT loads from the scan thread and from the audio thread; this lock makes those take turns.
/// Held only around load/instantiate — never around `process`.
static LOAD_LOCK: Mutex<()> = Mutex::new(());

/// A VST2 plugin file, opened once and probed for what it is.
pub struct Vst2Module {
    path: PathBuf,
    info: PluginInfo,
}

impl Vst2Module {
    /// Open a DLL, ask it what it is, and keep it mapped for the rest of the session.
    pub fn open(path: &Path) -> Result<Vst2Module, String> {
        match pe_machine(path) {
            Some(IMAGE_FILE_MACHINE_AMD64) => {}
            Some(_) => {
                // The one honest error in the file: this is the message for the person who found
                // their favourite 2003 plugin missing from the list.
                return Err(format!("{}: 32-bit — a 64-bit VOLT cannot load it (needs a bridge)",
                                   path.display()));
            }
            None => return Err(format!("{}: not a Windows DLL", path.display())),
        }

        let _serial = LOAD_LOCK.lock().map_err(|_| "VST2 load lock poisoned")?;
        let (loader, mut probe) = load_and_instance(path, 512)?;
        probe.init();
        let vinfo = probe.get_info();
        drop(probe); // effClose — legitimate while the DLL stays mapped.

        // "Is this an instrument" is even vaguer than VST3's substring: `effGetPlugCategory` says
        // Synth when the plugin bothers to answer, and the rest of the convention is that a synth
        // has no audio inputs. Both are used; the subcategory string keeps `is_instrument()`
        // working unchanged across formats.
        let is_instrument =
            matches!(vinfo.category, Category::Synth) || (vinfo.inputs == 0 && vinfo.outputs > 0);

        let name = if vinfo.name.trim().is_empty() {
            // Plugins may leave effGetProductName blank; the file name is what the folder calls it.
            path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
        } else {
            vinfo.name.trim().to_string()
        };

        let info = PluginInfo {
            path: path.to_path_buf(),
            index: 0, // one plugin per VST2 DLL (shell plugins exist; none installed here)
            name,
            category: "Audio Module Class".to_string(),
            subcategories: if is_instrument { "Instrument|Synth".into() } else { "Fx".into() },
            vendor: vinfo.vendor.trim().to_string(),
            cid: [0; 16],
            format: PluginFormat::Vst2,
        };

        // Leak the loader — and with it the `Arc<Library>` — so this DLL can never be unmapped.
        // Same policy, same reasoning, same bounded cost as `Drop for Module` next door: one
        // mapping per plugin file per session, versus a crash inside somebody's song.
        std::mem::forget(loader);

        Ok(Vst2Module { path: path.to_path_buf(), info })
    }

    pub fn classes(&self) -> Vec<PluginInfo> {
        vec![self.info.clone()]
    }

    /// A fresh instance, initialized and running, ready for `process`.
    pub fn instantiate(&self, sample_rate: f64, max_block: i32) -> Result<Vst2Plugin, String> {
        let _serial = LOAD_LOCK.lock().map_err(|_| "VST2 load lock poisoned")?;
        // Loading again is a refcount bump on an already-mapped DLL — the leak in `open` holds the
        // mapping — and gives this instance its own host, so its transport is its own.
        let transport = Arc::new(Transport {
            sample_pos: std::sync::atomic::AtomicU64::new(0f64.to_bits()),
            sample_rate,
            tempo: 120.0,
        });
        let tag = crate::next_tag();
        let host = Arc::new(Mutex::new(VoltVst2Host {
            transport: Arc::clone(&transport),
            block_size: max_block as isize,
            tag,
        }));
        let mut loader = PluginLoader::load(&self.path, host)
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        let mut instance = loader
            .instance()
            .map_err(|e| format!("{}: {e}", self.path.display()))?;

        // The fixed order, VST2 edition: open, rates, mains on. Same rule as VST3's create →
        // initialize → setup → activate — plugins are entitled to misbehave if it is shuffled.
        instance.init();
        instance.set_sample_rate(sample_rate as f32);
        instance.set_block_size(max_block as i64);
        instance.resume();

        let vinfo = instance.get_info();
        let inputs = vinfo.inputs.max(0) as usize;
        let outputs = vinfo.outputs.max(0) as usize;

        // The parameter surface, for automation. The params object is the crate's shared handle
        // (a function-pointer proxy into the effect, usable from any thread — it is what the
        // editor uses too), and the names are read once here rather than per pick. NOT filtered
        // on canBeAutomated: too many VST2s answer 0 for everything, and hiding every parameter
        // is worse than listing a few odd ones. Capped like the VST3 side.
        let params = instance.get_parameter_object();
        let n_params = vinfo.parameters.clamp(0, 4096) as usize;
        let param_list = (0..n_params)
            .map(|i| {
                let name = params.get_parameter_name(i as i32);
                let name = if name.trim().is_empty() { format!("param {i}") } else { name.trim().to_string() };
                crate::ParamDesc { id: i as u32, name }
            })
            .collect();

        Ok(Vst2Plugin {
            instance,
            params,
            param_list,
            tag,
            transport,
            host_buffer: HostBuffer::new(inputs, outputs),
            in_scratch: (0..inputs).map(|_| Vec::with_capacity(max_block as usize)).collect(),
            midi_scratch: Vec::with_capacity(64),
            send_buffer: SendEventBuffer::new(64),
            name: self.info.name.clone(),
            inputs,
            outputs,
            latency: vinfo.initial_delay.max(0) as u32,
            parameters: vinfo.parameters.max(0) as usize,
            max_block,
            _keepalive_loader: loader,
        })
    }
}

/// Load + instance under one roof, with the crate's two-step spelled once.
fn load_and_instance(
    path: &Path,
    max_block: i32,
) -> Result<(PluginLoader<VoltVst2Host>, PluginInstance), String> {
    let host = Arc::new(Mutex::new(VoltVst2Host {
        transport: Arc::new(Transport {
            sample_pos: std::sync::atomic::AtomicU64::new(0f64.to_bits()),
            sample_rate: 44100.0,
            tempo: 120.0,
        }),
        block_size: max_block as isize,
        tag: 0, // scan path — touches from here have no address and are dropped
    }));
    let mut loader = PluginLoader::load(path, host).map_err(|e| match e {
        vst::host::PluginLoadError::NotAPlugin => {
            format!("{}: not a VST2 (no VSTPluginMain)", path.display())
        }
        other => format!("{}: {other}", path.display()),
    })?;
    let instance = loader.instance().map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((loader, instance))
}

/// A live VST2 instance. The counterpart of the VST3 `Plugin`'s innards, presenting the same
/// `process(&HostEvents, out, frames)` shape so nothing upstream knows the difference.
pub struct Vst2Plugin {
    instance: PluginInstance,
    /// The crate's shared parameter handle — `setParameter`/`getParameter` function-pointer
    /// proxies into the effect. How automation reaches a VST2.
    params: Arc<dyn vst::plugin::PluginParameters>,
    /// Automatable parameters (all of them — see `instantiate`), hoisted into `Plugin`.
    pub param_list: Vec<crate::ParamDesc>,
    /// This instance's address in the touch log.
    pub tag: u64,
    transport: Arc<Transport>,
    host_buffer: HostBuffer<f32>,
    /// Zeroed input for plugins that have input buses (effects, or synths with sidechains).
    in_scratch: Vec<Vec<f32>>,
    /// Note events for this block, reused so the audio thread never allocates once warm.
    midi_scratch: Vec<MidiEvent>,
    send_buffer: SendEventBuffer,
    pub name: String,
    pub inputs: usize,
    pub outputs: usize,
    pub latency: u32,
    pub parameters: usize,
    max_block: i32,
    /// Holds an `Arc<Library>` reference for this instance's lifetime. The mapping itself is
    /// already immortal via the leak in `open`; this is belt to that braces.
    _keepalive_loader: PluginLoader<VoltVst2Host>,
}

impl Vst2Plugin {
    /// The editor object, once. The crate refuses a second one (`is_editor_active`), which suits
    /// the rule next door in `editor.rs`: one view per plugin, ever.
    pub fn take_editor(&mut self) -> Option<Box<dyn vst::editor::Editor>> {
        self.instance.get_editor()
    }

    /// One normalised parameter change — VST2's `setParameter` is a plain function pointer into
    /// the effect and takes the value immediately; there is no queue to ride.
    pub fn set_param(&mut self, id: u32, value: f64) {
        self.params.set_parameter(id as i32, value.clamp(0.0, 1.0) as f32);
    }

    /// Render one block. Same contract as the VST3 twin: `out` is resized and filled, notes come
    /// from the one event list the engine fills, nothing allocates once the scratch has grown.
    /// `input` is what the plugin's inputs hear — empty for an instrument, the strip's audio for
    /// an effect, mono repeated across a wider input exactly as the VST3 side does it.
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
        if out.len() < self.outputs {
            // The crate PANICS across FFI if handed fewer buffers than the plugin declared, so
            // this is checked here where it can be an error instead.
            return Err(format!("{}: needs {} output channels, given {}",
                               self.name, self.outputs, out.len()));
        }

        // Notes: the engine's one list, translated. 0x90 note-on / 0x80 note-off, pitch already
        // MIDI (the +12 lives in `HostEvents::note_on`), velocity scaled to 1–127 — a MIDI note-on
        // at velocity 0 *is* a note-off, so the floor of 1 is load-bearing, not cosmetic.
        self.midi_scratch.clear();
        events.for_each_note(|n| {
            let pitch = n.pitch.clamp(0, 127) as u8;
            let data = if n.on {
                [0x90, pitch, ((n.velocity * 127.0).round() as u8).clamp(1, 127)]
            } else {
                [0x80, pitch, 0]
            };
            self.midi_scratch.push(MidiEvent {
                data,
                delta_frames: n.offset,
                live: false,
                note_length: None,
                note_offset: None,
                detune: 0,
                note_off_velocity: 0,
            });
        });
        if !self.midi_scratch.is_empty() {
            self.send_buffer.store_events(self.midi_scratch.iter().copied());
            self.instance.process_events(self.send_buffer.events());
        }

        for (c, ch) in self.in_scratch.iter_mut().enumerate() {
            ch.resize(frames, 0.0);
            match input.get(c.min(input.len().wrapping_sub(1))) {
                Some(src) if !input.is_empty() => {
                    let n = src.len().min(frames);
                    ch[..n].copy_from_slice(&src[..n]);
                    ch[n..].fill(0.0);
                }
                _ => ch.fill(0.0),
            }
        }
        {
            let mut buffer = self.host_buffer.bind(&self.in_scratch, out);
            self.instance.process(&mut buffer);
        }

        // Advance the transport this instance shows the world, exactly as the VST3 side advances
        // its ProcessContext — a clock that never moves reads as a paused host.
        let t = &self.transport;
        let pos = f64::from_bits(t.sample_pos.load(std::sync::atomic::Ordering::Relaxed));
        t.sample_pos.store((pos + frames as f64).to_bits(), std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// VST2's buffer flush would be the mains toggle (`suspend`/`resume`) — but those go through
    /// the DISPATCHER, and this crate's whole thread discipline is that a VST2's dispatcher
    /// belongs to the thread that created it (FL's plugins enforce it with hangs and exit 222).
    /// `flush_state` is called from the RENDER thread, so for VST2 it deliberately does NOTHING:
    /// the caller's all-notes-off sweep ends the voices, and stale delay tails are a smaller sin
    /// than Maximus wedging the audio callback.
    pub fn flush_state(&mut self) {}

    /// The bank chunk — see [`crate::Plugin::state`].
    ///
    /// A VST2 that keeps its patch in parameters rather than a chunk answers with nothing, which
    /// is correct and not a failure: its parameters are saved separately.
    pub fn state(&self) -> Option<Vec<u8>> {
        let bytes = self.params.get_bank_data();
        if bytes.is_empty() {
            None
        } else {
            Some(bytes)
        }
    }

    /// Restore a bank chunk — see [`crate::Plugin::set_state`].
    pub fn set_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.params.load_bank_data(bytes);
        Ok(())
    }
}

impl Drop for Vst2Plugin {
    fn drop(&mut self) {
        // Mains off before the instance's own Drop sends effClose — the documented order, and the
        // one plugins were tested against by every other host they ran in.
        self.instance.suspend();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_pe(path: &Path, machine: u16) {
        let mut f = std::fs::File::create(path).unwrap();
        let mut dos = [0u8; 0x40];
        dos[0] = b'M';
        dos[1] = b'Z';
        dos[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        f.write_all(&dos).unwrap();
        f.write_all(b"PE\0\0").unwrap();
        f.write_all(&machine.to_le_bytes()).unwrap();
    }

    #[test]
    fn pe_machine_reads_the_header_not_the_extension() {
        let dir = std::env::temp_dir().join("volt-vst2-pe-test");
        std::fs::create_dir_all(&dir).unwrap();

        let x64 = dir.join("x64.dll");
        write_pe(&x64, IMAGE_FILE_MACHINE_AMD64);
        assert_eq!(pe_machine(&x64), Some(IMAGE_FILE_MACHINE_AMD64));

        let x86 = dir.join("x86.dll");
        write_pe(&x86, 0x014C);
        assert_eq!(pe_machine(&x86), Some(0x014C));

        // Data wearing a .dll extension: not a PE, not counted, not loaded.
        let junk = dir.join("junk.dll");
        std::fs::write(&junk, b"just some text").unwrap();
        assert_eq!(pe_machine(&junk), None);
    }

    #[test]
    fn the_scan_separates_x64_from_skipped() {
        let dir = std::env::temp_dir().join("volt-vst2-scan-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Vendor")).unwrap();

        write_pe(&dir.join("good.dll"), IMAGE_FILE_MACHINE_AMD64);
        write_pe(&dir.join("old.dll"), 0x014C);
        write_pe(&dir.join("Vendor").join("nested.dll"), IMAGE_FILE_MACHINE_AMD64);
        std::fs::write(dir.join("readme.txt"), b"not a dll").unwrap();
        // A VST3 sitting in a VST2 folder (Jup-8000 V does this) must not be double-listed.
        write_pe(&dir.join("Both.vst3"), IMAGE_FILE_MACHINE_AMD64);

        let mut scan = Vst2Scan { dlls: Vec::new(), skipped_32bit: 0 };
        collect_dlls(&dir, 0, &mut scan);
        scan.dlls.sort();

        assert_eq!(scan.dlls.len(), 2, "the x64 DLLs, nested included: {:?}", scan.dlls);
        assert_eq!(scan.skipped_32bit, 1, "the 32-bit one is counted, not silently dropped");
        assert!(scan.dlls.iter().all(|p| p.extension().unwrap() == "dll"));
    }
}
