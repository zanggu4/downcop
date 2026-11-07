//! Minimal dynamic binding to `WinDivert.dll` (WinDivert 2.2).
//!
//! The DLL is loaded explicitly from the executable's directory so no import
//! library is needed at build time and DLL search-order hijacking is avoided.

use super::EngineError;
use libloading::Library;
use std::ffi::{CString, c_char, c_void};
use std::path::PathBuf;
use std::sync::OnceLock;

type Handle = *mut c_void;
type Bool = i32;

const INVALID_HANDLE: Handle = -1isize as Handle;
const LAYER_NETWORK: u32 = 0;

pub const PARAM_QUEUE_LENGTH: u32 = 0;
pub const PARAM_QUEUE_TIME: u32 = 1;
pub const PARAM_QUEUE_SIZE: u32 = 2;

pub const SHUTDOWN_RECV: u32 = 1;

/// Maximum packets per batched receive (WINDIVERT_BATCH_MAX).
pub const BATCH_MAX: usize = 0xFF;

/// `WINDIVERT_ADDRESS` (80 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Address {
    pub timestamp: i64,
    /// Layer:8, Event:8, Sniffed:1, Outbound:1, Loopback:1, Impostor:1, ...
    pub flags: u32,
    reserved2: u32,
    data: [u8; 64],
}

const _: () = assert!(std::mem::size_of::<Address>() == 80);

impl Address {
    pub const fn zeroed() -> Self {
        Address { timestamp: 0, flags: 0, reserved2: 0, data: [0; 64] }
    }

    pub fn outbound(&self) -> bool {
        (self.flags >> 17) & 1 == 1
    }
}

type OpenFn = unsafe extern "C" fn(*const c_char, u32, i16, u64) -> Handle;
type RecvExFn =
    unsafe extern "C" fn(Handle, *mut c_void, u32, *mut u32, u64, *mut Address, *mut u32, *mut c_void) -> Bool;
type SendExFn =
    unsafe extern "C" fn(Handle, *const c_void, u32, *mut u32, u64, *const Address, u32, *mut c_void) -> Bool;
type ShutdownFn = unsafe extern "C" fn(Handle, u32) -> Bool;
type CloseFn = unsafe extern "C" fn(Handle) -> Bool;
type SetParamFn = unsafe extern "C" fn(Handle, u32, u64) -> Bool;
#[cfg(test)]
type EvalFilterFn = unsafe extern "C" fn(*const c_char, *const c_void, u32, *const Address) -> Bool;
type CompileFilterFn = unsafe extern "C" fn(*const c_char, u32, *mut c_char, u32, *mut *const c_char, *mut u32) -> Bool;

struct Api {
    _lib: Library,
    open: OpenFn,
    recv_ex: RecvExFn,
    send_ex: SendExFn,
    shutdown: ShutdownFn,
    close: CloseFn,
    set_param: SetParamFn,
    compile_filter: CompileFilterFn,
    #[cfg(test)]
    eval_filter: EvalFilterFn,
}

fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| PathBuf::from("."))
}

fn load_api() -> Result<Api, EngineError> {
    let dir = exe_dir();
    let dll = dir.join("WinDivert.dll");
    let sys = dir.join("WinDivert64.sys");
    if !dll.is_file() || !sys.is_file() {
        return Err(EngineError::MissingFiles);
    }
    unsafe {
        let lib = Library::new(&dll).map_err(|e| EngineError::Other(format!("WinDivert.dll: {e}")))?;
        macro_rules! sym {
            ($name:literal, $ty:ty) => {
                *lib.get::<$ty>($name).map_err(|e| EngineError::Other(format!("WinDivert.dll: {e}")))?
            };
        }
        let open = sym!(b"WinDivertOpen\0", OpenFn);
        let recv_ex = sym!(b"WinDivertRecvEx\0", RecvExFn);
        let send_ex = sym!(b"WinDivertSendEx\0", SendExFn);
        let shutdown = sym!(b"WinDivertShutdown\0", ShutdownFn);
        let close = sym!(b"WinDivertClose\0", CloseFn);
        let set_param = sym!(b"WinDivertSetParam\0", SetParamFn);
        let compile_filter = sym!(b"WinDivertHelperCompileFilter\0", CompileFilterFn);
        #[cfg(test)]
        let eval_filter = sym!(b"WinDivertHelperEvalFilter\0", EvalFilterFn);
        Ok(Api {
            _lib: lib,
            open,
            recv_ex,
            send_ex,
            shutdown,
            close,
            set_param,
            compile_filter,
            #[cfg(test)]
            eval_filter,
        })
    }
}

fn api() -> Result<&'static Api, EngineError> {
    static API: OnceLock<Result<Api, EngineError>> = OnceLock::new();
    API.get_or_init(load_api).as_ref().map_err(Clone::clone)
}

fn last_error() -> u32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32
}

/// An open WinDivert handle. Closing it (drop) releases all captured traffic.
pub struct Divert {
    api: &'static Api,
    raw: Handle,
}

// WinDivert handles may be used concurrently from several threads.
unsafe impl Send for Divert {}
unsafe impl Sync for Divert {}

/// Validates a filter in user mode (no driver needed).
pub fn check_filter(filter: &str) -> Result<(), EngineError> {
    let api = api()?;
    let c = CString::new(filter).map_err(|_| EngineError::Other("filter contains NUL".into()))?;
    let mut err: *const c_char = std::ptr::null();
    let mut pos = 0u32;
    let ok = unsafe { (api.compile_filter)(c.as_ptr(), LAYER_NETWORK, std::ptr::null_mut(), 0, &mut err, &mut pos) };
    if ok != 0 {
        return Ok(());
    }
    let msg = if err.is_null() {
        "invalid filter".to_string()
    } else {
        unsafe { std::ffi::CStr::from_ptr(err) }.to_string_lossy().into_owned()
    };
    Err(EngineError::Other(format!("filter error at {pos}: {msg}")))
}

impl Divert {
    pub fn open(filter: &str) -> Result<Divert, EngineError> {
        let api = api()?;
        check_filter(filter)?;
        let c = CString::new(filter).map_err(|_| EngineError::Other("filter contains NUL".into()))?;
        let raw = unsafe { (api.open)(c.as_ptr(), LAYER_NETWORK, 0, 0) };
        if raw == INVALID_HANDLE || raw.is_null() {
            return Err(EngineError::from_open_error(last_error()));
        }
        Ok(Divert { api, raw })
    }

    pub fn set_param(&self, param: u32, value: u64) -> bool {
        unsafe { (self.api.set_param)(self.raw, param, value) != 0 }
    }

    /// Receives a batch of packets. Returns `(bytes, packet_count)`.
    pub fn recv_batch(&self, buf: &mut [u8], addrs: &mut [Address]) -> Result<(usize, usize), u32> {
        let mut recv_len = 0u32;
        let mut addr_len = std::mem::size_of_val(addrs) as u32;
        let ok = unsafe {
            (self.api.recv_ex)(
                self.raw,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                &mut recv_len,
                0,
                addrs.as_mut_ptr(),
                &mut addr_len,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(last_error());
        }
        let count = addr_len as usize / std::mem::size_of::<Address>();
        Ok((recv_len as usize, count))
    }

    /// Re-injects a single packet.
    pub fn send(&self, packet: &[u8], addr: &Address) -> bool {
        unsafe {
            (self.api.send_ex)(
                self.raw,
                packet.as_ptr().cast(),
                packet.len() as u32,
                std::ptr::null_mut(),
                0,
                addr,
                std::mem::size_of::<Address>() as u32,
                std::ptr::null_mut(),
            ) != 0
        }
    }

    pub fn shutdown(&self, how: u32) {
        unsafe {
            (self.api.shutdown)(self.raw, how);
        }
    }
}

impl Drop for Divert {
    fn drop(&mut self) {
        unsafe {
            (self.api.close)(self.raw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{Action, Policy};

    /// Compiles every filter the engine can generate with the real WinDivert
    /// parser. Runs only when WinDivert.dll/.sys sit next to the test binary
    /// (CI copies them there); otherwise it is skipped.
    #[test]
    fn generated_filters_compile() {
        if matches!(api(), Err(EngineError::MissingFiles)) {
            eprintln!("WinDivert files not found next to the test binary; skipping");
            return;
        }
        let actions = [Action::Pass, Action::Block, Action::Limit(1000)];
        let mut checked = 0;
        for d in actions {
            for u in actions {
                for lan_exclude in [false, true] {
                    let policy = Policy { download: d, upload: u, lan_exclude };
                    if let Some(f) = crate::filter::build(&policy) {
                        check_filter(&f).unwrap_or_else(|e| panic!("{e:?}\n{f}"));
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 16);
        assert!(check_filter("inbound and (").is_err());
    }

    fn udp_v4(src: [u8; 4], dst: [u8; 4]) -> Vec<u8> {
        let mut p = vec![0u8; 28];
        p[0] = 0x45;
        p[2..4].copy_from_slice(&28u16.to_be_bytes());
        p[8] = 64;
        p[9] = 17;
        p[12..16].copy_from_slice(&src);
        p[16..20].copy_from_slice(&dst);
        p[20..22].copy_from_slice(&1234u16.to_be_bytes());
        p[22..24].copy_from_slice(&53u16.to_be_bytes());
        p[24..26].copy_from_slice(&8u16.to_be_bytes());
        p
    }

    fn udp_v6(src: std::net::Ipv6Addr, dst: std::net::Ipv6Addr) -> Vec<u8> {
        let mut p = vec![0u8; 48];
        p[0] = 0x60;
        p[4..6].copy_from_slice(&8u16.to_be_bytes());
        p[6] = 17;
        p[7] = 64;
        p[8..24].copy_from_slice(&src.octets());
        p[24..40].copy_from_slice(&dst.octets());
        p[44..46].copy_from_slice(&8u16.to_be_bytes());
        p
    }

    fn matches(filter: &str, packet: &[u8], outbound: bool) -> bool {
        let api = api().unwrap();
        let c = CString::new(filter).unwrap();
        let mut addr = Address::zeroed();
        addr.flags = (outbound as u32) << 17 | (((packet[0] >> 4) == 6) as u32) << 20;
        unsafe { (api.eval_filter)(c.as_ptr(), packet.as_ptr().cast(), packet.len() as u32, &addr) != 0 }
    }

    /// Checks what the generated filters actually capture.
    #[test]
    fn lan_exclusion_semantics() {
        if matches!(api(), Err(EngineError::MissingFiles)) {
            eprintln!("WinDivert files not found next to the test binary; skipping");
            return;
        }
        let me = [192, 168, 0, 10];
        let both = |lan_exclude| {
            crate::filter::build(&Policy { download: Action::Block, upload: Action::Block, lan_exclude }).unwrap()
        };
        let (lan, all) = (both(true), both(false));

        // internet traffic is always captured
        assert!(matches(&lan, &udp_v4([8, 8, 8, 8], me), false));
        assert!(matches(&lan, &udp_v4(me, [1, 1, 1, 1]), true));
        // LAN peers are skipped only with lan_exclude
        for peer in [[192, 168, 0, 1], [10, 1, 2, 3], [172, 20, 0, 1], [169, 254, 9, 9]] {
            assert!(!matches(&lan, &udp_v4(peer, me), false), "{peer:?} in");
            assert!(!matches(&lan, &udp_v4(me, peer), true), "{peer:?} out");
            assert!(matches(&all, &udp_v4(peer, me), false));
        }
        // 172.32.x is public
        assert!(matches(&lan, &udp_v4([172, 32, 0, 1], me), false));
        // multicast / broadcast
        assert!(!matches(&lan, &udp_v4(me, [224, 0, 0, 251]), true));
        assert!(!matches(&lan, &udp_v4(me, [255, 255, 255, 255]), true));

        // IPv6
        let me6: std::net::Ipv6Addr = "2001:db8::10".parse().unwrap();
        let link: std::net::Ipv6Addr = "fe80::1".parse().unwrap();
        let ula: std::net::Ipv6Addr = "fd12:3456::1".parse().unwrap();
        let public: std::net::Ipv6Addr = "2606:4700::1111".parse().unwrap();
        let mdns: std::net::Ipv6Addr = "ff02::fb".parse().unwrap();
        assert!(!matches(&lan, &udp_v6(link, me6), false));
        assert!(!matches(&lan, &udp_v6(me6, ula), true));
        assert!(!matches(&lan, &udp_v6(me6, mdns), true));
        assert!(matches(&lan, &udp_v6(public, me6), false));
        assert!(matches(&all, &udp_v6(link, me6), false));

        // direction selection
        let down_only =
            crate::filter::build(&Policy { download: Action::Block, upload: Action::Pass, lan_exclude: false })
                .unwrap();
        assert!(matches(&down_only, &udp_v4([8, 8, 8, 8], me), false));
        assert!(!matches(&down_only, &udp_v4(me, [8, 8, 8, 8]), true));
    }
}
