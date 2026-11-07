//! downcop (Download Cop / 다운로드단속반)
//!
//! One-click, system-wide download/upload blocking and speed limiting for
//! Windows 10/11, built on WinDivert.

// No console window in release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
// Much of the engine is Windows-only; the preview build on other OSes keeps it compiled but unused.
#![cfg_attr(not(windows), allow(dead_code))]

slint::include_modules!();

mod app;
mod autostart;
mod config;
mod engine;
mod filter;
mod i18n;
mod icon;
mod packet;
mod policy;
mod shaper;
#[cfg(windows)]
mod single_instance;
mod stats;
#[cfg(windows)]
mod tray;

fn main() {
    // The release build has no console: record panics in downcop.log.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        config::log(&format!("panic: {info}"));
        default_hook(info);
    }));

    let launched_at_logon = std::env::args().any(|a| a == autostart::AUTOSTART_ARG);

    #[cfg(windows)]
    let _instance = match single_instance::acquire() {
        Some(guard) => guard,
        None => {
            single_instance::signal_existing();
            return;
        }
    };

    #[cfg(windows)]
    pin_system_fonts();

    if let Err(e) = app::run(launched_at_logon) {
        let msg = format!("downcop failed to start: {e}");
        config::log(&msg);
        fatal_dialog(&msg);
        std::process::exit(1);
    }
}

/// Points Slint's renderer straight at Malgun Gothic (Hangul + Latin) so text
/// rendering never depends on system font enumeration. Respects values that
/// are already set in the environment.
#[cfg(windows)]
fn pin_system_fonts() {
    let windir = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
    let fonts = std::path::Path::new(&windir).join("Fonts");
    let existing = |names: &[&str]| -> Vec<std::path::PathBuf> {
        names.iter().map(|n| fonts.join(n)).filter(|p| p.is_file()).collect()
    };
    // Single-threaded at this point, so mutating the environment is sound.
    if std::env::var_os("SLINT_DEFAULT_FONT").is_none()
        && let Some(primary) = existing(&["malgun.ttf"]).into_iter().next()
    {
        unsafe { std::env::set_var("SLINT_DEFAULT_FONT", primary) };
    }
    if std::env::var_os("SLINT_FONT_PATH").is_none() {
        let extra = existing(&["malgunbd.ttf", "segoeui.ttf", "segoeuib.ttf", "seguisym.ttf"]);
        if let Ok(joined) = std::env::join_paths(extra) {
            unsafe { std::env::set_var("SLINT_FONT_PATH", joined) };
        }
    }
}

#[cfg(windows)]
fn fatal_dialog(msg: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (text, caption) = (wide(msg), wide("downcop"));
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_OK | MB_ICONERROR);
    }
}

#[cfg(not(windows))]
fn fatal_dialog(msg: &str) {
    eprintln!("{msg}");
}
