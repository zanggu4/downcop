//! Glue between the UI, the tray, the configuration and the engine.
//!
//! All state lives on the UI thread in [`App`]; background threads (tray
//! events, second-instance signal) hop onto it via
//! `slint::invoke_from_event_loop` + [`with_app`].

use crate::config::{Config, DirectionConfig, Language, Unit};
use crate::engine::{self, Engine, EngineError};
use crate::i18n::{self, Severity};
use crate::icon::{self, IconStyle};
use crate::stats::{self, RateMeter};
use crate::{AppWindow, I18n, Strings, autostart};
use slint::{ComponentHandle, ModelRc, SharedPixelBuffer, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

const HISTORY_LEN: usize = 60;

#[derive(Clone, Debug)]
enum AppError {
    Engine(EngineError),
    Autostart(String),
}

pub struct App {
    cfg: Config,
    engine: Engine,
    error: Option<AppError>,
    ui: slint::Weak<AppWindow>,
    #[cfg(windows)]
    tray: Option<crate::tray::Tray>,
    window_visible: bool,
    meter: RateMeter,
    hist_dl: VecDeque<f64>,
    hist_ul: VecDeque<f64>,
}

thread_local! {
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}

/// Runs `f` with the application state. Must be called on the UI thread.
fn with_app(f: impl FnOnce(&mut App)) {
    let app = APP.with(|a| a.borrow().clone());
    if let Some(app) = app {
        f(&mut app.borrow_mut());
    }
}

fn default_font() -> &'static str {
    if cfg!(windows) { "Malgun Gothic" } else { "" }
}

fn icon_image(size: u32, style: IconStyle) -> slint::Image {
    let px = icon::render_rgba(size, style);
    slint::Image::from_rgba8(SharedPixelBuffer::clone_from_slice(&px, size, size))
}

fn format_value(v: f64) -> String {
    if v.fract() == 0.0 { format!("{v:.0}") } else { format!("{v}") }
}

fn parse_value(s: &str) -> f64 {
    s.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(0.0)
}

impl App {
    fn ui(&self) -> Option<AppWindow> {
        self.ui.upgrade()
    }

    /// Pushes translated strings into the UI.
    fn push_texts(&self) {
        let Some(ui) = self.ui() else { return };
        let t = i18n::texts(self.cfg.language);
        ui.global::<I18n>().set_s(Strings {
            app_title: t.app_title.into(),
            download: t.download.into(),
            upload: t.upload.into(),
            block: t.block.into(),
            limit: t.limit.into(),
            block_all: t.block_all.into(),
            unblock_all: t.unblock_all.into(),
            settings: t.settings.into(),
            lan_exclude: t.lan_exclude.into(),
            autostart: t.autostart.into(),
            apply_on_launch: t.apply_on_launch.into(),
            language: t.language.into(),
            history: t.history.into(),
            close_hint: t.close_hint.into(),
        });
        ui.set_notice_text(if engine::SUPPORTED { SharedString::new() } else { t.preview_mode.into() });
    }

    /// Pushes the configuration into the UI widgets.
    fn push_config(&self) {
        let Some(ui) = self.ui() else { return };
        let c = &self.cfg;
        ui.set_dl_blocked(c.download.blocked);
        ui.set_dl_limit_on(c.download.limit_enabled);
        ui.set_dl_limit_text(format_value(c.download.limit_value).into());
        ui.set_dl_unit(c.download.limit_unit.index());
        ui.set_ul_blocked(c.upload.blocked);
        ui.set_ul_limit_on(c.upload.limit_enabled);
        ui.set_ul_limit_text(format_value(c.upload.limit_value).into());
        ui.set_ul_unit(c.upload.limit_unit.index());
        ui.set_lan_exclude(c.lan_exclude);
        ui.set_autostart(c.autostart);
        ui.set_apply_on_launch(c.apply_on_launch);
        ui.set_language_index(c.language.index());
    }

    /// Reads the widget state (except language/autostart) into the config.
    fn pull_config(&mut self) {
        let Some(ui) = self.ui() else { return };
        let read = |blocked: bool, limit_on: bool, text: SharedString, unit: i32| DirectionConfig {
            blocked,
            limit_enabled: limit_on,
            limit_value: parse_value(&text),
            limit_unit: Unit::from_index(unit),
        };
        self.cfg.download = read(ui.get_dl_blocked(), ui.get_dl_limit_on(), ui.get_dl_limit_text(), ui.get_dl_unit());
        self.cfg.upload = read(ui.get_ul_blocked(), ui.get_ul_limit_on(), ui.get_ul_limit_text(), ui.get_ul_unit());
        self.cfg.lan_exclude = ui.get_lan_exclude();
        self.cfg.apply_on_launch = ui.get_apply_on_launch();
    }

    /// Applies the configuration to the engine and refreshes every view.
    fn apply(&mut self) {
        match self.engine.apply(self.cfg.policy()) {
            Ok(()) => {
                if matches!(self.error, Some(AppError::Engine(_))) {
                    self.error = None;
                }
            }
            Err(e) => {
                crate::config::log(&format!("engine error: {e:?}"));
                self.error = Some(AppError::Engine(e));
            }
        }
        self.refresh_status();
        self.cfg.save();
    }

    fn refresh_status(&mut self) {
        let active = self.engine.active();
        let lang = self.cfg.language;
        let t = i18n::texts(lang);
        let error_text = match &self.error {
            Some(AppError::Engine(e)) => format!("{}: {}", t.err_engine, i18n::engine_error(lang, e)),
            Some(AppError::Autostart(e)) => format!("{}: {e}", t.err_autostart),
            None => String::new(),
        };
        if let Some(ui) = self.ui() {
            let (status, severity) = if matches!(self.error, Some(AppError::Engine(_))) {
                (t.status_error.to_string(), Severity::Error)
            } else {
                i18n::status(lang, &active)
            };
            ui.set_status_text(status.into());
            ui.set_severity(severity as i32);
            ui.set_error_text(error_text.into());
        }
        #[cfg(windows)]
        if let Some(tray) = &mut self.tray {
            tray.update(&self.cfg, &active, matches!(self.error, Some(AppError::Engine(_))));
        }
    }

    fn set_autostart(&mut self, enabled: bool) {
        self.cfg.autostart = enabled;
        match autostart::set(enabled) {
            Ok(()) => {
                if matches!(self.error, Some(AppError::Autostart(_))) {
                    self.error = None;
                }
            }
            Err(e) => {
                crate::config::log(&format!("autostart error: {e}"));
                self.cfg.autostart = autostart_fallback(enabled);
                self.error = Some(AppError::Autostart(e));
                if let Some(ui) = self.ui() {
                    ui.set_autostart(self.cfg.autostart);
                }
            }
        }
        self.cfg.save();
        self.refresh_status();
    }

    fn set_language(&mut self, lang: Language) {
        self.cfg.language = lang;
        self.push_texts();
        self.refresh_status();
        self.cfg.save();
    }

    /// Periodic throughput sample for the speed labels and the graph.
    fn tick(&mut self) {
        let Some(c) = stats::read_counters() else { return };
        let (dl, ul) = self.meter.sample(Instant::now(), c);
        for (h, v) in [(&mut self.hist_dl, dl), (&mut self.hist_ul, ul)] {
            if h.len() == HISTORY_LEN {
                h.pop_front();
            }
            h.push_back(v);
        }
        if !self.window_visible {
            return; // keep history, skip UI work while hidden
        }
        self.push_speeds(dl, ul);
    }

    fn push_speeds(&self, dl: f64, ul: f64) {
        let Some(ui) = self.ui() else { return };
        ui.set_dl_speed(i18n::format_rate(dl).into());
        ui.set_ul_speed(i18n::format_rate(ul).into());
        let peak = self.hist_dl.iter().chain(self.hist_ul.iter()).copied().fold(1024.0, f64::max);
        let series = |h: &VecDeque<f64>| -> ModelRc<f32> {
            let pad = HISTORY_LEN - h.len();
            let v: Vec<f32> = std::iter::repeat_n(0.0, pad).chain(h.iter().map(|v| (v / peak) as f32)).collect();
            ModelRc::new(VecModel::from(v))
        };
        ui.set_dl_history(series(&self.hist_dl));
        ui.set_ul_history(series(&self.hist_ul));
        ui.set_graph_peak(i18n::format_rate(peak).into());
    }

    fn show_window(&mut self) {
        if let Some(ui) = self.ui() {
            let _ = ui.show();
            ui.window().set_minimized(false);
            self.window_visible = true;
            let (dl, ul) = (self.hist_dl.back().copied().unwrap_or(0.0), self.hist_ul.back().copied().unwrap_or(0.0));
            self.push_speeds(dl, ul);
        }
    }

    #[cfg(windows)]
    fn hide_window(&mut self) {
        if let Some(ui) = self.ui() {
            let _ = ui.hide();
        }
        self.window_visible = false;
    }

    #[cfg(windows)]
    fn on_tray(&mut self, cmd: crate::tray::TrayCommand) {
        use crate::tray::TrayCommand as C;
        match cmd {
            C::ToggleWindow => {
                if self.window_visible {
                    self.hide_window()
                } else {
                    self.show_window()
                }
                return;
            }
            C::ShowWindow => {
                self.show_window();
                return;
            }
            C::Quit => {
                let _ = slint::quit_event_loop();
                return;
            }
            C::ToggleDownloadBlock => self.cfg.download.blocked ^= true,
            C::ToggleUploadBlock => self.cfg.upload.blocked ^= true,
            C::ToggleDownloadLimit => self.cfg.download.limit_enabled ^= true,
            C::ToggleUploadLimit => self.cfg.upload.limit_enabled ^= true,
            C::ToggleAll => self.cfg.toggle_all_blocked(),
        }
        self.push_config();
        self.apply();
    }
}

/// When changing the autostart task fails, report what the OS really has.
fn autostart_fallback(requested: bool) -> bool {
    #[cfg(windows)]
    {
        let _ = requested;
        autostart::is_registered()
    }
    #[cfg(not(windows))]
    {
        !requested
    }
}

pub fn run(launched_at_logon: bool) -> Result<(), slint::PlatformError> {
    let mut cfg = Config::load();
    if !cfg.apply_on_launch {
        cfg.clear_active_state();
    }

    let ui = AppWindow::new()?;
    ui.set_font_family(default_font().into());
    ui.set_app_icon(icon_image(64, IconStyle::App));
    ui.set_has_tray(cfg!(windows));

    let app = Rc::new(RefCell::new(App {
        cfg,
        engine: Engine::new(),
        error: None,
        ui: ui.as_weak(),
        #[cfg(windows)]
        tray: None,
        window_visible: false,
        meter: RateMeter::default(),
        hist_dl: VecDeque::with_capacity(HISTORY_LEN),
        hist_ul: VecDeque::with_capacity(HISTORY_LEN),
    }));
    APP.with(|a| *a.borrow_mut() = Some(app.clone()));

    // Tray (Windows). Without it the window must stay reachable.
    #[cfg(windows)]
    let has_tray = {
        let tray = crate::tray::Tray::new(|cmd| {
            let _ = slint::invoke_from_event_loop(move || with_app(|a| a.on_tray(cmd)));
        });
        match tray {
            Ok(t) => {
                app.borrow_mut().tray = Some(t);
                true
            }
            Err(e) => {
                crate::config::log(&format!("tray error: {e}"));
                ui.set_has_tray(false);
                false
            }
        }
    };
    #[cfg(not(windows))]
    let has_tray = false;

    #[cfg(windows)]
    crate::single_instance::listen(|| {
        let _ = slint::invoke_from_event_loop(|| with_app(|a| a.show_window()));
    });

    {
        let mut a = app.borrow_mut();
        // Re-register so the task follows the exe if it was moved.
        if a.cfg.autostart
            && let Err(e) = autostart::enable()
        {
            crate::config::log(&format!("autostart refresh failed: {e}"));
            a.error = Some(AppError::Autostart(e));
        }
        a.push_texts();
        a.push_config();
        a.apply();
    }

    ui.on_state_changed(|| {
        with_app(|a| {
            a.pull_config();
            a.apply();
        })
    });
    ui.on_toggle_all(|| {
        with_app(|a| {
            a.pull_config();
            a.cfg.toggle_all_blocked();
            a.push_config();
            a.apply();
        })
    });
    {
        let weak = ui.as_weak();
        ui.on_autostart_toggled(move || {
            let Some(ui) = weak.upgrade() else { return };
            let enabled = ui.get_autostart();
            with_app(|a| a.set_autostart(enabled));
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_language_selected(move || {
            let Some(ui) = weak.upgrade() else { return };
            let lang = Language::from_index(ui.get_language_index());
            with_app(|a| a.set_language(lang));
        });
    }

    ui.window().on_close_requested(move || {
        if has_tray {
            with_app(|a| a.window_visible = false);
            slint::CloseRequestResponse::HideWindow
        } else {
            let _ = slint::quit_event_loop();
            slint::CloseRequestResponse::HideWindow
        }
    });

    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_secs(1), || with_app(|a| a.tick()));
    with_app(|a| a.tick()); // prime the rate meter

    if !(launched_at_logon && has_tray) {
        with_app(|a| a.show_window());
    }

    let result = slint::run_event_loop_until_quit();

    // Release all traffic before exiting.
    timer.stop();
    APP.with(|a| {
        if let Some(app) = a.borrow_mut().take() {
            let mut app = app.borrow_mut();
            app.engine.stop();
            app.cfg.save();
            #[cfg(windows)]
            {
                app.tray = None;
            }
        }
    });
    result
}
