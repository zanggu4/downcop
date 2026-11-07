//! "Start with Windows" via a Task Scheduler logon task.
//!
//! downcop requires administrator rights (WinDivert), and a `Run` registry key
//! cannot start an elevated program without a UAC prompt. A scheduled task with
//! `RunLevel=HighestAvailable` can. The task is imported from XML so that the
//! default "stop after 3 days" and "only on AC power" conditions are disabled.

#![cfg_attr(not(windows), allow(dead_code))]

pub const TASK_NAME: &str = "downcop";
pub const AUTOSTART_ARG: &str = "--autostart";

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Task Scheduler definition for a logon task running `exe --autostart`.
pub fn task_xml(exe: &str, workdir: &str, user: &str) -> String {
    let (exe, workdir, user) = (xml_escape(exe), xml_escape(workdir), xml_escape(user));
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>downcop (Download Cop) - start at logon</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
      <Delay>PT3S</Delay>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>{AUTOSTART_ARG}</Arguments>
      <WorkingDirectory>{workdir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#
    )
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn schtasks(args: &[&str]) -> Result<(), String> {
        let out = Command::new("schtasks.exe")
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if msg.is_empty() { format!("schtasks exit {}", out.status) } else { msg })
        }
    }

    fn current_user() -> String {
        let user = std::env::var("USERNAME").unwrap_or_default();
        match std::env::var("USERDOMAIN") {
            Ok(d) if !d.is_empty() => format!("{d}\\{user}"),
            _ => user,
        }
    }

    /// Registers (or refreshes, e.g. after the exe moved) the logon task.
    pub fn enable() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let dir = exe.parent().map(|d| d.display().to_string()).unwrap_or_default();
        let xml = task_xml(&exe.display().to_string(), &dir, &current_user());

        // schtasks expects UTF-16 for an XML declared as such
        let mut bytes = vec![0xFF, 0xFE];
        for u in xml.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let path = std::env::temp_dir().join(format!("downcop-task-{}.xml", std::process::id()));
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        let p = path.display().to_string();
        let result = schtasks(&["/Create", "/TN", TASK_NAME, "/XML", &p, "/F"]);
        let _ = std::fs::remove_file(&path);
        result
    }

    pub fn disable() -> Result<(), String> {
        if !is_registered() {
            return Ok(());
        }
        schtasks(&["/Delete", "/TN", TASK_NAME, "/F"])
    }

    pub fn is_registered() -> bool {
        schtasks(&["/Query", "/TN", TASK_NAME]).is_ok()
    }
}

#[cfg(windows)]
pub use imp::is_registered;

#[cfg(not(windows))]
mod imp {
    pub fn enable() -> Result<(), String> {
        Ok(())
    }
    pub fn disable() -> Result<(), String> {
        Ok(())
    }
}

pub use imp::{disable, enable};

/// Makes the OS state match the setting.
pub fn set(enabled: bool) -> Result<(), String> {
    if enabled { enable() } else { disable() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_contains_essentials_and_escapes() {
        let x = task_xml(r"C:\Tools & Apps\downcop.exe", r"C:\Tools & Apps", r"PC\홍길동");
        assert!(x.contains(r"<Command>C:\Tools &amp; Apps\downcop.exe</Command>"));
        assert!(x.contains("<Arguments>--autostart</Arguments>"));
        assert!(x.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(x.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(x.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(x.contains(r"<UserId>PC\홍길동</UserId>"));
        assert!(!x.contains(" & "));
    }
}
