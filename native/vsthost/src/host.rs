//! The host object a plugin is given when it starts up.
//!
//! `IComponent::initialize` takes a "host context" — an object the plugin can query for services.
//! The spec permits passing null, and the first version of this host did. It cost two of Mickey's
//! synths: **u-he Diva and Phase Plant crash with an access violation inside `initialize`** when the
//! context is null, because the first thing they do is ask it for `IHostApplication` and use the
//! answer without checking it. That is arguably their bug, but it is not one we can fix from here,
//! and "works in every other DAW" is the only standard that matters to somebody making music.
//!
//! So VOLT presents itself properly. `IHostApplication` is the minimum: a name, and a factory for
//! the two utility objects the API expects a host to be able to make (`IMessage`, `IAttributeList`,
//! used for component↔controller chatter). The factory is not implemented yet — plugins that need
//! it are supposed to cope with being told no, and none of Mickey's have needed it so far.

use std::ffi::c_void;

use vst3::Steinberg::Vst::{
    IComponentHandler, IComponentHandlerTrait, IHostApplication, IHostApplicationTrait, ParamID,
    ParamValue, String128,
};
use vst3::Steinberg::{kResultFalse, kResultOk, int32, tresult, TUID};
use vst3::{Class, ComPtr, ComWrapper};

/// What VOLT calls itself when a plugin asks. Some plugins branch on the host name — usually to
/// work around a specific DAW's bugs — so being honest here is also the safest choice.
const HOST_NAME: &str = "VOLT";

pub struct HostApp;

impl Class for HostApp {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        if name.is_null() {
            return kResultFalse;
        }
        // String128 is UTF-16, fixed length, NUL-terminated. Written byte by byte rather than with
        // a helper because getting the terminator wrong here means a plugin reads off the end of
        // the array looking for it.
        let dst = &mut *name;
        dst.fill(0);
        for (i, u) in HOST_NAME.encode_utf16().take(dst.len() - 1).enumerate() {
            dst[i] = u as _;
        }
        kResultOk
    }

    unsafe fn createInstance(
        &self,
        _cid: *mut TUID,
        _iid: *mut TUID,
        _obj: *mut *mut c_void,
    ) -> tresult {
        // Not implemented, and saying so is the correct answer rather than a lie that returns
        // garbage. If a plugin turns up that genuinely needs IMessage/IAttributeList, this is where
        // they get built — it is a small amount of work and nothing has demanded it yet.
        kResultFalse
    }
}

/// A host context to hand to `initialize`, with its reference count held for as long as the
/// plugins that were given it are alive.
pub struct Host {
    _wrapper: ComWrapper<HostApp>,
    ptr: ComPtr<IHostApplication>,
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}

impl Host {
    pub fn new() -> Host {
        let wrapper = ComWrapper::new(HostApp);
        let ptr = wrapper.to_com_ptr::<IHostApplication>().expect("HostApp implements IHostApplication");
        Host { _wrapper: wrapper, ptr }
    }

    pub fn as_ptr(&self) -> *mut IHostApplication {
        self.ptr.as_ptr()
    }
}

/// The controller's component handler — how a plugin's editor talks BACK to the host.
///
/// `performEdit` is the call behind every knob a user turns in a plugin's own window, and it is
/// the whole reason this object exists: it feeds the touch log (`record_touch`), which is what
/// "right-click → automate the knob you just moved" reads. Everything else answers a polite yes.
///
/// The one deliberate omission: `performEdit` is NOT forwarded to the processor. Every plugin on
/// this machine already routes its own GUI edits to its own audio (that is how they all worked
/// while the handler was null), and double-applying the same value buys nothing.
pub struct EditHandler {
    /// Which plugin instance this is — see `next_tag` in lib.rs.
    tag: u64,
}

impl Class for EditHandler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for EditHandler {
    unsafe fn beginEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }

    unsafe fn performEdit(&self, id: ParamID, value_normalized: ParamValue) -> tresult {
        crate::record_touch(self.tag, id, value_normalized);
        kResultOk
    }

    unsafe fn endEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }

    unsafe fn restartComponent(&self, _flags: int32) -> tresult {
        // Preset loads and layout changes land here. The audio side reads its state from the
        // component directly, so acknowledging is both honest and sufficient.
        kResultOk
    }
}

/// Owns the handler for as long as the controller that was given it lives — the plugin keeps the
/// raw pointer from `setComponentHandler` and may call it at any time until terminate.
pub struct EditHooks {
    _wrapper: ComWrapper<EditHandler>,
    ptr: ComPtr<IComponentHandler>,
}

impl EditHooks {
    pub fn new(tag: u64) -> EditHooks {
        let wrapper = ComWrapper::new(EditHandler { tag });
        let ptr = wrapper
            .to_com_ptr::<IComponentHandler>()
            .expect("EditHandler implements IComponentHandler");
        EditHooks { _wrapper: wrapper, ptr }
    }

    pub fn as_ptr(&self) -> *mut IComponentHandler {
        self.ptr.as_ptr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vst3::ComRef;

    #[test]
    fn a_plugin_asking_our_name_gets_it_as_utf16() {
        let host = Host::new();
        unsafe {
            let app = ComRef::from_raw(host.as_ptr()).unwrap();
            let mut name: String128 = [0; 128];
            assert_eq!(app.getName(&mut name), kResultOk);

            let len = name.iter().position(|&c| c == 0).expect("must be NUL-terminated");
            let got = String::from_utf16(&name[..len].iter().map(|&c| c as u16).collect::<Vec<_>>()).unwrap();
            assert_eq!(got, "VOLT");
            // The terminator is the part a plugin trusts blindly, so it is the part worth asserting.
            assert_eq!(name[len], 0);
        }
    }

    #[test]
    fn a_null_name_pointer_is_refused_not_followed() {
        let host = Host::new();
        unsafe {
            let app = ComRef::from_raw(host.as_ptr()).unwrap();
            assert_eq!(app.getName(std::ptr::null_mut()), kResultFalse);
        }
    }
}
