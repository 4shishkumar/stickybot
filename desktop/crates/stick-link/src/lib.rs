//! The Bluetooth link to the StickyBot stick (PROTOCOL.md section 5, a port of
//! `mac-agent/stick_agent/link.py`).
//!
//! - [`proto`]: the wire format, no I/O (framing, chunking, line decoding, message shapes).
//! - [`Link`]: the reconnect loop, hellos, `proto` requests with ids, the message stream.
//! - [`transport`]: what the link needs from Bluetooth ([`Connector`] / [`Connection`]).
//! - [`corebt`]: CoreBluetooth through objc2 (macOS).
//! - [`fake`]: a stick in memory, for tests and `--fake-stick`.

pub mod fake;
mod link;
pub mod proto;
pub mod transport;

#[cfg(target_os = "macos")]
pub mod corebt;

pub use link::{Link, LinkState, Options};
pub use transport::{BleError, Connection, Connector, Found, Incoming, Target};

use std::time::Duration;
use tokio::sync::mpsc;

/// Either real Bluetooth or the fake stick, picked at start-up.
#[derive(Clone)]
pub enum Backend {
    #[cfg(target_os = "macos")]
    CoreBluetooth(corebt::CoreBluetooth),
    Fake(fake::FakeStick),
}

impl Backend {
    /// The real one on macOS (the fake elsewhere: there is no other backend yet).
    pub fn system() -> Self {
        #[cfg(target_os = "macos")]
        return Backend::CoreBluetooth(corebt::CoreBluetooth::new());
        #[cfg(not(target_os = "macos"))]
        return Backend::Fake(fake::FakeStick::new());
    }

    pub fn is_fake(&self) -> bool {
        matches!(self, Backend::Fake(_))
    }
}

pub enum BackendConn {
    #[cfg(target_os = "macos")]
    CoreBluetooth(corebt::CbConn),
    Fake(fake::FakeConn),
}

impl Connection for BackendConn {
    async fn write_rx(&self, chunk: Vec<u8>) -> Result<(), BleError> {
        match self {
            #[cfg(target_os = "macos")]
            BackendConn::CoreBluetooth(c) => c.write_rx(chunk).await,
            BackendConn::Fake(c) => c.write_rx(chunk).await,
        }
    }
    fn mtu(&self) -> usize {
        match self {
            #[cfg(target_os = "macos")]
            BackendConn::CoreBluetooth(c) => c.mtu(),
            BackendConn::Fake(c) => c.mtu(),
        }
    }
    async fn disconnect(&self) {
        match self {
            #[cfg(target_os = "macos")]
            BackendConn::CoreBluetooth(c) => c.disconnect().await,
            BackendConn::Fake(c) => c.disconnect().await,
        }
    }
}

impl Connector for Backend {
    type Conn = BackendConn;

    async fn connect(&self, target: &Target) -> Result<(BackendConn, mpsc::UnboundedReceiver<Incoming>), BleError> {
        match self {
            #[cfg(target_os = "macos")]
            Backend::CoreBluetooth(b) => b.connect(target).await.map(|(c, rx)| (BackendConn::CoreBluetooth(c), rx)),
            Backend::Fake(b) => b.connect(target).await.map(|(c, rx)| (BackendConn::Fake(c), rx)),
        }
    }

    async fn scan(&self, timeout: Duration) -> Result<Vec<Found>, BleError> {
        match self {
            #[cfg(target_os = "macos")]
            Backend::CoreBluetooth(b) => b.scan(timeout).await,
            Backend::Fake(b) => b.scan(timeout).await,
        }
    }
}
