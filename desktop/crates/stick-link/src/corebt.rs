//! CoreBluetooth through objc2 (macOS). Why not btleplug: see the README. In short, btleplug 0.11
//! drops the error of a failed write on macOS (the write never completes), so "Insufficient
//! Resource" can't be retried, and it doesn't expose `canSendWriteWithoutResponse` or the write
//! length, which the firmware update and sound clips need (both paced here).
//!
//! Every CoreBluetooth call runs on one serial dispatch queue (the manager's), and the delegate's
//! callbacks arrive there too. They turn into Rust events: connection steps on a broadcast
//! channel, notifications straight into the connection's channel, write results into its waiter,
//! "ready to send" (room again for writes without response) into a `Notify`.
//!
//! The central manager is only made when it's first needed (a scan, or a stick to connect to),
//! because making it is what asks macOS for the Bluetooth permission.

use crate::pacing;
use crate::proto::{AUDIO, AUDIO_IN, NUS, OTA_DATA, RX, TX};
use crate::transport::{BleError, Char, Connection, Connector, Found, Incoming, Target};
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, Message, define_class, msg_send};
use objc2_core_bluetooth::{
    CBAdvertisementDataLocalNameKey, CBCentralManager, CBCentralManagerDelegate, CBCharacteristic,
    CBCharacteristicWriteType, CBManagerState, CBPeripheral, CBPeripheralDelegate, CBService, CBUUID,
};
use objc2_foundation::{
    NSArray, NSData, NSDictionary, NSError, NSNumber, NSObject, NSObjectProtocol, NSString, NSUUID,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, OnceCell, broadcast, mpsc, oneshot, watch};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const STEP_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const POWER_TIMEOUT: Duration = Duration::from_secs(5);

/// A CoreBluetooth object that crosses threads. SAFETY: we only *message* these objects on the
/// manager's serial queue (`on_queue`); moving the pointer and retain/release are thread-safe.
struct Obj<T>(Retained<T>);
unsafe impl<T> Send for Obj<T> {}
unsafe impl<T> Sync for Obj<T> {}
impl<T> Obj<T> {
    fn get(&self) -> &T {
        &self.0
    }
}
impl<T: Message> Clone for Obj<T> {
    fn clone(&self) -> Self {
        Obj(self.0.clone())
    }
}

#[derive(Debug, Clone)]
enum Ev {
    Discovered(String),
    Connected(String),
    ConnectFailed(String, String),
    Disconnected(String),
    Services(String, Option<String>),
    Chars(String, Option<String>),
    Notify(String, String, Option<String>),
}

struct Route {
    incoming: mpsc::UnboundedSender<Incoming>,
    /// the write with response in flight: its characteristic and who waits for the result
    write_waiter: Option<(String, oneshot::Sender<Result<(), BleError>>)>,
    /// `peripheralIsReadyToSendWriteWithoutResponse:` (room in the queue again)
    ready: Arc<Notify>,
}

struct Seen {
    p: Obj<CBPeripheral>,
    name: String,
    rssi: i64,
}

struct Shared {
    events: broadcast::Sender<Ev>,
    routes: Mutex<HashMap<String, Route>>,
    seen: Mutex<HashMap<String, Seen>>,
    power: watch::Sender<isize>,
}

fn peer_id(p: &CBPeripheral) -> String {
    unsafe { p.identifier() }.UUIDString().to_string().to_uppercase()
}

fn uuid_of(u: Retained<CBUUID>) -> String {
    unsafe { u.UUIDString() }.to_string().to_lowercase()
}

fn cbuuid(s: &str) -> Retained<CBUUID> {
    unsafe { CBUUID::UUIDWithString(&NSString::from_str(s)) }
}

fn err_text(e: Option<&NSError>) -> Option<String> {
    e.map(|e| e.localizedDescription().to_string())
}

/// ATT errors (CBATTErrorDomain) keep their code: 0x11 is "Insufficient Resources".
fn ble_error(e: &NSError) -> BleError {
    let domain = e.domain().to_string();
    let text = e.localizedDescription().to_string();
    if domain == "CBATTErrorDomain" {
        BleError::with_code(e.code() as i64, text)
    } else {
        BleError::new(format!("{text} ({domain} {})", e.code()))
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; Delegate doesn't implement Drop.
    #[unsafe(super(NSObject))]
    #[name = "StickyBotLinkDelegate"]
    #[ivars = Arc<Shared>]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl CBCentralManagerDelegate for Delegate {
        #[unsafe(method(centralManagerDidUpdateState:))]
        fn did_update_state(&self, central: &CBCentralManager) {
            let s = unsafe { central.state() };
            self.ivars().power.send_replace(s.0);
        }

        #[unsafe(method(centralManager:didDiscoverPeripheral:advertisementData:RSSI:))]
        fn did_discover(
            &self,
            _central: &CBCentralManager,
            p: &CBPeripheral,
            adv: &NSDictionary<NSString, AnyObject>,
            rssi: &NSNumber,
        ) {
            let id = peer_id(p);
            let local = unsafe { adv.objectForKey(CBAdvertisementDataLocalNameKey) }
                .and_then(|o| o.downcast::<NSString>().ok())
                .map(|s| s.to_string());
            let name = local.or_else(|| unsafe { p.name() }.map(|n| n.to_string())).unwrap_or_else(|| "?".into());
            let seen = Seen { p: Obj(p.retain()), name, rssi: rssi.integerValue() as i64 };
            self.ivars().seen.lock().unwrap().insert(id.clone(), seen);
            let _ = self.ivars().events.send(Ev::Discovered(id));
        }

        #[unsafe(method(centralManager:didConnectPeripheral:))]
        fn did_connect(&self, _central: &CBCentralManager, p: &CBPeripheral) {
            let _ = self.ivars().events.send(Ev::Connected(peer_id(p)));
        }

        #[unsafe(method(centralManager:didFailToConnectPeripheral:error:))]
        fn did_fail(&self, _central: &CBCentralManager, p: &CBPeripheral, error: Option<&NSError>) {
            let why = err_text(error).unwrap_or_else(|| "connection failed".into());
            let _ = self.ivars().events.send(Ev::ConnectFailed(peer_id(p), why));
        }

        #[unsafe(method(centralManager:didDisconnectPeripheral:error:))]
        fn did_disconnect(&self, _central: &CBCentralManager, p: &CBPeripheral, _error: Option<&NSError>) {
            let id = peer_id(p);
            // dropping the route closes the connection's channel: the link sees the drop
            if let Some(mut r) = self.ivars().routes.lock().unwrap().remove(&id) {
                if let Some((_, w)) = r.write_waiter.take() {
                    let _ = w.send(Err(BleError::new("disconnected")));
                }
                r.ready.notify_waiters();
            }
            let _ = self.ivars().events.send(Ev::Disconnected(id));
        }
    }

    unsafe impl CBPeripheralDelegate for Delegate {
        #[unsafe(method(peripheral:didDiscoverServices:))]
        fn did_services(&self, p: &CBPeripheral, error: Option<&NSError>) {
            let _ = self.ivars().events.send(Ev::Services(peer_id(p), err_text(error)));
        }

        #[unsafe(method(peripheral:didDiscoverCharacteristicsForService:error:))]
        fn did_chars(&self, p: &CBPeripheral, _service: &CBService, error: Option<&NSError>) {
            let _ = self.ivars().events.send(Ev::Chars(peer_id(p), err_text(error)));
        }

        #[unsafe(method(peripheral:didUpdateValueForCharacteristic:error:))]
        fn did_value(&self, p: &CBPeripheral, c: &CBCharacteristic, error: Option<&NSError>) {
            if error.is_some() {
                return;
            }
            let Some(data) = (unsafe { c.value() }) else { return };
            let uuid = uuid_of(unsafe { c.UUID() });
            let msg = match uuid.as_str() {
                TX => Incoming::Tx(data.to_vec()),
                AUDIO => Incoming::Audio(data.to_vec()),
                _ => return,
            };
            if let Some(r) = self.ivars().routes.lock().unwrap().get(&peer_id(p)) {
                let _ = r.incoming.send(msg);
            }
        }

        #[unsafe(method(peripheral:didWriteValueForCharacteristic:error:))]
        fn did_write(&self, p: &CBPeripheral, c: &CBCharacteristic, error: Option<&NSError>) {
            let uuid = uuid_of(unsafe { c.UUID() });
            let res = match error {
                None => Ok(()),
                Some(e) => Err(ble_error(e)),
            };
            if let Some(r) = self.ivars().routes.lock().unwrap().get_mut(&peer_id(p))
                && r.write_waiter.as_ref().is_some_and(|(u, _)| *u == uuid)
                && let Some((_, w)) = r.write_waiter.take()
            {
                let _ = w.send(res);
            }
        }

        #[unsafe(method(peripheralIsReadyToSendWriteWithoutResponse:))]
        fn ready_to_send(&self, p: &CBPeripheral) {
            if let Some(r) = self.ivars().routes.lock().unwrap().get(&peer_id(p)) {
                r.ready.notify_waiters();
            }
        }

        #[unsafe(method(peripheral:didUpdateNotificationStateForCharacteristic:error:))]
        fn did_notify(&self, p: &CBPeripheral, c: &CBCharacteristic, error: Option<&NSError>) {
            let uuid = uuid_of(unsafe { c.UUID() });
            let _ = self.ivars().events.send(Ev::Notify(peer_id(p), uuid, err_text(error)));
        }
    }
);

impl Delegate {
    fn new(shared: Arc<Shared>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(shared);
        unsafe { msg_send![super(this), init] }
    }
}

struct Manager {
    central: Obj<CBCentralManager>,
    delegate: Obj<Delegate>, // the manager only holds it weakly
}

struct Inner {
    shared: Arc<Shared>,
    queue: DispatchRetained<DispatchQueue>,
    manager: OnceCell<Manager>,
    scan_lock: tokio::sync::Mutex<()>,
}

/// The CoreBluetooth [`Connector`].
#[derive(Clone)]
pub struct CoreBluetooth {
    inner: Arc<Inner>,
}

impl Default for CoreBluetooth {
    fn default() -> Self {
        Self::new()
    }
}

impl CoreBluetooth {
    /// Doesn't touch Bluetooth yet (no permission prompt until it's used).
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            events: broadcast::channel(256).0,
            routes: Mutex::new(HashMap::new()),
            seen: Mutex::new(HashMap::new()),
            power: watch::channel(0).0,
        });
        Self {
            inner: Arc::new(Inner {
                shared,
                queue: DispatchQueue::new("com.stickybot.rs.bluetooth", None),
                manager: OnceCell::new(),
                scan_lock: tokio::sync::Mutex::new(()),
            }),
        }
    }

    fn on_queue<R: Send>(&self, f: impl FnOnce() -> R + Send) -> R {
        let mut out = None;
        self.inner.queue.exec_sync(|| out = Some(f()));
        out.expect("ran on the Bluetooth queue")
    }

    /// The central manager, once Bluetooth is on and allowed.
    async fn manager(&self) -> Result<&Manager, BleError> {
        let m = self
            .inner
            .manager
            .get_or_init(|| async {
                let delegate = Obj(Delegate::new(self.inner.shared.clone()));
                let queue = &*self.inner.queue;
                let central = unsafe {
                    CBCentralManager::initWithDelegate_queue(
                        CBCentralManager::alloc(),
                        Some(ProtocolObject::from_ref(&*delegate.0)),
                        Some(queue),
                    )
                };
                Manager { central: Obj(central), delegate }
            })
            .await;
        let _ = &m.delegate;
        let mut power = self.inner.shared.power.subscribe();
        let ready = tokio::time::timeout(
            POWER_TIMEOUT,
            power.wait_for(|s| *s != CBManagerState::Unknown.0 && *s != CBManagerState::Resetting.0),
        )
        .await;
        let state = match ready {
            Ok(Ok(s)) => *s,
            _ => return Err(BleError::new("Bluetooth didn't start")),
        };
        match CBManagerState(state) {
            CBManagerState::PoweredOn => Ok(m),
            CBManagerState::PoweredOff => Err(BleError::new("Bluetooth is off")),
            CBManagerState::Unauthorized => Err(BleError::new(
                "this app isn't allowed to use Bluetooth (System Settings → Privacy & Security → Bluetooth)",
            )),
            CBManagerState::Unsupported => Err(BleError::new("this Mac has no Bluetooth LE")),
            _ => Err(BleError::new(format!("Bluetooth isn't ready (state {state})"))),
        }
    }

    /// A known peripheral by its CoreBluetooth id (no scan needed for one this Mac has seen).
    fn retrieve(&self, m: &Manager, id: &str) -> Option<Obj<CBPeripheral>> {
        let id = id.to_string();
        let central = m.central.clone();
        self.on_queue(move || {
            let uuid = NSUUID::initWithUUIDString(NSUUID::alloc(), &NSString::from_str(&id))?;
            let list =
                unsafe { central.get().retrievePeripheralsWithIdentifiers(&NSArray::from_retained_slice(&[uuid])) };
            list.iter().next().map(Obj)
        })
    }

    async fn scan_ids(&self, m: &Manager, timeout: Duration, stop_at: Option<&str>) -> HashSet<String> {
        let _one = self.inner.scan_lock.lock().await;
        let mut ev = self.inner.shared.events.subscribe();
        let central = m.central.clone();
        self.on_queue(move || unsafe {
            central
                .get()
                .scanForPeripheralsWithServices_options(Some(&NSArray::from_retained_slice(&[cbuuid(NUS)])), None)
        });
        let mut ids = HashSet::new();
        let _ = tokio::time::timeout(timeout, async {
            loop {
                match ev.recv().await {
                    Ok(Ev::Discovered(id)) => {
                        let done = stop_at == Some(id.as_str());
                        ids.insert(id);
                        if done {
                            return;
                        }
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        })
        .await;
        let central = m.central.clone();
        self.on_queue(move || unsafe { central.get().stopScan() });
        ids
    }

    fn cancel(&self, m: &Manager, p: &Obj<CBPeripheral>, id: &str) {
        self.inner.shared.routes.lock().unwrap().remove(id);
        let (central, p) = (m.central.clone(), p.clone());
        self.on_queue(move || unsafe { central.get().cancelPeripheralConnection(p.get()) });
    }

    async fn setup(
        &self,
        m: &Manager,
        p: &Obj<CBPeripheral>,
        id: &str,
        ev: &mut broadcast::Receiver<Ev>,
    ) -> Result<(HashMap<String, Obj<CBCharacteristic>>, usize, usize), BleError> {
        let (central, pp, delegate) = (m.central.clone(), p.clone(), m.delegate.clone());
        self.on_queue(move || unsafe {
            pp.get().setDelegate(Some(ProtocolObject::from_ref(delegate.get())));
            central.get().connectPeripheral_options(pp.get(), None);
        });
        wait(ev, CONNECT_TIMEOUT, "connecting", |e| match e {
            Ev::Connected(i) if i == id => Some(Ok(())),
            Ev::ConnectFailed(i, why) if i == id => Some(Err(BleError::new(why.clone()))),
            _ => None,
        })
        .await
        .map_err(|e| if e.message.starts_with("timed out") { BleError::not_found(e.message) } else { e })?;

        let pp = p.clone();
        self.on_queue(move || unsafe {
            pp.get().discoverServices(Some(&NSArray::from_retained_slice(&[cbuuid(NUS)])))
        });
        wait(ev, STEP_TIMEOUT, "finding the service", |e| step(e, id, |x| matches!(x, Ev::Services(..)))).await?;
        let pp = p.clone();
        let service = self
            .on_queue(move || {
                unsafe { pp.get().services() }
                    .and_then(|list| list.iter().find(|s| uuid_of(unsafe { s.UUID() }) == NUS).map(Obj))
            })
            .ok_or_else(|| BleError::new("the stick has no Mac link service (firmware too old?)"))?;

        let (pp, sv) = (p.clone(), service.clone());
        self.on_queue(move || unsafe { pp.get().discoverCharacteristics_forService(None, sv.get()) });
        wait(ev, STEP_TIMEOUT, "finding the characteristics", |e| step(e, id, |x| matches!(x, Ev::Chars(..)))).await?;
        let sv = service.clone();
        let chars: HashMap<String, Obj<CBCharacteristic>> = self.on_queue(move || {
            unsafe { sv.get().characteristics() }
                .map(|list| list.iter().map(|c| (uuid_of(unsafe { c.UUID() }), Obj(c))).collect())
                .unwrap_or_default()
        });
        if !chars.contains_key(RX) {
            return Err(BleError::new("the stick has no RX characteristic"));
        }
        for uuid in [TX, AUDIO] {
            let Some(c) = chars.get(uuid).cloned() else {
                if uuid == TX {
                    return Err(BleError::new("the stick has no TX characteristic"));
                }
                continue;
            };
            let pp = p.clone();
            self.on_queue(move || unsafe { pp.get().setNotifyValue_forCharacteristic(true, c.get()) });
            wait(ev, STEP_TIMEOUT, "turning on notifications", |e| match e {
                Ev::Notify(i, u, err) if i == id && u == uuid => {
                    Some(err.clone().map_or(Ok(()), |m| Err(BleError::new(format!("notifications: {m}")))))
                }
                Ev::Disconnected(i) if i == id => Some(Err(BleError::new("disconnected"))),
                _ => None,
            })
            .await?;
        }
        let pp = p.clone();
        let max = self.on_queue(move || unsafe {
            pp.get().maximumWriteValueLengthForType(CBCharacteristicWriteType::WithoutResponse)
        });
        let writable: HashMap<String, Obj<CBCharacteristic>> =
            chars.into_iter().filter(|(u, _)| [RX, AUDIO_IN, OTA_DATA].contains(&u.as_str())).collect();
        Ok((writable, max + 3, max))
    }
}

fn step(e: &Ev, id: &str, is: impl Fn(&Ev) -> bool) -> Option<Result<(), BleError>> {
    match e {
        Ev::Disconnected(i) if i == id => Some(Err(BleError::new("disconnected"))),
        Ev::Services(i, err) | Ev::Chars(i, err) if i == id && is(e) => {
            Some(err.clone().map_or(Ok(()), |m| Err(BleError::new(m))))
        }
        _ => None,
    }
}

async fn wait<T>(
    ev: &mut broadcast::Receiver<Ev>,
    timeout: Duration,
    what: &str,
    mut f: impl FnMut(&Ev) -> Option<Result<T, BleError>>,
) -> Result<T, BleError> {
    let res = tokio::time::timeout(timeout, async {
        loop {
            match ev.recv().await {
                Ok(e) => {
                    if let Some(r) = f(&e) {
                        return r;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return Err(BleError::new("Bluetooth went away")),
            }
        }
    })
    .await;
    res.unwrap_or_else(|_| Err(BleError::new(format!("timed out {what}"))))
}

/// One connection made by [`CoreBluetooth`].
pub struct CbConn {
    bt: CoreBluetooth,
    id: String,
    p: Obj<CBPeripheral>,
    /// RX, and AUDIO_IN / OTA_DATA when the firmware has them
    chars: HashMap<String, Obj<CBCharacteristic>>,
    mtu: usize,
    max_without_response: usize,
}

impl CbConn {
    fn char(&self, ch: Char) -> Result<Obj<CBCharacteristic>, BleError> {
        self.chars
            .get(ch.uuid())
            .cloned()
            .ok_or_else(|| BleError::new(format!("the stick has no {ch:?} characteristic")))
    }

    async fn write_with_response(&self, c: Obj<CBCharacteristic>, data: Vec<u8>) -> Result<(), BleError> {
        let (tx, done) = oneshot::channel();
        let uuid = {
            let c = c.clone();
            self.bt.on_queue(move || uuid_of(unsafe { c.get().UUID() }))
        };
        {
            let mut routes = self.bt.inner.shared.routes.lock().unwrap();
            let r = routes.get_mut(&self.id).ok_or_else(|| BleError::new("disconnected"))?;
            r.write_waiter = Some((uuid, tx));
        }
        let p = self.p.clone();
        self.bt.on_queue(move || unsafe {
            p.get().writeValue_forCharacteristic_type(
                &NSData::with_bytes(&data),
                c.get(),
                CBCharacteristicWriteType::WithResponse,
            )
        });
        match tokio::time::timeout(WRITE_TIMEOUT, done).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err(BleError::new("disconnected")),
            Err(_) => Err(BleError::new("the write timed out")),
        }
    }

    /// Waits for `canSendWriteWithoutResponse` (woken by `peripheralIsReadyToSendWriteWithoutResponse:`
    /// *or* every [`pacing::POLL`]: the callback alone came late and made clips slower than
    /// link.py, see `pacing.rs`), then queues the packet. Checking and writing happen in one block
    /// on the Bluetooth queue, so the room can't go away in between.
    async fn write_without_response(&self, c: Obj<CBCharacteristic>, data: Vec<u8>) -> Result<(), BleError> {
        let ready = {
            let routes = self.bt.inner.shared.routes.lock().unwrap();
            routes.get(&self.id).map(|r| r.ready.clone()).ok_or_else(|| BleError::new("disconnected"))?
        };
        let data = Obj(NSData::with_bytes(&data));
        pacing::wait_and_send(&ready, Some(pacing::POLL), pacing::ROOM_TIMEOUT, |late| {
            let (p, c2, d) = (self.p.clone(), c.clone(), data.clone());
            let wrote = self.bt.on_queue(move || unsafe {
                if late || p.get().canSendWriteWithoutResponse() {
                    p.get().writeValue_forCharacteristic_type(d.get(), c2.get(), CBCharacteristicWriteType::WithoutResponse);
                    true
                } else {
                    false
                }
            });
            if !wrote && !self.bt.inner.shared.routes.lock().unwrap().contains_key(&self.id) {
                return Err(BleError::new("disconnected"));
            }
            Ok(wrote)
        })
        .await
        .map(|_| ())
    }
}

impl Connection for CbConn {
    async fn write_rx(&self, chunk: Vec<u8>) -> Result<(), BleError> {
        self.write(Char::Rx, chunk, true).await
    }

    async fn write(&self, ch: Char, data: Vec<u8>, response: bool) -> Result<(), BleError> {
        let c = self.char(ch)?;
        if response { self.write_with_response(c, data).await } else { self.write_without_response(c, data).await }
    }

    fn has(&self, ch: Char) -> bool {
        self.chars.contains_key(ch.uuid())
    }

    fn max_write_without_response(&self) -> usize {
        self.max_without_response
    }

    fn mtu(&self) -> usize {
        self.mtu
    }

    async fn disconnect(&self) {
        if let Some(m) = self.bt.inner.manager.get() {
            self.bt.cancel(m, &self.p, &self.id);
        }
    }
}

impl Connector for CoreBluetooth {
    type Conn = CbConn;

    async fn connect(&self, target: &Target) -> Result<(CbConn, mpsc::UnboundedReceiver<Incoming>), BleError> {
        let m = self.manager().await?;
        let id = target.address.trim().to_uppercase();
        let p = match self.retrieve(m, &id) {
            Some(p) => p,
            None => {
                self.scan_ids(m, Duration::from_secs(10), Some(&id)).await;
                let seen = self.inner.shared.seen.lock().unwrap();
                seen.get(&id)
                    .map(|s| s.p.clone())
                    .ok_or_else(|| BleError::not_found(format!("{} not found", target.name)))?
            }
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let route = Route { incoming: tx, write_waiter: None, ready: Arc::new(Notify::new()) };
        self.inner.shared.routes.lock().unwrap().insert(id.clone(), route);
        let mut ev = self.inner.shared.events.subscribe();
        // if this future is dropped half way (re-picked, paused), don't leave a connection behind
        let mut guard = CancelOnDrop { bt: self, p: Some(p.clone()), id: &id };
        match self.setup(m, &p, &id, &mut ev).await {
            Ok((chars, mtu, max_without_response)) => {
                guard.p = None;
                Ok((CbConn { bt: self.clone(), id: id.clone(), p, chars, mtu, max_without_response }, rx))
            }
            Err(e) => Err(e), // the guard cancels
        }
    }

    async fn scan(&self, timeout: Duration) -> Result<Vec<Found>, BleError> {
        let m = self.manager().await?;
        let ids = self.scan_ids(m, timeout, None).await;
        let seen = self.inner.shared.seen.lock().unwrap();
        let mut out: Vec<Found> = ids
            .iter()
            .filter_map(|id| seen.get(id).map(|s| Found { address: id.clone(), name: s.name.clone(), rssi: s.rssi }))
            .collect();
        out.sort_by_key(|f| -f.rssi);
        Ok(out)
    }
}

struct CancelOnDrop<'a> {
    bt: &'a CoreBluetooth,
    p: Option<Obj<CBPeripheral>>,
    id: &'a str,
}

impl Drop for CancelOnDrop<'_> {
    fn drop(&mut self) {
        if let (Some(p), Some(m)) = (self.p.take(), self.bt.inner.manager.get()) {
            self.bt.cancel(m, &p, self.id);
        }
    }
}
