//! Korean / English UI strings.

use crate::config::Language;
use crate::engine::EngineError;
use crate::policy::{Action, Policy};

pub struct Texts {
    pub app_title: &'static str,
    pub download: &'static str,
    pub upload: &'static str,
    pub block: &'static str,
    pub limit: &'static str,
    pub block_all: &'static str,
    pub unblock_all: &'static str,
    pub settings: &'static str,
    pub lan_exclude: &'static str,
    pub autostart: &'static str,
    pub apply_on_launch: &'static str,
    pub language: &'static str,
    pub history: &'static str,
    pub close_hint: &'static str,
    pub status_normal: &'static str,
    pub status_all_blocked: &'static str,
    pub status_error: &'static str,
    pub blocked_suffix: &'static str,
    pub limited_suffix: &'static str,
    pub tray_open: &'static str,
    pub tray_quit: &'static str,
    pub tray_dl_block: &'static str,
    pub tray_ul_block: &'static str,
    pub tray_dl_limit: &'static str,
    pub tray_ul_limit: &'static str,
    pub err_engine: &'static str,
    pub err_autostart: &'static str,
    pub preview_mode: &'static str,
}

const KO: Texts = Texts {
    app_title: "다운로드단속반",
    download: "다운로드",
    upload: "업로드",
    block: "차단",
    limit: "속도 제한",
    block_all: "전체 차단",
    unblock_all: "전체 해제",
    settings: "설정",
    lan_exclude: "로컬 네트워크(LAN)는 제외",
    autostart: "윈도우 시작 시 자동 실행",
    apply_on_launch: "실행하면 마지막 상태 바로 적용",
    language: "언어",
    history: "최근 60초",
    close_hint: "창을 닫으면 트레이로 숨겨집니다",
    status_normal: "정상",
    status_all_blocked: "전체 차단 중",
    status_error: "오류",
    blocked_suffix: "차단",
    limited_suffix: "제한",
    tray_open: "창 열기",
    tray_quit: "종료",
    tray_dl_block: "다운로드 차단",
    tray_ul_block: "업로드 차단",
    tray_dl_limit: "다운로드 속도 제한",
    tray_ul_limit: "업로드 속도 제한",
    err_engine: "네트워크 엔진 오류",
    err_autostart: "자동 실행 설정 실패",
    preview_mode: "미리보기 모드: 이 OS에서는 트래픽을 제어하지 않습니다",
};

const EN: Texts = Texts {
    app_title: "Download Cop",
    download: "Download",
    upload: "Upload",
    block: "Block",
    limit: "Speed limit",
    block_all: "Block all",
    unblock_all: "Unblock all",
    settings: "Settings",
    lan_exclude: "Exclude local network (LAN)",
    autostart: "Start with Windows",
    apply_on_launch: "Apply last state on launch",
    language: "Language",
    history: "Last 60 s",
    close_hint: "Closing the window hides it to the tray",
    status_normal: "Normal",
    status_all_blocked: "All blocked",
    status_error: "Error",
    blocked_suffix: "blocked",
    limited_suffix: "limited",
    tray_open: "Open window",
    tray_quit: "Quit",
    tray_dl_block: "Block download",
    tray_ul_block: "Block upload",
    tray_dl_limit: "Limit download speed",
    tray_ul_limit: "Limit upload speed",
    err_engine: "Network engine error",
    err_autostart: "Failed to change auto start",
    preview_mode: "Preview mode: traffic is not controlled on this OS",
};

pub fn texts(lang: Language) -> &'static Texts {
    match lang {
        Language::Ko => &KO,
        Language::En => &EN,
    }
}

/// Visual severity of the current state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Normal = 0,
    Limited = 1,
    Blocked = 2,
    Error = 3,
}

/// Human readable status line for the policy that is *actually* active.
pub fn status(lang: Language, active: &Policy) -> (String, Severity) {
    let t = texts(lang);
    if active.download == Action::Block && active.upload == Action::Block {
        return (t.status_all_blocked.to_string(), Severity::Blocked);
    }
    let mut parts = Vec::new();
    let mut severity = Severity::Normal;
    for (name, action) in [(t.download, active.download), (t.upload, active.upload)] {
        match action {
            Action::Pass => {}
            Action::Block => {
                parts.push(format!("{name} {}", t.blocked_suffix));
                severity = Severity::Blocked;
            }
            Action::Limit(rate) => {
                parts.push(format!("{name} {} {}", t.limited_suffix, format_rate(rate as f64)));
                if severity == Severity::Normal {
                    severity = Severity::Limited;
                }
            }
        }
    }
    if parts.is_empty() { (t.status_normal.to_string(), Severity::Normal) } else { (parts.join(" · "), severity) }
}

/// Formats bytes/second with binary units, e.g. `1.5 MB/s`.
pub fn format_rate(bytes_per_sec: f64) -> String {
    const K: f64 = 1024.0;
    if bytes_per_sec < K {
        format!("{:.0} B/s", bytes_per_sec.max(0.0))
    } else if bytes_per_sec < K * K {
        let v = bytes_per_sec / K;
        if v < 10.0 { format!("{v:.1} KB/s") } else { format!("{v:.0} KB/s") }
    } else if bytes_per_sec < K * K * K {
        let v = bytes_per_sec / (K * K);
        if v < 10.0 { format!("{v:.2} MB/s") } else { format!("{v:.1} MB/s") }
    } else {
        format!("{:.2} GB/s", bytes_per_sec / (K * K * K))
    }
}

/// Localized description of an engine failure.
pub fn engine_error(lang: Language, e: &EngineError) -> String {
    let ko = lang == Language::Ko;
    match e {
        EngineError::MissingFiles => {
            if ko {
                "WinDivert.dll과 WinDivert64.sys를 downcop.exe와 같은 폴더에 두세요.".into()
            } else {
                "Put WinDivert.dll and WinDivert64.sys in the same folder as downcop.exe.".into()
            }
        }
        EngineError::AccessDenied => {
            if ko {
                "관리자 권한으로 실행해야 합니다.".into()
            } else {
                "downcop must be run as administrator.".into()
            }
        }
        EngineError::DriverLoad(c) => {
            if ko {
                format!(
                    "WinDivert 드라이버를 불러오지 못했습니다(코드 {c}). 백신이나 '메모리 무결성' 설정이 막고 있을 수 있습니다."
                )
            } else {
                format!(
                    "Could not load the WinDivert driver (code {c}). An antivirus or Memory Integrity may be blocking it."
                )
            }
        }
        EngineError::FilteringEngineStopped => {
            if ko {
                "Windows '기본 필터링 엔진(BFE)' 서비스가 실행 중이 아닙니다.".into()
            } else {
                "The Windows Base Filtering Engine (BFE) service is not running.".into()
            }
        }
        EngineError::VersionConflict => {
            if ko {
                "다른 버전의 WinDivert 드라이버가 이미 사용 중입니다. 해당 프로그램을 끄거나 재부팅하세요.".into()
            } else {
                "Another WinDivert driver version is in use. Close that program or reboot.".into()
            }
        }
        EngineError::Code(c) => {
            if ko {
                format!("오류 코드 {c}")
            } else {
                format!("error code {c}")
            }
        }
        EngineError::Other(s) => s.clone(),
    }
}

#[cfg(windows)]
pub fn system_language() -> Language {
    // LANG_KOREAN = 0x12; the primary language id is the low 10 bits.
    let lang = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
    if lang & 0x3ff == 0x12 { Language::Ko } else { Language::En }
}

#[cfg(not(windows))]
pub fn system_language() -> Language {
    let lang = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    if lang.starts_with("ko") { Language::Ko } else { Language::En }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates() {
        assert_eq!(format_rate(0.0), "0 B/s");
        assert_eq!(format_rate(512.0), "512 B/s");
        assert_eq!(format_rate(1536.0), "1.5 KB/s");
        assert_eq!(format_rate(500.0 * 1024.0), "500 KB/s");
        assert_eq!(format_rate(1.5 * 1024.0 * 1024.0), "1.50 MB/s");
        assert_eq!(format_rate(25.0 * 1024.0 * 1024.0), "25.0 MB/s");
    }

    #[test]
    fn statuses() {
        let p = |d, u| Policy { download: d, upload: u, lan_exclude: false };
        assert_eq!(status(Language::Ko, &p(Action::Pass, Action::Pass)), ("정상".into(), Severity::Normal));
        assert_eq!(status(Language::Ko, &p(Action::Block, Action::Block)), ("전체 차단 중".into(), Severity::Blocked));
        assert_eq!(
            status(Language::En, &p(Action::Limit(1024 * 1024), Action::Pass)),
            ("Download limited 1.00 MB/s".into(), Severity::Limited)
        );
        assert_eq!(
            status(Language::Ko, &p(Action::Limit(2048), Action::Block)),
            ("다운로드 제한 2.0 KB/s · 업로드 차단".into(), Severity::Blocked)
        );
    }
}
