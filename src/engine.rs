use crate::windivert_ffi::{DivertHandle, WindivertAddress, WINDIVERT_MTU_MAX};
use std::cmp::Ordering as CmpOrdering;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_QUEUE_PACKETS: usize = 8192;
const MAX_BW_QUEUE_WAIT_SECS: f64 = 1.5;

/// GUI <-> 워커 스레드 간 실시간 공유 설정 (재시작 없이 즉시 반영)
pub struct LiveConfig {
    pub lag_enabled: AtomicBool,
    pub lag_inbound: AtomicBool,
    pub lag_outbound: AtomicBool,
    pub lag_ms: AtomicU64,

    pub bw_enabled: AtomicBool,
    pub bw_inbound: AtomicBool,
    pub bw_outbound: AtomicBool,
    pub bw_kbps: AtomicU64,
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            lag_enabled: AtomicBool::new(true),
            lag_inbound: AtomicBool::new(true),
            lag_outbound: AtomicBool::new(true),
            lag_ms: AtomicU64::new(150),

            bw_enabled: AtomicBool::new(true),
            bw_inbound: AtomicBool::new(true),
            bw_outbound: AtomicBool::new(true),
            bw_kbps: AtomicU64::new(50),
        }
    }
}

/// 실시간 패킷 통계
#[derive(Default)]
pub struct Stats {
    pub rx_packets: AtomicU64,
    pub tx_packets: AtomicU64,
    pub dropped_packets: AtomicU64,
    pub queued_packets: AtomicU64,
    pub rx_bytes_total: AtomicU64,
    pub tx_bytes_total: AtomicU64,
}

impl Stats {
    pub fn reset(&self) {
        self.rx_packets.store(0, Ordering::Relaxed);
        self.tx_packets.store(0, Ordering::Relaxed);
        self.dropped_packets.store(0, Ordering::Relaxed);
        self.queued_packets.store(0, Ordering::Relaxed);
        self.rx_bytes_total.store(0, Ordering::Relaxed);
        self.tx_bytes_total.store(0, Ordering::Relaxed);
    }
}

/// 가상 스케줄 기반 Token Bucket (패킷 버림 대신 목표 전송 시각으로 매끄럽게 셰이핑)
struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new() -> Self {
        Self {
            tokens: 4096.0,
            last_refill: Instant::now(),
        }
    }

    /// 통과 가능 시 필요한 추가 대기 시간(Duration) 반환, 한계 초과 시 None(Tail-Drop)
    fn schedule(&mut self, now: Instant, pkt_len: usize, bw_kbps: u64) -> Option<Duration> {
        if bw_kbps == 0 {
            return None;
        }

        let rate_bps = (bw_kbps as f64) * 1024.0;
        // 버스트 상한: 0.2초치 트래픽 또는 최소 4KB (약 2~3 MTU 패킷)
        let burst_cap = (rate_bps * 0.2).max(4096.0);

        let elapsed = now.saturating_duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * rate_bps).min(burst_cap);
        self.last_refill = now;

        let cost = pkt_len as f64;
        if self.tokens >= cost {
            self.tokens -= cost;
            Some(Duration::ZERO)
        } else {
            let new_tokens = self.tokens - cost;
            let wait_secs = (-new_tokens) / rate_bps;
            if wait_secs > MAX_BW_QUEUE_WAIT_SECS {
                None
            } else {
                self.tokens = new_tokens;
                Some(Duration::from_secs_f64(wait_secs))
            }
        }
    }
}

struct DelayedPacket {
    send_at: Instant,
    seq: u64,
    data: Vec<u8>,
    addr: WindivertAddress,
}

impl PartialEq for DelayedPacket {
    fn eq(&self, other: &Self) -> bool {
        self.send_at == other.send_at && self.seq == other.seq
    }
}
impl Eq for DelayedPacket {}

impl Ord for DelayedPacket {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        // Min-heap: send_at이 빠른 패킷이 먼저 나오도록 역순 비교
        other
            .send_at
            .cmp(&self.send_at)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for DelayedPacket {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

struct QueueState {
    heap: BinaryHeap<DelayedPacket>,
    shutdown: bool,
}

struct PacketQueue {
    lock: Mutex<QueueState>,
    cvar: Condvar,
}

impl PacketQueue {
    fn new() -> Self {
        Self {
            lock: Mutex::new(QueueState {
                heap: BinaryHeap::new(),
                shutdown: false,
            }),
            cvar: Condvar::new(),
        }
    }
}

pub struct EngineHandle {
    handle: Arc<DivertHandle>,
    running: Arc<AtomicBool>,
    queue: Arc<PacketQueue>,
    recv_thread: Option<JoinHandle<()>>,
    send_thread: Option<JoinHandle<()>>,
}

impl EngineHandle {
    pub fn start(
        filter: &str,
        cfg: Arc<LiveConfig>,
        stats: Arc<Stats>,
    ) -> Result<Self, String> {
        let handle = Arc::new(DivertHandle::open(filter)?);
        stats.reset();

        let running = Arc::new(AtomicBool::new(true));
        let queue = Arc::new(PacketQueue::new());

        let recv_thread = {
            let handle = Arc::clone(&handle);
            let running = Arc::clone(&running);
            let queue = Arc::clone(&queue);
            let cfg = Arc::clone(&cfg);
            let stats = Arc::clone(&stats);

            thread::spawn(move || {
                let mut buf = vec![0u8; WINDIVERT_MTU_MAX];
                let mut addr = WindivertAddress::default();
                let mut bucket = TokenBucket::new();
                let mut seq: u64 = 0;

                while running.load(Ordering::Acquire) {
                    let len = match handle.recv(&mut buf, &mut addr) {
                        Ok(n) => n,
                        Err(232) => break, // ERROR_NO_DATA: WinDivertShutdown 호출됨
                        Err(_) => {
                            if !running.load(Ordering::Acquire) {
                                break;
                            }
                            continue;
                        }
                    };

                    stats.rx_packets.fetch_add(1, Ordering::Relaxed);
                    stats.rx_bytes_total.fetch_add(len as u64, Ordering::Relaxed);

                    let outbound = addr.is_outbound();
                    let lag_applies = cfg.lag_enabled.load(Ordering::Relaxed)
                        && ((outbound && cfg.lag_outbound.load(Ordering::Relaxed))
                            || (!outbound && cfg.lag_inbound.load(Ordering::Relaxed)));
                    let bw_applies = cfg.bw_enabled.load(Ordering::Relaxed)
                        && ((outbound && cfg.bw_outbound.load(Ordering::Relaxed))
                            || (!outbound && cfg.bw_inbound.load(Ordering::Relaxed)));

                    // 둘 다 해당 없으면 큐/할당 없이 즉시 바이패스 전송
                    if !lag_applies && !bw_applies {
                        if handle.send(&buf[..len], &addr).is_ok() {
                            stats.tx_packets.fetch_add(1, Ordering::Relaxed);
                            stats.tx_bytes_total.fetch_add(len as u64, Ordering::Relaxed);
                        }
                        continue;
                    }

                    let now = Instant::now();
                    let bw_delay = if bw_applies {
                        let kbps = cfg.bw_kbps.load(Ordering::Relaxed);
                        match bucket.schedule(now, len, kbps) {
                            Some(d) => d,
                            None => {
                                stats.dropped_packets.fetch_add(1, Ordering::Relaxed);
                                continue;
                            }
                        }
                    } else {
                        Duration::ZERO
                    };

                    let lag_delay = if lag_applies {
                        Duration::from_millis(cfg.lag_ms.load(Ordering::Relaxed))
                    } else {
                        Duration::ZERO
                    };

                    let send_at = now + bw_delay + lag_delay;
                    seq = seq.wrapping_add(1);

                    let mut state = queue.lock.lock().unwrap();
                    if state.heap.len() >= MAX_QUEUE_PACKETS {
                        stats.dropped_packets.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }

                    let should_notify = state
                        .heap
                        .peek()
                        .map_or(true, |top| send_at < top.send_at);

                    state.heap.push(DelayedPacket {
                        send_at,
                        seq,
                        data: buf[..len].to_vec(),
                        addr,
                    });
                    stats
                        .queued_packets
                        .store(state.heap.len() as u64, Ordering::Relaxed);

                    if should_notify {
                        queue.cvar.notify_one();
                    }
                }
            })
        };

        let send_thread = {
            let handle = Arc::clone(&handle);
            let queue = Arc::clone(&queue);
            let stats = Arc::clone(&stats);

            thread::spawn(move || {
                let sleeper = spin_sleep::SpinSleeper::default();
                let mut state = queue.lock.lock().unwrap();

                loop {
                    if state.shutdown {
                        // 종료 시 대기 중인 패킷을 즉시 모두 방출해 연결 유실 방지
                        while let Some(pkt) = state.heap.pop() {
                            stats
                                .queued_packets
                                .store(state.heap.len() as u64, Ordering::Relaxed);
                            drop(state);
                            if handle.send(&pkt.data, &pkt.addr).is_ok() {
                                stats.tx_packets.fetch_add(1, Ordering::Relaxed);
                                stats
                                    .tx_bytes_total
                                    .fetch_add(pkt.data.len() as u64, Ordering::Relaxed);
                            }
                            state = queue.lock.lock().unwrap();
                        }
                        break;
                    }

                    if let Some(top) = state.heap.peek() {
                        let now = Instant::now();
                        if top.send_at <= now {
                            let pkt = state.heap.pop().unwrap();
                            stats
                                .queued_packets
                                .store(state.heap.len() as u64, Ordering::Relaxed);
                            drop(state);
                            if handle.send(&pkt.data, &pkt.addr).is_ok() {
                                stats.tx_packets.fetch_add(1, Ordering::Relaxed);
                                stats
                                    .tx_bytes_total
                                    .fetch_add(pkt.data.len() as u64, Ordering::Relaxed);
                            }
                            state = queue.lock.lock().unwrap();
                            continue;
                        }

                        let wait = top.send_at - now;
                        if wait > Duration::from_millis(20) {
                            let (new_state, _) = queue
                                .cvar
                                .wait_timeout(state, wait - Duration::from_millis(18))
                                .unwrap();
                            state = new_state;
                        } else {
                            drop(state);
                            sleeper.sleep(wait.min(Duration::from_millis(4)));
                            state = queue.lock.lock().unwrap();
                        }
                    } else {
                        state = queue.cvar.wait(state).unwrap();
                    }
                }
            })
        };

        Ok(Self {
            handle,
            running,
            queue,
            recv_thread: Some(recv_thread),
            send_thread: Some(send_thread),
        })
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Release);
        // Recv 블로킹 즉시 해제
        self.handle.shutdown_recv();

        if let Some(t) = self.recv_thread.take() {
            let _ = t.join();
        }

        {
            let mut state = self.queue.lock.lock().unwrap();
            state.shutdown = true;
            self.queue.cvar.notify_all();
        }

        if let Some(t) = self.send_thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        self.stop();
    }
}