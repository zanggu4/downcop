//! Traffic control engine.
//!
//! On Windows this drives WinDivert. On other platforms it is an inert stub so
//! the UI can be developed and tested anywhere.

use crate::policy::Policy;

#[cfg(windows)]
mod windivert;
#[cfg(windows)]
mod worker;

/// Whether this build can actually control traffic.
pub const SUPPORTED: bool = cfg!(windows);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineError {
    /// `WinDivert.dll` / `WinDivert64.sys` not next to the executable.
    MissingFiles,
    /// Not running as administrator.
    AccessDenied,
    /// The driver could not be loaded (signature, Secure Boot / HVCI, ...).
    DriverLoad(u32),
    /// Base Filtering Engine service is not running.
    FilteringEngineStopped,
    /// An incompatible WinDivert driver version is already loaded.
    VersionConflict,
    /// Any other Win32 error code.
    Code(u32),
    Other(String),
}

impl EngineError {
    pub fn from_open_error(code: u32) -> Self {
        match code {
            2 | 3 => EngineError::MissingFiles,
            5 => EngineError::AccessDenied,
            577 | 1275 => EngineError::DriverLoad(code),
            1753 => EngineError::FilteringEngineStopped,
            654 => EngineError::VersionConflict,
            c => EngineError::Code(c),
        }
    }
}

pub struct Engine {
    active: Policy,
    #[cfg(windows)]
    running: Option<worker::Running>,
}

impl Engine {
    pub fn new() -> Self {
        Engine {
            active: Policy::default(),
            #[cfg(windows)]
            running: None,
        }
    }

    /// The policy that is currently enforced (may differ from the requested
    /// one after an error).
    pub fn active(&self) -> Policy {
        self.active
    }

    /// Applies `policy`. On failure all traffic is released and the error is
    /// returned; `active()` then reports pass-through.
    #[cfg(windows)]
    pub fn apply(&mut self, policy: Policy) -> Result<(), EngineError> {
        let Some(filter) = crate::filter::build(&policy) else {
            self.stop();
            self.active = policy;
            return Ok(());
        };
        if let Some(r) = &self.running
            && r.filter() == filter
        {
            r.update(&policy);
            self.active = policy;
            return Ok(());
        }
        self.stop();
        match worker::Running::start(filter, &policy) {
            Ok(r) => {
                self.running = Some(r);
                self.active = policy;
                Ok(())
            }
            Err(e) => {
                self.active = Policy { lan_exclude: policy.lan_exclude, ..Policy::default() };
                Err(e)
            }
        }
    }

    #[cfg(not(windows))]
    pub fn apply(&mut self, policy: Policy) -> Result<(), EngineError> {
        // Preview mode: remember the policy so the UI reflects it.
        self.active = policy;
        Ok(())
    }

    /// Releases all traffic.
    pub fn stop(&mut self) {
        #[cfg(windows)]
        if let Some(r) = self.running.take() {
            r.stop();
        }
        self.active = Policy { lan_exclude: self.active.lan_exclude, ..Policy::default() };
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
    }
}
