//! The note list the host hands a plugin each block.
//!
//! VST3 turns the direction of control around compared to everything else in the API: the host does
//! not push notes into the plugin, it implements `IEventList` and the plugin *reads* from it during
//! `process`. So this is a COM object written in Rust and given to somebody else's C++ to call
//! back into, which is exactly as delicate as it sounds and the reason it lives in its own file.
//!
//! The plugin may call `getEventCount` and `getEvent` from the audio thread at any point inside
//! `process`, so nothing here allocates or locks during a call: the list is filled by the caller
//! *before* `process` and only read during it.

use std::cell::RefCell;

use vst3::Steinberg::Vst::{
    Event, Event_::EventTypes_, IEventList, IEventListTrait, NoteOnEvent, NoteOffEvent,
};
use vst3::Steinberg::{kResultFalse, kResultOk, tresult, int32};
use vst3::{Class, ComPtr, ComWrapper};

/// The COM class itself. Kept private: callers use [`HostEvents`], which owns the reference count.
pub struct EventList {
    /// `RefCell` rather than a lock because every access is on one thread — the audio thread —
    /// and a lock here would be a priority-inversion waiting to happen. The borrow rules still
    /// catch a plugin re-entering us, which is the failure worth catching.
    events: RefCell<Vec<Event>>,
}

impl Class for EventList {
    type Interfaces = (IEventList,);
}

impl IEventListTrait for EventList {
    unsafe fn getEventCount(&self) -> int32 {
        match self.events.try_borrow() {
            Ok(e) => e.len() as int32,
            // A plugin calling back into us while we are filling the list is a bug in the host,
            // not in the plugin; report an empty list rather than panicking across an FFI boundary,
            // which is undefined behaviour and would take the whole app with it.
            Err(_) => 0,
        }
    }

    unsafe fn getEvent(&self, index: int32, e: *mut Event) -> tresult {
        if e.is_null() {
            return kResultFalse;
        }
        if index < 0 {
            // NOT clamped to 0. Clamping would hand back the first note of the block to a plugin
            // that asked for something impossible, which is a wrong note played confidently — the
            // worst kind of bug to find by ear. An index out of range is an error and says so.
            return kResultFalse;
        }
        let Ok(events) = self.events.try_borrow() else { return kResultFalse };
        let Some(ev) = events.get(index as usize) else { return kResultFalse };
        *e = *ev;
        kResultOk
    }

    unsafe fn addEvent(&self, e: *mut Event) -> tresult {
        if e.is_null() {
            return kResultFalse;
        }
        let Ok(mut events) = self.events.try_borrow_mut() else { return kResultFalse };
        events.push(*e);
        kResultOk
    }
}

/// The host's note list for one block: fill it, hand it to `process`, clear it, repeat.
pub struct HostEvents {
    wrapper: ComWrapper<EventList>,
    /// Held for as long as the list exists so the pointer given to `process` stays valid. Taking a
    /// fresh `ComPtr` per block would work too and would touch an atomic refcount 344 times a
    /// second for no reason.
    ptr: ComPtr<IEventList>,
}

impl Default for HostEvents {
    fn default() -> Self {
        Self::new()
    }
}

impl HostEvents {
    pub fn new() -> HostEvents {
        let wrapper = ComWrapper::new(EventList { events: RefCell::new(Vec::with_capacity(64)) });
        let ptr = wrapper.to_com_ptr::<IEventList>().expect("EventList implements IEventList");
        HostEvents { wrapper, ptr }
    }

    /// Empty the list. Called at the top of every block — an event left behind is a note that
    /// retriggers every 128 samples, which is unmistakable and only funny once.
    pub fn clear(&self) {
        self.wrapper.events.borrow_mut().clear();
    }

    pub fn len(&self) -> usize {
        self.wrapper.events.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A note on, `offset` samples into the coming block.
    ///
    /// `pitch` is a MIDI note number — and VOLT's note numbers are not MIDI's. VOLT counts A4 = 440
    /// at note 57 (`nfreq()` in the engine); MIDI puts A4 at 69. The +12 is that difference, and it
    /// belongs here rather than at every call site.
    pub fn note_on(&self, note: f32, velocity: f32, offset: i32) {
        let mut ev: Event = unsafe { std::mem::zeroed() };
        ev.busIndex = 0;
        ev.sampleOffset = offset;
        ev.r#type = EventTypes_::kNoteOnEvent as u16;
        ev.__field0.noteOn = NoteOnEvent {
            channel: 0,
            pitch: (note.round() as i16) + 12,
            tuning: 0.0,
            velocity: velocity.clamp(0.0, 1.0),
            length: 0,
            noteId: -1,
        };
        self.wrapper.events.borrow_mut().push(ev);
    }

    pub fn note_off(&self, note: f32, offset: i32) {
        let mut ev: Event = unsafe { std::mem::zeroed() };
        ev.busIndex = 0;
        ev.sampleOffset = offset;
        ev.r#type = EventTypes_::kNoteOffEvent as u16;
        ev.__field0.noteOff = NoteOffEvent {
            channel: 0,
            pitch: (note.round() as i16) + 12,
            velocity: 0.0,
            noteId: -1,
            tuning: 0.0,
        };
        self.wrapper.events.borrow_mut().push(ev);
    }

    /// The pointer `ProcessData.inputEvents` wants.
    pub fn com_ptr(&self) -> *mut IEventList {
        self.ptr.as_ptr()
    }

    /// Read the list back out, note by note, in plain terms.
    ///
    /// This exists for the VST2 half of the host: the engine fills ONE list per block — this one —
    /// and a VST2 plugin cannot read a COM `IEventList`, it wants `VstMidiEvent`s. Rather than
    /// teach the engine two event types, the VST2 side translates at the last moment, and this is
    /// the read half of that translation. A callback rather than a returned `Vec` because it runs
    /// on the audio thread, where an allocation is a dropout.
    ///
    /// `pitch` here is already MIDI (the +12 happened in `note_on`), which is exactly what a MIDI
    /// status byte wants too — no second conversion.
    pub fn for_each_note(&self, mut f: impl FnMut(PlainNote)) {
        let Ok(events) = self.wrapper.events.try_borrow() else { return };
        for ev in events.iter() {
            unsafe {
                if ev.r#type == EventTypes_::kNoteOnEvent as u16 {
                    f(PlainNote {
                        on: true,
                        pitch: ev.__field0.noteOn.pitch,
                        velocity: ev.__field0.noteOn.velocity,
                        offset: ev.sampleOffset,
                    });
                } else if ev.r#type == EventTypes_::kNoteOffEvent as u16 {
                    f(PlainNote {
                        on: false,
                        pitch: ev.__field0.noteOff.pitch,
                        velocity: 0.0,
                        offset: ev.sampleOffset,
                    });
                }
            }
        }
    }
}

/// One note out of the list, with the COM and the unions stripped away.
#[derive(Debug, Clone, Copy)]
pub struct PlainNote {
    pub on: bool,
    /// MIDI pitch, 0–127 once clamped by the consumer.
    pub pitch: i16,
    /// 0..1, as VST3 has it. The VST2 side scales to 0–127.
    pub velocity: f32,
    /// Samples into the block, same meaning in both formats.
    pub offset: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use vst3::ComRef;

    /// Read the list back the way a plugin does — through the COM vtable, not through Rust.
    ///
    /// This is the test worth having: everything else about the host needs a plugin on disk, but
    /// the note list is code we wrote that somebody else's C++ calls into, and this exercises that
    /// path end to end with no plugin involved. If the vtable were wrong, or the union field
    /// misnamed, or the struct layout off, it shows up here rather than as a synth that silently
    /// ignores every note.
    fn read_back(ev: &HostEvents) -> Vec<Event> {
        unsafe {
            let list = ComRef::from_raw(ev.com_ptr()).expect("non-null");
            let n = list.getEventCount();
            (0..n)
                .map(|i| {
                    let mut out: Event = std::mem::zeroed();
                    assert_eq!(list.getEvent(i, &mut out), kResultOk);
                    out
                })
                .collect()
        }
    }

    #[test]
    fn notes_survive_the_round_trip_through_com() {
        let ev = HostEvents::new();
        ev.note_on(48.0, 0.8, 17);
        ev.note_off(48.0, 129);

        let got = read_back(&ev);
        assert_eq!(got.len(), 2);

        unsafe {
            assert_eq!(got[0].r#type, EventTypes_::kNoteOnEvent as u16);
            assert_eq!(got[0].sampleOffset, 17);
            // VOLT note 48 is MIDI 60 — middle C. Get this wrong and every plugin plays an octave
            // out, which sounds fine on its own and wrong against everything else.
            assert_eq!(got[0].__field0.noteOn.pitch, 60);
            assert!((got[0].__field0.noteOn.velocity - 0.8).abs() < 1e-6);

            assert_eq!(got[1].r#type, EventTypes_::kNoteOffEvent as u16);
            assert_eq!(got[1].sampleOffset, 129);
            assert_eq!(got[1].__field0.noteOff.pitch, 60);
        }
    }

    #[test]
    fn clear_empties_it() {
        let ev = HostEvents::new();
        ev.note_on(48.0, 1.0, 0);
        assert_eq!(read_back(&ev).len(), 1);
        ev.clear();
        assert!(ev.is_empty());
        assert_eq!(read_back(&ev).len(), 0);
    }

    #[test]
    fn velocity_is_clamped() {
        // A velocity outside 0..1 is out of spec and plugins react to it in their own ways, none of
        // them good. VOLT's own velocities are 0..1 already; this is about what a bad Vxx or a
        // future automation lane could send.
        let ev = HostEvents::new();
        ev.note_on(48.0, 4.5, 0);
        unsafe {
            assert_eq!(read_back(&ev)[0].__field0.noteOn.velocity, 1.0);
        }
    }

    #[test]
    fn a_plugin_asking_past_the_end_gets_an_error_not_a_crash() {
        // Plugins do ask for events that are not there. The answer has to be kResultFalse, because
        // an out-of-range read on the host side would be a crash inside somebody's mix.
        let ev = HostEvents::new();
        ev.note_on(48.0, 1.0, 0);
        unsafe {
            let list = ComRef::from_raw(ev.com_ptr()).unwrap();
            let mut out: Event = std::mem::zeroed();
            assert_eq!(list.getEvent(9, &mut out), kResultFalse);
            assert_eq!(list.getEvent(-1, &mut out), kResultFalse);
            assert_eq!(list.getEvent(0, std::ptr::null_mut()), kResultFalse);
        }
    }

    #[test]
    fn a_plugin_can_add_events_too() {
        // Rare but legal: some plugins write back into the list they were given.
        let ev = HostEvents::new();
        unsafe {
            let list = ComRef::from_raw(ev.com_ptr()).unwrap();
            let mut e: Event = std::mem::zeroed();
            e.sampleOffset = 42;
            assert_eq!(list.addEvent(&mut e), kResultOk);
            assert_eq!(list.addEvent(std::ptr::null_mut()), kResultFalse);
        }
        assert_eq!(read_back(&ev)[0].sampleOffset, 42);
    }
}
