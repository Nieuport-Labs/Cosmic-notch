// SPDX-License-Identifier: GPL-3.0-only

//! Recording through libpulse-simple (PipeWire's PulseAudio layer), loaded at
//! run time: the library is on every desktop and in the Flatpak runtime, so
//! neither a helper program nor development headers are needed.

use std::ffi::{CStr, c_char, c_int, c_void};

const PA_STREAM_RECORD: c_int = 2;
const PA_SAMPLE_S16LE: c_int = 3;

#[repr(C)]
struct SampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

#[repr(C)]
struct BufferAttr {
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
    fragsize: u32,
}

type New = unsafe extern "C" fn(
    server: *const c_char,
    name: *const c_char,
    dir: c_int,
    dev: *const c_char,
    stream_name: *const c_char,
    ss: *const SampleSpec,
    map: *const c_void,
    attr: *const BufferAttr,
    error: *mut c_int,
) -> *mut c_void;
type Read = unsafe extern "C" fn(s: *mut c_void, data: *mut c_void, bytes: usize, error: *mut c_int) -> c_int;
type Free = unsafe extern "C" fn(s: *mut c_void);

/// A mono 16-bit recording of the default output's monitor.
pub struct Recording {
    stream: *mut c_void,
    read: Read,
    free: Free,
}

// The stream is used from one thread at a time.
unsafe impl Send for Recording {}

impl Recording {
    /// Starts recording what the speakers play, delivered in blocks of `block` samples.
    pub fn monitor(rate: u32, block: usize) -> Result<Self, String> {
        // SAFETY: the symbols are looked up with the signatures libpulse-simple documents.
        unsafe {
            let library = libc::dlopen(c"libpulse-simple.so.0".as_ptr(), libc::RTLD_NOW);
            if library.is_null() {
                return Err("libpulse-simple is not installed".into());
            }
            let symbol = |name: &CStr| {
                let symbol = libc::dlsym(library, name.as_ptr());
                (!symbol.is_null()).then_some(symbol)
            };
            let (Some(new), Some(read), Some(free)) = (
                symbol(c"pa_simple_new"),
                symbol(c"pa_simple_read"),
                symbol(c"pa_simple_free"),
            ) else {
                return Err("libpulse-simple lacks its functions".into());
            };
            let new: New = std::mem::transmute(new);
            let spec = SampleSpec {
                format: PA_SAMPLE_S16LE,
                rate,
                channels: 1,
            };
            // Small fragments keep the bars in time with the sound.
            let attr = BufferAttr {
                maxlength: u32::MAX,
                tlength: u32::MAX,
                prebuf: u32::MAX,
                minreq: u32::MAX,
                fragsize: (block * 2) as u32,
            };
            let mut error = 0;
            let stream = new(
                std::ptr::null(),
                c"Dynamic Island".as_ptr(),
                PA_STREAM_RECORD,
                c"@DEFAULT_MONITOR@".as_ptr(),
                c"Bars".as_ptr(),
                &spec,
                std::ptr::null(),
                &attr,
                &mut error,
            );
            if stream.is_null() {
                return Err(format!("cannot record the speakers (error {error})"));
            }
            Ok(Self {
                stream,
                read: std::mem::transmute::<*mut c_void, Read>(read),
                free: std::mem::transmute::<*mut c_void, Free>(free),
            })
        }
    }

    /// Fills `samples`, waiting until they have been played.
    pub fn read(&mut self, samples: &mut [i16]) -> Result<(), String> {
        let mut error = 0;
        // SAFETY: the buffer is valid for its length in bytes.
        let result = unsafe {
            (self.read)(
                self.stream,
                samples.as_mut_ptr().cast(),
                std::mem::size_of_val(samples),
                &mut error,
            )
        };
        if result < 0 {
            return Err(format!("recording stopped (error {error})"));
        }
        Ok(())
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        // SAFETY: the stream came from pa_simple_new and is freed once.
        unsafe { (self.free)(self.stream) }
    }
}
