//! What the link needs from Bluetooth, so it can run on CoreBluetooth or on a fake in tests.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::future::Future;
use std::time::Duration;
use tokio::sync::mpsc;

/// The stick the user picked (config `"stick"`): the CoreBluetooth identifier and its name.
/// The same shape as the Python agent's config, so a value can be copied over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub address: String,
    #[serde(default)]
    pub name: String,
}

/// A stick seen in a scan (link.py `scan()`): strongest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Found {
    pub address: String,
    pub name: String,
    pub rssi: i64,
}

/// What arrives from the stick on one connection. The channel closing means it disconnected.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// a TX notification: a piece of JSON lines
    Tx(Vec<u8>),
    /// an AUDIO notification: one ADPCM frame
    Audio(Vec<u8>),
}

/// A Bluetooth error: CoreBluetooth's code when there is one (ATT errors are 0x01..0x11) and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleError {
    pub code: Option<i64>,
    pub message: String,
    /// the stick wasn't seen (scan timed out): try again later, quietly
    pub not_found: bool,
}

impl BleError {
    pub fn new(message: impl Into<String>) -> Self {
        Self { code: None, message: message.into(), not_found: false }
    }
    pub fn with_code(code: i64, message: impl Into<String>) -> Self {
        Self { code: Some(code), message: message.into(), not_found: false }
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self { code: None, message: message.into(), not_found: true }
    }
    /// The stick's Bluetooth buffers are busy for a moment: the write is worth repeating.
    pub fn is_insufficient_resource(&self) -> bool {
        crate::proto::is_insufficient_resource(self.code, &self.message)
    }
}

impl fmt::Display for BleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.code {
            Some(c) => write!(f, "{} (code {c})", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for BleError {}

/// The characteristics the Mac writes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Char {
    /// JSON lines
    Rx,
    /// sound clips (PROTOCOL.md "Audio in")
    AudioIn,
    /// firmware image packets (stick 1.8.0+)
    OtaData,
}

impl Char {
    pub fn uuid(self) -> &'static str {
        match self {
            Char::Rx => crate::proto::RX,
            Char::AudioIn => crate::proto::AUDIO_IN,
            Char::OtaData => crate::proto::OTA_DATA,
        }
    }
}

/// One live connection to the stick, with notifications on TX and AUDIO already turned on.
pub trait Connection: Send + Sync + 'static {
    /// One write with response on RX (at most [`crate::proto::CHUNK`] bytes).
    fn write_rx(&self, chunk: Vec<u8>) -> impl Future<Output = Result<(), BleError>> + Send;
    /// One write to `ch`: with response (waits for the stick's answer), or without (waits until
    /// CoreBluetooth has room in its queue — `canSendWriteWithoutResponse` — then queues it;
    /// a packet can still be lost under pressure, the protocols on top cope with that).
    fn write(&self, ch: Char, data: Vec<u8>, response: bool) -> impl Future<Output = Result<(), BleError>> + Send;
    /// The stick has this characteristic (AUDIO_IN / OTA_DATA depend on the firmware).
    fn has(&self, ch: Char) -> bool;
    /// The largest write without response (`maximumWriteValueLengthForType`), 0 if unknown.
    fn max_write_without_response(&self) -> usize;
    /// The ATT MTU (0 if unknown).
    fn mtu(&self) -> usize;
    /// Ends this app's connection (the Mac's keyboard connection to the stick stays).
    fn disconnect(&self) -> impl Future<Output = ()> + Send;
}

/// Finds and connects to sticks.
pub trait Connector: Send + Sync + 'static {
    type Conn: Connection;
    /// Connects to `target` and subscribes to TX and AUDIO. Notifications arrive on the receiver,
    /// which closes when the stick disconnects.
    fn connect(
        &self,
        target: &Target,
    ) -> impl Future<Output = Result<(Self::Conn, mpsc::UnboundedReceiver<Incoming>), BleError>> + Send;
    /// Sticks advertising the service, strongest first. Never connects.
    fn scan(&self, timeout: Duration) -> impl Future<Output = Result<Vec<Found>, BleError>> + Send;
}
