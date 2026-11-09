//! Lists sticks advertising the StickyBot service. Read-only: it never connects, so it's safe while
//! the Python agent holds the stick. The first run asks macOS for the Bluetooth permission (for the
//! terminal app that runs it).
//!
//!     cargo run -p stick-link --example scan [seconds]

use std::time::Duration;
use stick_link::{Backend, Connector};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    match Backend::system().scan(Duration::from_secs(secs)).await {
        Ok(found) if found.is_empty() => println!("no sticks seen in {secs} s"),
        Ok(found) => {
            for f in found {
                println!("{:>4} dBm  {}  {}", f.rssi, f.address, f.name);
            }
        }
        Err(e) => {
            eprintln!("scan failed: {e}");
            std::process::exit(1);
        }
    }
}
