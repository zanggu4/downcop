//! Helpers for splitting a WinDivert receive batch into individual packets.

#![cfg_attr(not(windows), allow(dead_code))]

/// Total length of the IP packet at the start of `buf`, from its header.
pub fn ip_packet_len(buf: &[u8]) -> Option<usize> {
    let version = buf.first()? >> 4;
    let len = match version {
        4 if buf.len() >= 20 => u16::from_be_bytes([buf[2], buf[3]]) as usize,
        6 if buf.len() >= 40 => 40 + u16::from_be_bytes([buf[4], buf[5]]) as usize,
        _ => return None,
    };
    (len > 0).then_some(len)
}

/// Splits `buf` (holding `count` concatenated packets) into slices.
///
/// When the batch holds a single packet, the whole buffer is that packet; this
/// also covers oversized (LSO) frames whose length field may be unreliable.
/// Parsing stops early on malformed data.
pub fn split_batch(buf: &[u8], count: usize) -> Vec<&[u8]> {
    if count <= 1 {
        return if buf.is_empty() { Vec::new() } else { vec![buf] };
    }
    let mut out = Vec::with_capacity(count);
    let mut rest = buf;
    while out.len() < count && !rest.is_empty() {
        let len = if out.len() + 1 == count {
            rest.len()
        } else {
            match ip_packet_len(rest) {
                Some(l) if l <= rest.len() => l,
                _ => break,
            }
        };
        let (pkt, tail) = rest.split_at(len);
        out.push(pkt);
        rest = tail;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(len: u16) -> Vec<u8> {
        let mut p = vec![0u8; len as usize];
        p[0] = 0x45;
        p[2..4].copy_from_slice(&len.to_be_bytes());
        p
    }

    fn v6(payload: u16) -> Vec<u8> {
        let mut p = vec![0u8; 40 + payload as usize];
        p[0] = 0x60;
        p[4..6].copy_from_slice(&payload.to_be_bytes());
        p
    }

    #[test]
    fn lengths() {
        assert_eq!(ip_packet_len(&v4(60)), Some(60));
        assert_eq!(ip_packet_len(&v6(20)), Some(60));
        assert_eq!(ip_packet_len(&[0x45, 0]), None);
        assert_eq!(ip_packet_len(&[]), None);
        assert_eq!(ip_packet_len(&[0x10; 40]), None);
    }

    #[test]
    fn splits_mixed_batch() {
        let mut buf = v4(40);
        buf.extend(v6(100));
        buf.extend(v4(1500));
        let parts = split_batch(&buf, 3);
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), vec![40, 140, 1500]);
    }

    #[test]
    fn single_packet_uses_whole_buffer() {
        let mut buf = v4(1500);
        buf[2..4].copy_from_slice(&0u16.to_be_bytes()); // bogus length field, e.g. LSO
        assert_eq!(split_batch(&buf, 1), vec![&buf[..]]);
        assert!(split_batch(&[], 1).is_empty());
    }

    #[test]
    fn stops_on_garbage() {
        let mut buf = v4(40);
        buf.extend([0xFFu8; 10]);
        buf.extend(v4(40));
        assert_eq!(split_batch(&buf, 3).len(), 1);
    }
}
