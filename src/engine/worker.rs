//! Capture / shaping threads.
//!
//! One receive thread pulls batches of packets from WinDivert and dispatches
//! them per direction: blocked packets are dropped, passed packets are
//! re-injected immediately and limited packets go to that direction's queue.
//! One shaper thread per direction releases queued packets at the configured
//! rate.

use super::EngineError;
use super::windivert::{self, Address, Divert};
use crate::config::log;
use crate::packet::split_batch;
use crate::policy::{Action, Policy};
use crate::shaper::{PacketQueue, TokenBucket, queue_capacity_for};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

const MODE_PASS: u8 = 0;
const MODE_BLOCK: u8 = 1;
const MODE_LIMIT: u8 = 2;

const RECV_BUF: usize = 1 << 20;

// Win32 error codes seen from WinDivertRecvEx
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_NO_DATA: u32 = 232;
const ERROR_OPERATION_ABORTED: u32 = 995;

struct Packet {
    data: Vec<u8>,
    addr: Address,
}

struct ShapeState {
    bucket: TokenBucket,
    queue: PacketQueue<Packet>,
    stop: bool,
}

struct DirShared {
    mode: AtomicU8,
    state: Mutex<ShapeState>,
    cond: Condvar,
}

impl DirShared {
    fn new(action: Action) -> Self {
        let rate = match action {
            Action::Limit(r) => r,
            _ => 1,
        };
        DirShared {
            mode: AtomicU8::new(MODE_PASS),
            state: Mutex::new(ShapeState {
                bucket: TokenBucket::new(rate, Instant::now()),
                queue: PacketQueue::new(queue_capacity_for(rate)),
                stop: false,
            }),
            cond: Condvar::new(),
        }
        .with_action(action)
    }

    fn with_action(self, action: Action) -> Self {
        self.set_action(action);
        self
    }

    fn set_action(&self, action: Action) {
        let mode = match action {
            Action::Pass => MODE_PASS,
            Action::Block => MODE_BLOCK,
            Action::Limit(rate) => {
                let mut st = self.state.lock().unwrap();
                st.bucket.set_rate(rate);
                st.queue.set_capacity(queue_capacity_for(rate));
                MODE_LIMIT
            }
        };
        self.mode.store(mode, Ordering::Release);
        self.cond.notify_all();
    }
}

pub struct Running {
    filter: String,
    divert: Arc<Divert>,
    dirs: [Arc<DirShared>; 2], // [inbound, outbound]
    stopping: Arc<AtomicBool>,
    recv_thread: Option<JoinHandle<()>>,
    shaper_threads: Vec<JoinHandle<()>>,
}

impl Running {
    pub fn start(filter: String, policy: &Policy) -> Result<Running, EngineError> {
        let divert = Arc::new(Divert::open(&filter)?);
        // Give ourselves headroom so the kernel doesn't drop packets while the
        // shaper is holding others back.
        divert.set_param(windivert::PARAM_QUEUE_LENGTH, 8192);
        divert.set_param(windivert::PARAM_QUEUE_TIME, 2000);
        divert.set_param(windivert::PARAM_QUEUE_SIZE, 32 * 1024 * 1024);

        let dirs = [Arc::new(DirShared::new(policy.download)), Arc::new(DirShared::new(policy.upload))];
        let stopping = Arc::new(AtomicBool::new(false));

        let mut shaper_threads = Vec::new();
        for (i, dir) in dirs.iter().enumerate() {
            let dir = dir.clone();
            let divert = divert.clone();
            let name = if i == 0 { "downcop-shaper-in" } else { "downcop-shaper-out" };
            shaper_threads.push(
                std::thread::Builder::new()
                    .name(name.into())
                    .spawn(move || shaper_loop(&divert, &dir))
                    .map_err(|e| EngineError::Other(e.to_string()))?,
            );
        }

        let recv_thread = {
            let divert = divert.clone();
            let dirs = dirs.clone();
            let stopping = stopping.clone();
            std::thread::Builder::new()
                .name("downcop-recv".into())
                .spawn(move || recv_loop(&divert, &dirs, &stopping))
                .map_err(|e| EngineError::Other(e.to_string()))?
        };

        Ok(Running { filter, divert, dirs, stopping, recv_thread: Some(recv_thread), shaper_threads })
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn update(&self, policy: &Policy) {
        self.dirs[0].set_action(policy.download);
        self.dirs[1].set_action(policy.upload);
    }

    pub fn stop(mut self) {
        self.stopping.store(true, Ordering::Release);
        self.divert.shutdown(windivert::SHUTDOWN_RECV);
        if let Some(t) = self.recv_thread.take() {
            let _ = t.join();
        }
        for d in &self.dirs {
            d.state.lock().unwrap().stop = true;
            d.cond.notify_all();
        }
        for t in self.shaper_threads.drain(..) {
            let _ = t.join();
        }
        // dropping the last Arc<Divert> closes the handle
    }
}

fn recv_loop(divert: &Divert, dirs: &[Arc<DirShared>; 2], stopping: &AtomicBool) {
    let mut buf = vec![0u8; RECV_BUF];
    let mut addrs = vec![Address::zeroed(); windivert::BATCH_MAX];
    loop {
        let (len, count) = match divert.recv_batch(&mut buf, &mut addrs) {
            Ok(r) => r,
            Err(code) => {
                if stopping.load(Ordering::Acquire)
                    || matches!(code, ERROR_NO_DATA | ERROR_INVALID_HANDLE | ERROR_OPERATION_ABORTED)
                {
                    break;
                }
                if code != ERROR_INSUFFICIENT_BUFFER {
                    log(&format!("WinDivertRecvEx failed: {code}"));
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                continue;
            }
        };
        let packets = split_batch(&buf[..len], count);
        for (pkt, addr) in packets.into_iter().zip(addrs.iter()) {
            let dir = &dirs[addr.outbound() as usize];
            match dir.mode.load(Ordering::Acquire) {
                MODE_BLOCK => {}
                MODE_LIMIT => {
                    let mut st = dir.state.lock().unwrap();
                    if st.queue.push(Packet { data: pkt.to_vec(), addr: *addr }, pkt.len()) {
                        drop(st);
                        dir.cond.notify_one();
                    }
                }
                _ => {
                    divert.send(pkt, addr);
                }
            }
        }
    }
}

fn shaper_loop(divert: &Divert, dir: &DirShared) {
    let mut st = dir.state.lock().unwrap();
    loop {
        let mode = dir.mode.load(Ordering::Acquire);
        if st.stop || mode != MODE_LIMIT {
            // Leaving limit mode: deliver (or, when blocking, discard) what we hold.
            if !st.queue.is_empty() {
                let pending: Vec<_> = st.queue.drain_all().collect();
                drop(st);
                if mode != MODE_BLOCK {
                    for (p, _) in pending {
                        divert.send(&p.data, &p.addr);
                    }
                }
                st = dir.state.lock().unwrap();
                continue;
            }
            if st.stop {
                return;
            }
            st = dir.cond.wait(st).unwrap();
            continue;
        }
        if st.queue.is_empty() {
            st = dir.cond.wait(st).unwrap();
            continue;
        }
        let now = Instant::now();
        let wait = st.bucket.time_until_ready(now);
        if !wait.is_zero() {
            st = dir.cond.wait_timeout(st, wait).unwrap().0;
            continue;
        }
        let (p, len) = st.queue.pop().expect("queue not empty");
        st.bucket.consume(len, now);
        drop(st);
        divert.send(&p.data, &p.addr);
        st = dir.state.lock().unwrap();
    }
}
