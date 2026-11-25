//! Scores the agent on `desktop/evals/assistant.json` (see `evals.rs`).
//!
//!     cargo run -p stickybot-server --bin stickybot-eval -- [--brain local|claude|openrouter]
//!         [--only id-or-category,...] [--repeat N] [--out report.json] [--keep]

use std::path::PathBuf;
use std::time::Duration;
use stickybot_server::evals;

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).expect("HOME")
}

#[tokio::main]
async fn main() {
    let evals_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../evals");
    let real_home = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".stickybot-rs"));
    let mut opts = evals::Options {
        brain: "local".into(),
        cases: evals_dir.join("assistant.json"),
        fixtures: evals_dir.join("fixtures"),
        workdir: dirs_home().join(format!("StickyBot-eval-{}", std::process::id())).join("fixtures"),
        only: vec![],
        repeat: 1,
        port: 18791,
        timeout: Duration::from_secs(180),
        runtime_from: real_home,
    };
    let (mut out, mut keep) = (None, false);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| panic!("{a} needs a value"));
        match a.as_str() {
            "--brain" => opts.brain = value(),
            "--only" => opts.only = value().split(',').map(str::to_owned).collect(),
            "--repeat" => opts.repeat = value().parse().expect("--repeat N"),
            "--cases" => opts.cases = PathBuf::from(value()),
            "--port" => opts.port = value().parse().expect("--port N"),
            "--out" => out = Some(PathBuf::from(value())),
            "--keep" => keep = true,
            _ => {
                eprintln!("usage: stickybot-eval [--brain local|claude|openrouter] [--only a,b] [--repeat N] [--out file] [--keep]");
                std::process::exit(2);
            }
        }
    }
    if !["local", "claude", "openrouter"].contains(&opts.brain.as_str()) {
        eprintln!("--brain: local, claude or openrouter");
        std::process::exit(2);
    }
    let home = std::env::temp_dir().join(format!("stickybot-eval-{}", std::process::id()));
    let brain = opts.brain.clone();
    let _ = std::fs::create_dir_all(&home);
    stickybot_server::logs::init(&home.join("app.log"));
    let workdir = opts.workdir.clone();
    let result = evals::run(opts, &home).await;
    if !keep {
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(workdir.parent().unwrap_or(&workdir));
    }
    match result {
        Ok(outcomes) => {
            let (text, json) = evals::report(&brain, &outcomes);
            println!("{text}");
            if let Some(path) = out {
                std::fs::write(&path, serde_json::to_vec_pretty(&json).unwrap()).expect("write the report");
                println!("report: {}", path.display());
            }
            if keep {
                println!("home kept: {}", home.display());
            }
        }
        Err(e) => {
            eprintln!("eval: {e}");
            std::process::exit(1);
        }
    }
}
