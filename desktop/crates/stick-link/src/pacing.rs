//! Write-without-response pacing, without I/O: how a packet waits for room in CoreBluetooth's
//! queue (`canSendWriteWithoutResponse`), shared by `corebt.rs` and the fake stick's radio model.
//!
//! CoreBluetooth drops a write without response when its queue is full, so each packet waits for
//! room first. The room comes back in two ways: `peripheralIsReadyToSendWriteWithoutResponse:`
//! (the delegate), and `canSendWriteWithoutResponse` turning true again. They don't line up: the
//! callback can come later than the flag (the queue drained further, or a whole connection event
//! later), and sometimes not at all. Waiting for the callback alone was what made clips slower
//! than link.py, which polls the flag every 5 ms. So [`wait_and_send`] does both: it wakes on the
//! callback *or* every [`POLL`], whichever comes first, and after [`ROOM_TIMEOUT`] sends anyway
//! (the protocols cope with a lost packet).

use crate::transport::BleError;
use std::time::Duration;
use tokio::sync::Notify;

/// How often the flag is checked while waiting (link.py: 5 ms for clips, 2 ms for firmware).
pub const POLL: Duration = Duration::from_millis(2);
/// A packet waits this long for room, then goes anyway (link.py: ~2 s).
pub const ROOM_TIMEOUT: Duration = Duration::from_secs(2);

/// What one paced write did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Paced {
    /// times there was no room at first
    pub waited: u32,
    /// sent after the timeout without room
    pub late: bool,
}

/// Waits for room and sends. `try_send(late)` checks the flag and, if there's room (or `late`),
/// queues the packet in the same step, returning `Ok(true)`; `Ok(false)` means no room yet;
/// `Err` (disconnected) ends it. `ready` is notified by the delegate's "ready to send";
/// `poll` is [`POLL`] (`None`: the callback only, kept for the timing test's comparison).
pub async fn wait_and_send(
    ready: &Notify,
    poll: Option<Duration>,
    timeout: Duration,
    mut try_send: impl FnMut(bool) -> Result<bool, BleError>,
) -> Result<Paced, BleError> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut out = Paced::default();
    loop {
        let notified = ready.notified();
        tokio::pin!(notified);
        notified.as_mut().enable(); // before the check: a wake-up in between isn't lost
        let late = tokio::time::Instant::now() >= deadline;
        if try_send(late)? {
            out.late = late;
            return Ok(out);
        }
        out.waited += 1;
        let until = match poll {
            Some(p) => deadline.min(tokio::time::Instant::now() + p),
            None => deadline,
        };
        let _ = tokio::time::timeout_at(until, notified).await;
    }
}

/// A model of CoreBluetooth's write-without-response queue and the radio behind it, for the fake
/// stick's timing tests: packets leave `per_event` at a time every connection `interval`;
/// `capacity` packets fit in the queue; the "ready to send" callback fires `ready_delay` after the
/// queue is down to `ready_at` packets (0: drained), or never (`ready_at: None`, as seen on some
/// macOS versions); a write with response is answered `ack_events` events after the queue drained.
#[derive(Debug, Clone)]
pub struct Radio {
    pub interval: Duration,
    pub per_event: usize,
    pub capacity: usize,
    pub ready_at: Option<usize>,
    pub ready_delay: Duration,
    pub ack_events: u32,
}

impl Radio {
    /// A macOS-like link to the stick after `fastLink()`: 15 ms interval, 4 long packets per
    /// event, 8 in the queue, the callback one event after it drained, an answer 2 events later.
    pub fn mac() -> Self {
        Radio {
            interval: Duration::from_millis(15),
            per_event: 4,
            capacity: 8,
            ready_at: Some(0),
            ready_delay: Duration::from_millis(15),
            ack_events: 2,
        }
    }
}

/// The queue's state in [`Radio`] time (tokio's clock, so paused-time tests run instantly).
#[derive(Debug)]
pub struct RadioQueue {
    pub radio: Radio,
    start: tokio::time::Instant,
    /// connection events handled so far (since `start`)
    events: u64,
    queued: usize,
}

impl RadioQueue {
    pub fn new(radio: Radio) -> Self {
        Self { radio, start: tokio::time::Instant::now(), events: 0, queued: 0 }
    }

    fn event_at(&self, n: u64) -> tokio::time::Instant {
        self.start + self.radio.interval * n as u32
    }

    /// Lets the connection events up to now send what's queued.
    pub fn advance(&mut self) {
        let n = (self.start.elapsed().as_nanos() / self.radio.interval.as_nanos().max(1)) as u64;
        if n > self.events {
            let sent = ((n - self.events) as usize).saturating_mul(self.radio.per_event);
            self.queued = self.queued.saturating_sub(sent);
            self.events = n;
        }
    }

    pub fn can_send(&mut self) -> bool {
        self.advance();
        self.queued < self.radio.capacity
    }

    /// Queues one packet (dropped when there's no room, like CoreBluetooth). Returns whether it
    /// was queued, and when the "ready to send" callback will fire if the queue is now full.
    pub fn push(&mut self) -> (bool, Option<tokio::time::Instant>) {
        self.advance();
        if self.queued >= self.radio.capacity {
            return (false, None);
        }
        self.queued += 1;
        let ready = if self.queued >= self.radio.capacity {
            self.radio.ready_at.map(|n| self.drained_to(n) + self.radio.ready_delay)
        } else {
            None
        };
        (true, ready)
    }

    /// When the queue is down to `n` packets.
    pub fn drained_to(&self, n: usize) -> tokio::time::Instant {
        let left = self.queued.saturating_sub(n);
        self.event_at(self.events + left.div_ceil(self.radio.per_event.max(1)) as u64)
    }

    /// When a write with response sent now is answered.
    pub fn ack_at(&self) -> tokio::time::Instant {
        let gone = self.drained_to(0).max(self.event_at(self.events + 1));
        gone + self.radio.interval * self.radio.ack_events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Sends `n` packets through the radio model with the given pacing; returns the time taken.
    async fn send(n: usize, radio: Radio, poll: Option<Duration>) -> (Duration, u32) {
        let q = Arc::new(Mutex::new(RadioQueue::new(radio)));
        let ready = Arc::new(Notify::new());
        let t = tokio::time::Instant::now();
        let mut waits = 0;
        for _ in 0..n {
            let p = wait_and_send(&ready, poll, ROOM_TIMEOUT, |late| {
                let mut q = q.lock().unwrap();
                if !late && !q.can_send() {
                    return Ok(false);
                }
                let (_, at) = q.push();
                if let Some(at) = at {
                    let ready = ready.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep_until(at).await;
                        ready.notify_waiters();
                    });
                }
                Ok(true)
            })
            .await
            .unwrap();
            assert!(!p.late);
            waits += p.waited;
        }
        let done = q.lock().unwrap().drained_to(0);
        tokio::time::sleep_until(done).await;
        (t.elapsed(), waits)
    }

    #[tokio::test(start_paused = true)]
    async fn polling_keeps_the_queue_full() {
        // 40 packets, 4 per 15 ms event: the radio's limit is 10 events = 150 ms
        let (polled, _) = send(40, Radio::mac(), Some(POLL)).await;
        assert!(polled <= Duration::from_millis(155), "{polled:?}");
        // the callback alone (fired only once drained) leaves the radio idle half the time
        let (callback, waits) = send(40, Radio::mac(), None).await;
        assert!(waits > 0);
        assert!(callback >= polled + Duration::from_millis(40), "{callback:?} vs {polled:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_lost_callback_costs_only_a_poll() {
        // the callback never comes: polling still sends at the radio's pace
        let radio = Radio { ready_at: None, ..Radio::mac() };
        let q = Arc::new(Mutex::new(RadioQueue::new(radio)));
        let ready = Notify::new();
        let t = tokio::time::Instant::now();
        for _ in 0..20 {
            wait_and_send(&ready, Some(POLL), ROOM_TIMEOUT, |late| {
                let mut q = q.lock().unwrap();
                if !late && !q.can_send() {
                    return Ok(false);
                }
                q.push();
                Ok(true)
            })
            .await
            .unwrap();
        }
        assert!(t.elapsed() < Duration::from_millis(60), "{:?}", t.elapsed());
    }

    #[tokio::test(start_paused = true)]
    async fn no_room_for_long_sends_anyway() {
        let ready = Notify::new();
        let t = tokio::time::Instant::now();
        let p = wait_and_send(&ready, Some(POLL), ROOM_TIMEOUT, Ok).await.unwrap();
        assert!(p.late && p.waited > 0);
        assert!(t.elapsed() >= ROOM_TIMEOUT);
        let e = wait_and_send(&ready, Some(POLL), ROOM_TIMEOUT, |_| Err(BleError::new("disconnected"))).await;
        assert!(e.is_err());
    }
}
