//! Builds WinDivert filter strings from a traffic policy.
//!
//! Only the directions that actually need intervention are captured, so a
//! direction set to "pass" costs nothing. Loopback is always excluded.

use crate::policy::Policy;

/// IPv4 ranges treated as "local network" (inclusive start/end).
const LAN_V4: &[(&str, &str)] = &[
    ("10.0.0.0", "10.255.255.255"),
    ("172.16.0.0", "172.31.255.255"),
    ("192.168.0.0", "192.168.255.255"),
    ("169.254.0.0", "169.254.255.255"),
];
/// IPv4 multicast, reserved and limited broadcast (224.0.0.0 and up).
const MULTICAST_V4: &str = "224.0.0.0";

/// IPv6 ranges treated as "local network": unique local (fc00::/7) and
/// link-local (fe80::/10).
const LAN_V6: &[(&str, &str)] =
    &[("fc00::", "fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"), ("fe80::", "febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff")];
/// IPv6 multicast (ff00::/8).
const MULTICAST_V6: &str = "ff00::";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Inbound,
    Outbound,
}

impl Direction {
    fn keyword(self) -> &'static str {
        match self {
            Direction::Inbound => "inbound",
            Direction::Outbound => "outbound",
        }
    }

    /// Header field holding the *remote* peer address for this direction.
    fn peer_field(self) -> &'static str {
        match self {
            Direction::Inbound => "SrcAddr",
            Direction::Outbound => "DstAddr",
        }
    }
}

fn range_test(field: &str, lo: &str, hi: &str) -> String {
    format!("({field} >= {lo} and {field} <= {hi})")
}

/// Expression that is true when the packet belongs to local network traffic
/// for the given direction (peer in a private range, or multicast/broadcast).
fn lan_expr(dir: Direction) -> String {
    let peer = dir.peer_field();

    let mut v4: Vec<String> = LAN_V4.iter().map(|(lo, hi)| range_test(&format!("ip.{peer}"), lo, hi)).collect();
    v4.push(format!("ip.DstAddr >= {MULTICAST_V4}"));

    let mut v6: Vec<String> = LAN_V6.iter().map(|(lo, hi)| range_test(&format!("ipv6.{peer}"), lo, hi)).collect();
    v6.push(format!("ipv6.DstAddr >= {MULTICAST_V6}"));

    format!("((ip and ({})) or (ipv6 and ({})))", v4.join(" or "), v6.join(" or "))
}

/// Logical negation of a compound expression. WinDivert's `not` only applies
/// to a single field test, so use its conditional operator instead.
fn negate(expr: &str) -> String {
    format!("({expr} ? false : true)")
}

fn direction_clause(dir: Direction, lan_exclude: bool) -> String {
    if lan_exclude { format!("({} and {})", dir.keyword(), negate(&lan_expr(dir))) } else { dir.keyword().to_string() }
}

/// Returns the WinDivert filter for `policy`, or `None` when no traffic needs
/// to be captured at all (both directions pass).
pub fn build(policy: &Policy) -> Option<String> {
    let mut clauses = Vec::new();
    if policy.download.intervenes() {
        clauses.push(direction_clause(Direction::Inbound, policy.lan_exclude));
    }
    if policy.upload.intervenes() {
        clauses.push(direction_clause(Direction::Outbound, policy.lan_exclude));
    }
    if clauses.is_empty() {
        return None;
    }
    Some(format!("(ip or ipv6) and not loopback and ({})", clauses.join(" or ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::Action;

    fn policy(download: Action, upload: Action, lan_exclude: bool) -> Policy {
        Policy { download, upload, lan_exclude }
    }

    #[test]
    fn nothing_to_capture_when_all_pass() {
        assert_eq!(build(&policy(Action::Pass, Action::Pass, false)), None);
        assert_eq!(build(&policy(Action::Pass, Action::Pass, true)), None);
    }

    #[test]
    fn download_only_captures_inbound() {
        let f = build(&policy(Action::Block, Action::Pass, false)).unwrap();
        assert_eq!(f, "(ip or ipv6) and not loopback and (inbound)");
    }

    #[test]
    fn upload_only_captures_outbound() {
        let f = build(&policy(Action::Pass, Action::Limit(1000), false)).unwrap();
        assert_eq!(f, "(ip or ipv6) and not loopback and (outbound)");
    }

    #[test]
    fn both_directions() {
        let f = build(&policy(Action::Limit(1), Action::Block, false)).unwrap();
        assert_eq!(f, "(ip or ipv6) and not loopback and (inbound or outbound)");
    }

    #[test]
    fn lan_exclusion_uses_peer_address_per_direction() {
        let f = build(&policy(Action::Block, Action::Block, true)).unwrap();
        assert!(f.starts_with("(ip or ipv6) and not loopback and ((inbound and (((ip and "));
        assert!(f.contains(" ? false : true)"));
        // `not` must only ever precede a plain field (WinDivert grammar)
        assert!(!f.contains("not ("), "{f}");
        // inbound checks the source (remote) address, outbound the destination
        assert!(f.contains("(ip.SrcAddr >= 192.168.0.0 and ip.SrcAddr <= 192.168.255.255)"));
        assert!(f.contains("(ip.DstAddr >= 192.168.0.0 and ip.DstAddr <= 192.168.255.255)"));
        assert!(f.contains("(ipv6.SrcAddr >= fe80:: and ipv6.SrcAddr <= febf:"));
        assert!(f.contains("ip.DstAddr >= 224.0.0.0"));
        assert!(f.contains("ipv6.DstAddr >= ff00::"));
        assert!(f.contains("172.31.255.255"));
        assert!(f.contains("10.255.255.255"));
        assert!(f.contains("169.254.255.255"));
        assert!(f.contains("fdff:ffff"));
    }

    #[test]
    fn parentheses_are_balanced() {
        for lan in [false, true] {
            for (d, u) in
                [(Action::Block, Action::Pass), (Action::Pass, Action::Block), (Action::Block, Action::Limit(5))]
            {
                let f = build(&policy(d, u, lan)).unwrap();
                let mut depth = 0i32;
                for c in f.chars() {
                    match c {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                    assert!(depth >= 0, "{f}");
                }
                assert_eq!(depth, 0, "{f}");
            }
        }
    }
}
