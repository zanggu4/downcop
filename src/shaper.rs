//! Rate limiting primitives: a token bucket and a byte-bounded packet queue.
//!
//! Pure logic (no OS calls) so it can be unit tested on any platform. The
//! engine feeds captured packets into a [`PacketQueue`] and releases them as
//! the [`TokenBucket`] allows.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Smallest burst we allow, roughly two full-size Ethernet frames.
const MIN_BURST: f64 = 3_000.0;
/// Burst window: how much traffic may be sent back-to-back after idling.
const BURST_WINDOW_SECS: f64 = 0.05;

/// Classic token bucket that allows going into "debt".
///
/// A packet may be sent whenever the balance is non-negative; its size is
/// then subtracted, possibly driving the balance negative. This means packets
/// bigger than the burst size (e.g. LSO super-frames) are never starved, while
/// the long-term average still converges to `rate`.
#[derive(Debug, Clone)]
pub struct TokenBucket {
    rate: f64,
    burst: f64,
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    pub fn new(rate_bytes_per_sec: u64, now: Instant) -> Self {
        let mut b = TokenBucket { rate: 1.0, burst: MIN_BURST, tokens: 0.0, last: now };
        b.set_rate(rate_bytes_per_sec);
        b.tokens = b.burst;
        b
    }

    #[cfg(test)]
    pub fn rate(&self) -> u64 {
        self.rate as u64
    }

    pub fn set_rate(&mut self, rate_bytes_per_sec: u64) {
        self.rate = (rate_bytes_per_sec.max(1)) as f64;
        self.burst = (self.rate * BURST_WINDOW_SECS).max(MIN_BURST);
        self.tokens = self.tokens.min(self.burst);
    }

    fn refill(&mut self, now: Instant) {
        if now > self.last {
            let dt = now.duration_since(self.last).as_secs_f64();
            self.tokens = (self.tokens + dt * self.rate).min(self.burst);
            self.last = now;
        }
    }

    /// How long until a packet may be sent. `Duration::ZERO` means "now".
    pub fn time_until_ready(&mut self, now: Instant) -> Duration {
        self.refill(now);
        if self.tokens >= 0.0 { Duration::ZERO } else { Duration::from_secs_f64(-self.tokens / self.rate) }
    }

    /// Charges `bytes` against the bucket. Call only after
    /// [`time_until_ready`](Self::time_until_ready) returned zero.
    pub fn consume(&mut self, bytes: usize, now: Instant) {
        self.refill(now);
        self.tokens -= bytes as f64;
    }
}

/// Queue capacity for a given rate: about one second of traffic, clamped.
pub fn queue_capacity_for(rate_bytes_per_sec: u64) -> usize {
    (rate_bytes_per_sec as usize).clamp(64 * 1024, 4 * 1024 * 1024)
}

/// FIFO of packets bounded by total byte size (tail drop when full).
#[derive(Debug)]
pub struct PacketQueue<T> {
    items: VecDeque<(T, usize)>,
    bytes: usize,
    capacity: usize,
}

impl<T> PacketQueue<T> {
    pub fn new(capacity_bytes: usize) -> Self {
        PacketQueue { items: VecDeque::new(), bytes: 0, capacity: capacity_bytes }
    }

    pub fn set_capacity(&mut self, capacity_bytes: usize) {
        self.capacity = capacity_bytes;
    }

    /// Enqueues a packet. Returns `false` (and drops it) when the queue is full.
    pub fn push(&mut self, item: T, len: usize) -> bool {
        if self.bytes + len > self.capacity && !self.items.is_empty() {
            return false;
        }
        self.bytes += len;
        self.items.push_back((item, len));
        true
    }

    pub fn pop(&mut self) -> Option<(T, usize)> {
        let (item, len) = self.items.pop_front()?;
        self.bytes -= len;
        Some((item, len))
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[cfg(test)]
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn drain_all(&mut self) -> impl Iterator<Item = (T, usize)> + '_ {
        self.bytes = 0;
        self.items.drain(..)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_starts_with_burst_and_goes_into_debt() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(100_000, t0); // 100 KB/s → burst 5000
        assert_eq!(b.time_until_ready(t0), Duration::ZERO);
        b.consume(5_000, t0);
        assert_eq!(b.time_until_ready(t0), Duration::ZERO); // exactly 0 tokens
        b.consume(1_500, t0);
        let wait = b.time_until_ready(t0);
        // 1500 bytes of debt at 100 KB/s = 15 ms
        assert!((wait.as_secs_f64() - 0.015).abs() < 1e-6, "{wait:?}");
        assert_eq!(b.time_until_ready(t0 + Duration::from_millis(15)), Duration::ZERO);
    }

    #[test]
    fn bucket_long_term_rate_matches() {
        let t0 = Instant::now();
        let rate = 1_000_000u64;
        let mut b = TokenBucket::new(rate, t0);
        let mut now = t0;
        let mut sent = 0usize;
        let end = t0 + Duration::from_secs(10);
        while now < end {
            let w = b.time_until_ready(now);
            if w.is_zero() {
                b.consume(1_500, now);
                sent += 1_500;
            } else {
                now += w;
            }
        }
        let expected = rate as f64 * 10.0;
        let err = (sent as f64 - expected).abs() / expected;
        assert!(err < 0.01, "sent={sent} expected={expected}");
    }

    #[test]
    fn bucket_does_not_accumulate_beyond_burst() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(10_000, t0); // burst = MIN_BURST
        let later = t0 + Duration::from_secs(60);
        b.time_until_ready(later);
        b.consume(MIN_BURST as usize, later);
        b.consume(1, later);
        assert!(b.time_until_ready(later) > Duration::ZERO);
    }

    #[test]
    fn bucket_rate_change_applies() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(1_000, t0);
        b.consume(10_000, t0);
        let slow = b.time_until_ready(t0);
        b.set_rate(1_000_000);
        let fast = b.time_until_ready(t0);
        assert!(fast < slow);
        assert_eq!(b.rate(), 1_000_000);
    }

    #[test]
    fn queue_tail_drops_when_full() {
        let mut q = PacketQueue::new(3_000);
        assert!(q.push(1, 1_500));
        assert!(q.push(2, 1_500));
        assert!(!q.push(3, 1));
        assert_eq!(q.len(), 2);
        assert_eq!(q.pop(), Some((1, 1_500)));
        assert_eq!(q.bytes(), 1_500);
        assert!(q.push(4, 1_500));
        let drained: Vec<_> = q.drain_all().map(|(v, _)| v).collect();
        assert_eq!(drained, vec![2, 4]);
        assert!(q.is_empty());
        assert_eq!(q.bytes(), 0);
    }

    #[test]
    fn queue_accepts_oversized_packet_when_empty() {
        let mut q = PacketQueue::new(1_000);
        assert!(q.push((), 64_000));
        assert!(!q.push((), 10));
    }

    #[test]
    fn capacity_is_clamped() {
        assert_eq!(queue_capacity_for(1), 64 * 1024);
        assert_eq!(queue_capacity_for(1_000_000), 1_000_000);
        assert_eq!(queue_capacity_for(u64::MAX), 4 * 1024 * 1024);
    }
}
