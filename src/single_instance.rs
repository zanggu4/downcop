//! Single instance guard: a second launch asks the running instance to show
//! its window and exits.

use std::ffi::c_void;
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SetEvent, WaitForSingleObject,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

const MUTEX_NAME: &str = "Local\\downcop-single-instance";
const EVENT_NAME: &str = "Local\\downcop-show-window";

pub struct Guard {
    mutex: *mut c_void,
}

impl Drop for Guard {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.mutex);
        }
    }
}

/// Returns `None` when another instance is already running.
pub fn acquire() -> Option<Guard> {
    unsafe {
        let name = wide(MUTEX_NAME);
        let mutex = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if mutex.is_null() {
            return Some(Guard { mutex }); // can't tell; run anyway
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(mutex);
            return None;
        }
        Some(Guard { mutex })
    }
}

/// Asks the already-running instance to show its window.
pub fn signal_existing() {
    unsafe {
        let name = wide(EVENT_NAME);
        let ev = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
        if !ev.is_null() {
            SetEvent(ev);
            CloseHandle(ev);
        }
    }
}

/// Spawns a thread that calls `on_show` whenever a second instance starts.
pub fn listen(on_show: impl Fn() + Send + 'static) {
    let name = wide(EVENT_NAME);
    let ev = unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) };
    if ev.is_null() {
        return;
    }
    let ev = ev as usize; // HANDLE is not Send; it is valid process-wide
    let _ = std::thread::Builder::new().name("downcop-instance".into()).spawn(move || {
        loop {
            let r = unsafe { WaitForSingleObject(ev as *mut c_void, INFINITE) };
            if r != WAIT_OBJECT_0 {
                break;
            }
            on_show();
        }
    });
}
