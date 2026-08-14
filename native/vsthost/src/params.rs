//! The parameter-change list the host hands a VST3 plugin each block — automation's wire.
//!
//! Same construction and same rules as `events.rs` next door: a COM object written in Rust for
//! somebody else's C++ to read during `process`, filled by the host *before* the call, no
//! allocation and no locks during one. VST3 has no "set parameter" call that reaches the audio —
//! `IEditController::setParamNormalized` moves only the UI half; the processor half hears about a
//! parameter exclusively through `ProcessData.inputParameterChanges`. This is that object.
//!
//! One queue per parameter, one point per queue at sample offset 0: block-rate automation, the
//! same resolution the browser's mix LFOs deliver. Sample-accurate ramps are a later refinement —
//! the interface carries them (that is what the offsets are for), the host just does not emit
//! them yet.

use std::cell::RefCell;

use vst3::Steinberg::Vst::{
    IParamValueQueue, IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, ParamID,
    ParamValue,
};
use vst3::Steinberg::{int32, kInvalidArgument, kResultFalse, kResultOk, tresult};
use vst3::{Class, ComPtr, ComWrapper};

/// One parameter's points for the coming block. Kept to a single point in practice; the plugin
/// may `addPoint` more (some write back), which is accepted and ignored by the host side.
pub struct ParamQueue {
    id: RefCell<ParamID>,
    points: RefCell<Vec<(int32, ParamValue)>>,
}

impl Class for ParamQueue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for ParamQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id.try_borrow().map(|i| *i).unwrap_or(0)
    }

    unsafe fn getPointCount(&self) -> int32 {
        self.points.try_borrow().map(|p| p.len() as int32).unwrap_or(0)
    }

    unsafe fn getPoint(
        &self,
        index: int32,
        sample_offset: *mut int32,
        value: *mut ParamValue,
    ) -> tresult {
        if sample_offset.is_null() || value.is_null() || index < 0 {
            return kInvalidArgument;
        }
        let Ok(points) = self.points.try_borrow() else { return kResultFalse };
        let Some((off, v)) = points.get(index as usize) else { return kResultFalse };
        *sample_offset = *off;
        *value = *v;
        kResultOk
    }

    unsafe fn addPoint(&self, sample_offset: int32, value: ParamValue, index: *mut int32) -> tresult {
        let Ok(mut points) = self.points.try_borrow_mut() else { return kResultFalse };
        points.push((sample_offset, value));
        if !index.is_null() {
            *index = points.len() as int32 - 1;
        }
        kResultOk
    }
}

/// The `IParameterChanges` the host owns: a pool of queues, reused block to block so a sweep of
/// automation does not allocate 25 times a second.
pub struct ParamChanges {
    queues: RefCell<Vec<(ComWrapper<ParamQueue>, ComPtr<IParamValueQueue>)>>,
    /// How many of the pooled queues are live this block.
    used: RefCell<usize>,
}

impl Class for ParamChanges {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for ParamChanges {
    unsafe fn getParameterCount(&self) -> int32 {
        self.used.try_borrow().map(|u| *u as int32).unwrap_or(0)
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        if index < 0 {
            return std::ptr::null_mut();
        }
        let Ok(queues) = self.queues.try_borrow() else { return std::ptr::null_mut() };
        let used = self.used.try_borrow().map(|u| *u).unwrap_or(0);
        match queues.get(index as usize) {
            Some((_, ptr)) if (index as usize) < used => ptr.as_ptr(),
            _ => std::ptr::null_mut(),
        }
    }

    unsafe fn addParameterData(&self, id: *const ParamID, index: *mut int32) -> *mut IParamValueQueue {
        // Plugins may push OUTPUT changes at the host; accept them into the pool so the call is
        // legal, and let them vanish at the next clear. The host never reads them back.
        if id.is_null() {
            return std::ptr::null_mut();
        }
        let q = push_queue(&self.queues, &self.used, *id);
        if !index.is_null() {
            if let Ok(u) = self.used.try_borrow() {
                *index = *u as int32 - 1;
            }
        }
        q
    }
}

fn push_queue(
    queues: &RefCell<Vec<(ComWrapper<ParamQueue>, ComPtr<IParamValueQueue>)>>,
    used: &RefCell<usize>,
    id: ParamID,
) -> *mut IParamValueQueue {
    let Ok(mut queues) = queues.try_borrow_mut() else { return std::ptr::null_mut() };
    let Ok(mut used) = used.try_borrow_mut() else { return std::ptr::null_mut() };
    if *used < queues.len() {
        let (w, p) = &queues[*used];
        *w.id.borrow_mut() = id;
        w.points.borrow_mut().clear();
        *used += 1;
        return p.as_ptr();
    }
    let w = ComWrapper::new(ParamQueue {
        id: RefCell::new(id),
        points: RefCell::new(Vec::with_capacity(4)),
    });
    let p = w.to_com_ptr::<IParamValueQueue>().expect("ParamQueue implements IParamValueQueue");
    let ptr = p.as_ptr();
    queues.push((w, p));
    *used += 1;
    ptr
}

/// The host's handle: fill, hand to `process`, clear, repeat — `HostEvents`' twin for parameters.
pub struct HostParams {
    wrapper: ComWrapper<ParamChanges>,
    ptr: ComPtr<IParameterChanges>,
}

impl Default for HostParams {
    fn default() -> Self {
        Self::new()
    }
}

impl HostParams {
    pub fn new() -> HostParams {
        let wrapper = ComWrapper::new(ParamChanges {
            queues: RefCell::new(Vec::with_capacity(8)),
            used: RefCell::new(0),
        });
        let ptr = wrapper
            .to_com_ptr::<IParameterChanges>()
            .expect("ParamChanges implements IParameterChanges");
        HostParams { wrapper, ptr }
    }

    /// Empty the list — the queues stay pooled, only the count resets.
    pub fn clear(&self) {
        if let Ok(mut used) = self.wrapper.used.try_borrow_mut() {
            *used = 0;
        }
    }

    /// One change: `id` to `value` (normalised 0..1) at the top of the coming block.
    pub fn set(&self, id: u32, value: f64) {
        let q = push_queue(&self.wrapper.queues, &self.wrapper.used, id);
        if let Some(q) = unsafe { vst3::ComRef::from_raw(q) } {
            let mut idx: int32 = 0;
            unsafe { q.addPoint(0, value.clamp(0.0, 1.0), &mut idx) };
        }
    }

    pub fn is_empty(&self) -> bool {
        self.wrapper.used.try_borrow().map(|u| *u == 0).unwrap_or(true)
    }

    /// The pointer `ProcessData.inputParameterChanges` wants.
    pub fn com_ptr(&self) -> *mut IParameterChanges {
        self.ptr.as_ptr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vst3::ComRef;

    /// Read back through the COM vtable, the way a plugin does — same discipline as events.rs.
    #[test]
    fn changes_survive_the_round_trip_through_com() {
        let hp = HostParams::new();
        hp.set(42, 0.5);
        hp.set(7, 1.0);
        unsafe {
            let list = ComRef::from_raw(hp.com_ptr()).expect("non-null");
            assert_eq!(list.getParameterCount(), 2);
            let q = ComRef::from_raw(list.getParameterData(0)).expect("queue 0");
            assert_eq!(q.getParameterId(), 42);
            assert_eq!(q.getPointCount(), 1);
            let (mut off, mut v) = (-1, 0.0);
            assert_eq!(q.getPoint(0, &mut off, &mut v), kResultOk);
            assert_eq!(off, 0);
            assert!((v - 0.5).abs() < 1e-12);
            // Out of range asks answer with null / error, never a crash in somebody's mix.
            assert!(list.getParameterData(5).is_null());
            let mut o2 = 0;
            let mut v2 = 0.0;
            assert_eq!(q.getPoint(9, &mut o2, &mut v2), kResultFalse);
        }
    }

    /// Clearing resets the count but keeps the pool — and refilling reuses it with new ids.
    #[test]
    fn clear_recycles_the_pool() {
        let hp = HostParams::new();
        hp.set(1, 0.1);
        hp.set(2, 0.2);
        hp.clear();
        assert!(hp.is_empty());
        hp.set(3, 0.9);
        unsafe {
            let list = ComRef::from_raw(hp.com_ptr()).unwrap();
            assert_eq!(list.getParameterCount(), 1);
            let q = ComRef::from_raw(list.getParameterData(0)).unwrap();
            assert_eq!(q.getParameterId(), 3, "a recycled queue must carry the NEW id");
            assert_eq!(q.getPointCount(), 1, "and none of the old points");
        }
    }

    /// A value outside 0..1 is out of spec for a normalised parameter; clamp, don't trust.
    #[test]
    fn values_are_clamped() {
        let hp = HostParams::new();
        hp.set(1, 7.5);
        unsafe {
            let list = ComRef::from_raw(hp.com_ptr()).unwrap();
            let q = ComRef::from_raw(list.getParameterData(0)).unwrap();
            let (mut off, mut v) = (0, 0.0);
            q.getPoint(0, &mut off, &mut v);
            assert_eq!(v, 1.0);
        }
    }
}
