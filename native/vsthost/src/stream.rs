//! A chunk of bytes a plugin can read and write — `IBStream`.
//!
//! VST3 never hands you a plugin's settings as data. It hands you a stream and lets the plugin
//! write into it, and gives the bytes back the same way. So this one small object is what both of
//! these need:
//!
//!   * **Making a plugin sound at all.** After the component and controller are created they are
//!     two halves that have not spoken: the component holds the state, the controller does not know
//!     it. The host is required to move it across — `component.getState(s)`, rewind, then
//!     `controller.setComponentState(s)`. Plugins that stay silent with no preset are usually
//!     waiting for exactly that.
//!   * **Saving a plugin in a `.volt` file.** Same call, except the bytes get base64'd into the
//!     song instead of thrown away. That is step 3 of the plugin plan, and it will use this
//!     unchanged.
//!
//! It is a `Vec<u8>` with a cursor. The care goes into the edges, because a plugin is entitled to
//! seek anywhere, read past the end, and hand us null pointers, and this is somebody else's C++
//! calling into our Rust — a panic here is undefined behaviour, not a stack trace.

use std::cell::RefCell;
use std::ffi::c_void;

use vst3::Steinberg::{
    kInvalidArgument, kResultOk, tresult, IBStream, IBStreamTrait, IBStream_::IStreamSeekMode_,
    int32, int64,
};
use vst3::{Class, ComPtr, ComWrapper};

struct StreamData {
    bytes: Vec<u8>,
    pos: usize,
}

pub struct MemStream {
    data: RefCell<StreamData>,
}

impl Class for MemStream {
    type Interfaces = (IBStream,);
}

impl IBStreamTrait for MemStream {
    unsafe fn read(&self, buffer: *mut c_void, num_bytes: int32, num_read: *mut int32) -> tresult {
        if buffer.is_null() || num_bytes < 0 {
            return kInvalidArgument;
        }
        let Ok(mut d) = self.data.try_borrow_mut() else { return kInvalidArgument };

        // Reading past the end is normal — it is how a plugin discovers the end — and must return
        // a short count, not an error and not a wild copy.
        let available = d.bytes.len().saturating_sub(d.pos);
        let n = available.min(num_bytes as usize);
        if n > 0 {
            std::ptr::copy_nonoverlapping(d.bytes[d.pos..].as_ptr(), buffer as *mut u8, n);
            d.pos += n;
        }
        if !num_read.is_null() {
            *num_read = n as int32;
        }
        kResultOk
    }

    unsafe fn write(&self, buffer: *mut c_void, num_bytes: int32, num_written: *mut int32) -> tresult {
        if buffer.is_null() || num_bytes < 0 {
            return kInvalidArgument;
        }
        let Ok(mut d) = self.data.try_borrow_mut() else { return kInvalidArgument };

        let n = num_bytes as usize;
        // A plugin may seek past the end and then write, which means growing with zeros first —
        // sparse writes are legal and a plugin that does one would otherwise get its data silently
        // shifted to the wrong offset.
        let end = d.pos + n;
        if end > d.bytes.len() {
            d.bytes.resize(end, 0);
        }
        let pos = d.pos;
        std::ptr::copy_nonoverlapping(buffer as *const u8, d.bytes[pos..].as_mut_ptr(), n);
        d.pos = end;
        if !num_written.is_null() {
            *num_written = n as int32;
        }
        kResultOk
    }

    unsafe fn seek(&self, pos: int64, mode: int32, result: *mut int64) -> tresult {
        let Ok(mut d) = self.data.try_borrow_mut() else { return kInvalidArgument };
        let len = d.bytes.len() as i64;
        let base = match mode {
            x if x == IStreamSeekMode_::kIBSeekSet as int32 => 0,
            x if x == IStreamSeekMode_::kIBSeekCur as int32 => d.pos as i64,
            x if x == IStreamSeekMode_::kIBSeekEnd as int32 => len,
            _ => return kInvalidArgument,
        };
        // Clamped at zero rather than wrapping: a negative cursor would turn the next read into a
        // subtraction overflow, and an overflow here is a crash inside a plugin's load routine.
        let target = (base + pos).max(0);
        d.pos = target as usize;
        if !result.is_null() {
            *result = target;
        }
        kResultOk
    }

    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        if pos.is_null() {
            return kInvalidArgument;
        }
        let Ok(d) = self.data.try_borrow() else { return kInvalidArgument };
        *pos = d.pos as int64;
        kResultOk
    }
}

/// An `IBStream` to hand to a plugin, with the bytes still reachable from Rust afterwards.
pub struct Stream {
    wrapper: ComWrapper<MemStream>,
    ptr: ComPtr<IBStream>,
}

impl Default for Stream {
    fn default() -> Self {
        Self::new()
    }
}

impl Stream {
    pub fn new() -> Stream {
        let wrapper = ComWrapper::new(MemStream { data: RefCell::new(StreamData { bytes: Vec::new(), pos: 0 }) });
        let ptr = wrapper.to_com_ptr::<IBStream>().expect("MemStream implements IBStream");
        Stream { wrapper, ptr }
    }

    /// A stream holding `bytes`, ready to be read from the start — for loading a saved plugin state.
    pub fn from_bytes(bytes: Vec<u8>) -> Stream {
        let s = Stream::new();
        s.wrapper.data.borrow_mut().bytes = bytes;
        s
    }

    /// Back to the start. **The step that is easy to forget and silently ruins everything**: after
    /// a plugin has written its state the cursor sits at the end, and handing that straight to
    /// `setComponentState` gives the reader an empty stream and no error.
    pub fn rewind(&self) {
        self.wrapper.data.borrow_mut().pos = 0;
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.wrapper.data.borrow().bytes.clone()
    }

    pub fn len(&self) -> usize {
        self.wrapper.data.borrow().bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_ptr(&self) -> *mut IBStream {
        self.ptr.as_ptr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vst3::ComRef;

    /// Exercise the stream the way a plugin does — through the vtable.
    #[test]
    fn a_plugin_can_write_rewind_and_read_it_back() {
        let s = Stream::new();
        unsafe {
            let st = ComRef::from_raw(s.as_ptr()).unwrap();
            let mut out = 0i32;
            let mut data = *b"PRESET\x01\x02";
            assert_eq!(st.write(data.as_mut_ptr() as *mut c_void, data.len() as i32, &mut out), kResultOk);
            assert_eq!(out, data.len() as i32);

            let mut pos = 0i64;
            st.tell(&mut pos);
            assert_eq!(pos, data.len() as i64);

            s.rewind();
            let mut buf = [0u8; 16];
            let mut got = 0i32;
            assert_eq!(st.read(buf.as_mut_ptr() as *mut c_void, 16, &mut got), kResultOk);
            // Short read at the end, not an error: this is how a plugin finds the end of the data.
            assert_eq!(got, data.len() as i32);
            assert_eq!(&buf[..data.len()], &data[..]);
        }
        assert_eq!(s.bytes(), b"PRESET\x01\x02");
    }

    #[test]
    fn seeking_from_each_origin_lands_in_the_right_place() {
        let s = Stream::from_bytes(b"0123456789".to_vec());
        unsafe {
            let st = ComRef::from_raw(s.as_ptr()).unwrap();
            let mut r = 0i64;
            st.seek(3, IStreamSeekMode_::kIBSeekSet as i32, &mut r);
            assert_eq!(r, 3);
            st.seek(2, IStreamSeekMode_::kIBSeekCur as i32, &mut r);
            assert_eq!(r, 5);
            st.seek(-2, IStreamSeekMode_::kIBSeekEnd as i32, &mut r);
            assert_eq!(r, 8);

            let mut buf = [0u8; 4];
            let mut got = 0i32;
            st.read(buf.as_mut_ptr() as *mut c_void, 4, &mut got);
            assert_eq!(&buf[..got as usize], b"89");
        }
    }

    #[test]
    fn a_negative_seek_is_clamped_not_wrapped() {
        // The failure this prevents is not a wrong answer, it is a subtraction overflow on the
        // next read — inside somebody's plugin load, with no clue where it came from.
        let s = Stream::from_bytes(b"abc".to_vec());
        unsafe {
            let st = ComRef::from_raw(s.as_ptr()).unwrap();
            let mut r = 0i64;
            assert_eq!(st.seek(-99, IStreamSeekMode_::kIBSeekSet as i32, &mut r), kResultOk);
            assert_eq!(r, 0);
            let mut buf = [0u8; 3];
            let mut got = 0i32;
            st.read(buf.as_mut_ptr() as *mut c_void, 3, &mut got);
            assert_eq!(got, 3);
        }
    }

    #[test]
    fn nulls_and_nonsense_are_refused() {
        let s = Stream::new();
        unsafe {
            let st = ComRef::from_raw(s.as_ptr()).unwrap();
            let mut n = 0i32;
            assert_eq!(st.read(std::ptr::null_mut(), 4, &mut n), kInvalidArgument);
            assert_eq!(st.write(std::ptr::null_mut(), 4, &mut n), kInvalidArgument);
            assert_eq!(st.tell(std::ptr::null_mut()), kInvalidArgument);
            let mut r = 0i64;
            assert_eq!(st.seek(0, 99, &mut r), kInvalidArgument);
        }
    }

    #[test]
    fn a_plugin_that_ignores_the_counts_is_still_served() {
        // numBytesRead / numBytesWritten are allowed to be null. Plugins do pass null.
        let s = Stream::new();
        unsafe {
            let st = ComRef::from_raw(s.as_ptr()).unwrap();
            let mut data = *b"xy";
            assert_eq!(st.write(data.as_mut_ptr() as *mut c_void, 2, std::ptr::null_mut()), kResultOk);
            s.rewind();
            let mut buf = [0u8; 2];
            assert_eq!(st.read(buf.as_mut_ptr() as *mut c_void, 2, std::ptr::null_mut()), kResultOk);
            assert_eq!(&buf, b"xy");
        }
    }
}
