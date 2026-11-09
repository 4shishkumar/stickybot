//! The dashboard + link without the Tauri shell (no window, no menu bar icon): for development,
//! tests and screenshots.
//!
//!     cargo run -p stickybot-server --bin stickybot-headless -- [--fake-stick] [--port 8670]

use stickybot_server::{Options, start};

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut opts = Options::from_env();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--fake-stick" => opts.fake_stick = true,
            "--port" => opts.port = args.next().and_then(|p| p.parse().ok()).expect("--port <number>"),
            "-h" | "--help" => {
                println!(
                    "stickybot-headless [--fake-stick] [--port N]   (home: $STICKYBOT_RS_HOME or ~/.stickybot-rs)"
                );
                return;
            }
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    match start(opts).await {
        Ok(r) => println!("StickyBot (Rust) dashboard: {}", r.state.url()),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
