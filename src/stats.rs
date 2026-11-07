//! System-wide throughput measurement, independent of the engine.

use std::time::Instant;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub rx: u64,
    pub tx: u64,
}

/// Sum of octet counters over physical, connected interfaces.
#[cfg(windows)]
pub fn read_counters() -> Option<Counters> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetIfTable2, IF_TYPE_SOFTWARE_LOOPBACK, MIB_IF_TABLE2,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;

    const HARDWARE_INTERFACE: u8 = 1 << 0;
    const FILTER_INTERFACE: u8 = 1 << 1;

    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table) != 0 || table.is_null() {
            return None;
        }
        let n = (*table).NumEntries as usize;
        let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), n);
        let mut c = Counters::default();
        for row in rows {
            let flags = row.InterfaceAndOperStatusFlags._bitfield;
            if row.Type == IF_TYPE_SOFTWARE_LOOPBACK
                || row.OperStatus != IfOperStatusUp
                || flags & HARDWARE_INTERFACE == 0
                || flags & FILTER_INTERFACE != 0
            {
                continue;
            }
            c.rx = c.rx.wrapping_add(row.InOctets);
            c.tx = c.tx.wrapping_add(row.OutOctets);
        }
        FreeMibTable(table.cast());
        Some(c)
    }
}

/// Linux fallback (preview mode): `/proc/net/dev` without loopback.
#[cfg(not(windows))]
pub fn read_counters() -> Option<Counters> {
    let text = std::fs::read_to_string("/proc/net/dev").ok()?;
    Some(parse_proc_net_dev(&text))
}

#[cfg_attr(windows, allow(dead_code))]
fn parse_proc_net_dev(text: &str) -> Counters {
    let mut c = Counters::default();
    for line in text.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else { continue };
        if name.trim() == "lo" {
            continue;
        }
        let f: Vec<u64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
        if f.len() >= 9 {
            c.rx += f[0];
            c.tx += f[8];
        }
    }
    c
}

/// Turns successive counter samples into bytes/second.
#[derive(Default)]
pub struct RateMeter {
    last: Option<(Instant, Counters)>,
}

impl RateMeter {
    /// Returns `(download, upload)` in bytes/second since the previous sample.
    pub fn sample(&mut self, now: Instant, c: Counters) -> (f64, f64) {
        let rates = match self.last {
            Some((t, prev)) => {
                let dt = now.duration_since(t).as_secs_f64();
                if dt <= 0.0 {
                    (0.0, 0.0)
                } else {
                    // saturating: interfaces may disappear and counters drop
                    (c.rx.saturating_sub(prev.rx) as f64 / dt, c.tx.saturating_sub(prev.tx) as f64 / dt)
                }
            }
            None => (0.0, 0.0),
        };
        self.last = Some((now, c));
        rates
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn proc_net_dev() {
        let text = "Inter-|   Receive                                                |  Transmit\n \
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    \
lo: 1000 10 0 0 0 0 0 0 1000 10 0 0 0 0 0 0\n  \
eth0: 5000 50 0 0 0 0 0 0 7000 70 0 0 0 0 0 0\n  \
wlan0: 100 1 0 0 0 0 0 0 200 2 0 0 0 0 0 0\n";
        assert_eq!(parse_proc_net_dev(text), Counters { rx: 5100, tx: 7200 });
    }

    #[test]
    fn meter() {
        let mut m = RateMeter::default();
        let t0 = Instant::now();
        assert_eq!(m.sample(t0, Counters { rx: 100, tx: 100 }), (0.0, 0.0));
        let (d, u) = m.sample(t0 + Duration::from_secs(2), Counters { rx: 2100, tx: 600 });
        assert_eq!((d, u), (1000.0, 250.0));
        // counter reset never produces negative/huge values
        let (d, _) = m.sample(t0 + Duration::from_secs(3), Counters { rx: 5, tx: 600 });
        assert_eq!(d, 0.0);
    }
}
