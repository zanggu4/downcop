//! System tray icon and its context menu (Windows only).

use crate::config::Config;
use crate::i18n::{self, Severity};
use crate::icon::{self, IconStyle};
use crate::policy::Policy;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    ToggleWindow,
    ShowWindow,
    ToggleDownloadBlock,
    ToggleUploadBlock,
    ToggleDownloadLimit,
    ToggleUploadLimit,
    ToggleAll,
    Quit,
}

const ID_DL_BLOCK: &str = "dl-block";
const ID_UL_BLOCK: &str = "ul-block";
const ID_DL_LIMIT: &str = "dl-limit";
const ID_UL_LIMIT: &str = "ul-limit";
const ID_ALL: &str = "all";
const ID_OPEN: &str = "open";
const ID_QUIT: &str = "quit";

pub struct Tray {
    icon: TrayIcon,
    dl_block: CheckMenuItem,
    ul_block: CheckMenuItem,
    dl_limit: CheckMenuItem,
    ul_limit: CheckMenuItem,
    all: MenuItem,
    open: MenuItem,
    quit: MenuItem,
    style: IconStyle,
}

fn make_icon(style: IconStyle) -> Option<tray_icon::Icon> {
    // 32px renders crisply at 100–200% scaling; Windows downsamples as needed.
    tray_icon::Icon::from_rgba(icon::render_rgba(32, style), 32, 32).ok()
}

impl Tray {
    /// Creates the tray icon. `on_command` is invoked from the UI thread's
    /// message loop for every user action.
    pub fn new(on_command: impl Fn(TrayCommand) + Send + Sync + 'static) -> Result<Tray, String> {
        let dl_block = CheckMenuItem::with_id(ID_DL_BLOCK, "", true, false, None);
        let ul_block = CheckMenuItem::with_id(ID_UL_BLOCK, "", true, false, None);
        let dl_limit = CheckMenuItem::with_id(ID_DL_LIMIT, "", true, false, None);
        let ul_limit = CheckMenuItem::with_id(ID_UL_LIMIT, "", true, false, None);
        let all = MenuItem::with_id(ID_ALL, "", true, None);
        let open = MenuItem::with_id(ID_OPEN, "", true, None);
        let quit = MenuItem::with_id(ID_QUIT, "", true, None);

        let menu = Menu::new();
        menu.append_items(&[
            &dl_block,
            &ul_block,
            &PredefinedMenuItem::separator(),
            &dl_limit,
            &ul_limit,
            &PredefinedMenuItem::separator(),
            &all,
            &PredefinedMenuItem::separator(),
            &open,
            &quit,
        ])
        .map_err(|e| e.to_string())?;

        let style = IconStyle::Normal;
        let mut builder =
            TrayIconBuilder::new().with_menu(Box::new(menu)).with_menu_on_left_click(false).with_tooltip("downcop");
        if let Some(i) = make_icon(style) {
            builder = builder.with_icon(i);
        }
        let icon = builder.build().map_err(|e| e.to_string())?;

        let on_command = std::sync::Arc::new(on_command);
        {
            let cb = on_command.clone();
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| match e {
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } => {
                    cb(TrayCommand::ToggleWindow)
                }
                TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => cb(TrayCommand::ShowWindow),
                _ => {}
            }));
        }
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let cmd = match e.id().as_ref() {
                ID_DL_BLOCK => TrayCommand::ToggleDownloadBlock,
                ID_UL_BLOCK => TrayCommand::ToggleUploadBlock,
                ID_DL_LIMIT => TrayCommand::ToggleDownloadLimit,
                ID_UL_LIMIT => TrayCommand::ToggleUploadLimit,
                ID_ALL => TrayCommand::ToggleAll,
                ID_OPEN => TrayCommand::ShowWindow,
                ID_QUIT => TrayCommand::Quit,
                _ => return,
            };
            on_command(cmd);
        }));

        Ok(Tray { icon, dl_block, ul_block, dl_limit, ul_limit, all, open, quit, style })
    }

    /// Refreshes texts, check marks, icon color and tooltip.
    pub fn update(&mut self, cfg: &Config, active: &Policy, error: bool) {
        let t = i18n::texts(cfg.language);
        let limit_label = |base: &str, d: &crate::config::DirectionConfig| match d.limit_bytes() {
            Some(b) => format!("{base} ({})", i18n::format_rate(b as f64)),
            None => base.to_string(),
        };

        self.dl_block.set_text(t.tray_dl_block);
        self.ul_block.set_text(t.tray_ul_block);
        self.dl_limit.set_text(limit_label(t.tray_dl_limit, &cfg.download));
        self.ul_limit.set_text(limit_label(t.tray_ul_limit, &cfg.upload));
        self.all.set_text(if cfg.all_blocked() { t.unblock_all } else { t.block_all });
        self.open.set_text(t.tray_open);
        self.quit.set_text(t.tray_quit);

        self.dl_block.set_checked(cfg.download.blocked);
        self.ul_block.set_checked(cfg.upload.blocked);
        self.dl_limit.set_checked(cfg.download.limit_enabled);
        self.ul_limit.set_checked(cfg.upload.limit_enabled);
        self.dl_limit.set_enabled(!cfg.download.blocked);
        self.ul_limit.set_enabled(!cfg.upload.blocked);

        let (status, severity) = i18n::status(cfg.language, active);
        let style = match (error, severity) {
            (true, _) | (_, Severity::Error) => IconStyle::Normal,
            (_, Severity::Normal) => IconStyle::Normal,
            (_, Severity::Limited) => IconStyle::Limited,
            (_, Severity::Blocked) => IconStyle::Blocked,
        };
        if style != self.style {
            self.style = style;
            let _ = self.icon.set_icon(make_icon(style));
        }
        let tip = if error {
            format!("{} — {}", t.app_title, t.status_error)
        } else {
            format!("{} — {status}", t.app_title)
        };
        let _ = self.icon.set_tooltip(Some(tip));
    }
}
