//! Hold B, end to end, with the real speech model: macOS `say` records a sentence, it's encoded
//! with the stick's codec (IMA-ADPCM, 20 ms frames), the fake stick streams it like the firmware
//! (`talk start`, frames, `talk stop`), and the stick's screen must show the right answer.
//!
//! Needs the Parakeet model: `$STICKYBOT_RS_TEST_MODEL` (a model folder), else
//! `~/.stickybot-rs/models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8`. Without one the test says
//! so and passes (CI has no model). Run it with `cargo test -p stickybot-server --test talk_e2e --release -- --nocapture`.

use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use stick_link::fake::FakeStick;
use stick_link::{Backend, LinkState, adpcm};
use stickybot_server::commands::Action;
use stickybot_server::{AppState, Options, speech, talk};

fn model_dir() -> Option<PathBuf> {
    let dir = match std::env::var_os("STICKYBOT_RS_TEST_MODEL") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".stickybot-rs/models").join(speech::MODEL_NAME),
    };
    speech::model_ready(&dir).then_some(dir)
}

/// `say` -> 16 kHz mono 16-bit WAV -> samples.
fn say(text: &str, voice: &str, dir: &Path) -> Vec<i16> {
    let wav = dir.join(format!("{}.wav", text.len()));
    let ok = std::process::Command::new("say")
        .args(["-v", voice, "-o"])
        .arg(&wav)
        .args(["--data-format=LEI16@16000", text])
        .status()
        .expect("macOS say");
    assert!(ok.success());
    let mut r = hound::WavReader::open(&wav).unwrap();
    assert_eq!((r.spec().sample_rate, r.spec().channels), (16000, 1));
    r.samples::<i16>().map(Result::unwrap).collect()
}

/// Opt-in on a Mac with the app's verified Goose, llama.cpp, GGUF and Parakeet already installed.
/// Uses synthetic speech and an isolated server with no connected device or cloud credentials.
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the installed local assistant, speech model and Metal"]
async fn mac_keyboard_with_real_local_agent_offline() {
    use stickybot_server::local::{llama::tempdir::Dir, runtime};
    let home = Dir::new("stickybot-keyboard-test").unwrap();
    let installed = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".stickybot-rs/assistant");
    let assets = home.path().join("assistant");
    std::fs::create_dir(&assets).unwrap();
    for item in runtime::ALL {
        let marker = format!(".{}.ok", item.dir);
        std::fs::copy(installed.join(&marker), assets.join(marker)).expect("install the local assistant first");
        let dir = installed.join(item.dir);
        if dir.is_dir() {
            std::os::unix::fs::symlink(dir, assets.join(item.dir)).unwrap();
        }
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let opts = Options {
        port: listener.local_addr().unwrap().port(),
        home: home.path().into(),
        fake_stick: true,
        import_from: None,
    };
    let st = AppState::new(&opts, Backend::Fake(FakeStick::new())).unwrap();
    st.cfg.lock().unwrap().insert("brain".into(), json!("local"));
    st.set_brain(Arc::new(st.local()));
    let api = stickybot_server::api::router(st.clone());
    let server = tokio::spawn(async move { axum::serve(listener, api).await.unwrap() });
    let model = model_dir().expect("install the speech model first");
    st.speech.start(speech::parakeet(&model, &st.stt_hints()));
    tokio::time::timeout(Duration::from_secs(60), async {
        while !st.speech.ready() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("Parakeet failed to start");

    let phases = Arc::new(Mutex::new(Vec::new()));
    let received = phases.clone();
    let turn = talk::MacTurn::begin(
        st.clone(),
        Arc::new(move |phase, heard, text| {
            received.lock().unwrap().push((phase.to_owned(), heard.to_owned(), text.to_owned()));
        }),
    )
    .unwrap();
    let samples = say("What color is a ripe banana? Answer with just the color.", "Samantha", home.path());
    let audio = samples.into_iter().map(|s| f32::from(s) / 32768.).collect();
    let finished = tokio::time::timeout(Duration::from_secs(180), turn.submit(audio)).await;
    st.brain().forget();
    st.local_model.stop().await;
    server.abort();
    finished.expect("local turn did not finish");
    let phases = phases.lock().unwrap();
    let (phase, heard, answer) = phases.last().expect("no Mac response");
    assert_eq!(phase, "reply", "Mac panel events: {phases:?}");
    assert!(heard.to_lowercase().contains("banana"), "{heard}");
    assert!(answer.to_lowercase().contains("yellow"), "{answer}");
    assert!(!st.link.connected());
}

async fn until_done(fake: &FakeStick) -> Vec<(String, String, String)> {
    for _ in 0..1500 {
        let s = fake.screens();
        if s.last().is_some_and(|(st, _, _)| matches!(st.as_str(), "done" | "error" | "cancelled")) {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    fake.screens()
}

#[tokio::test(flavor = "multi_thread")]
async fn hold_b_with_real_speech() {
    let Some(model) = model_dir() else {
        eprintln!("no speech model: skipped (set STICKYBOT_RS_TEST_MODEL or download it with the app)");
        return;
    };
    let home = std::env::temp_dir().join(format!("stickybot-rs-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let fake = FakeStick::new();
    let st = AppState::new(&Options { port: 8670, home: home.clone(), fake_stick: true, import_from: None }, Backend::Fake(fake.clone()))
        .unwrap();
    let opened = Arc::new(Mutex::new(Vec::new()));
    let o = opened.clone();
    st.set_runner(Arc::new(move |a: &Action| {
        o.lock().unwrap().push(a.clone());
        String::new()
    }));
    st.commands
        .save(
            json!({"commands": [{"name": "GitHub PR", "phrases": ["pr {n}", "open pr {n}", "open pull request {n}"],
                                  "open_url": "https://github.com/acme/app/pull/{n}"}]})
            .as_object()
            .unwrap(),
        )
        .unwrap();
    st.link.spawn();
    talk::spawn(st.clone());
    st.speech.start(speech::parakeet(&model, &st.stt_hints()));
    st.update_config(json!({"stick": {"address": "FAKE", "name": "fake"}}).as_object().unwrap().clone()).unwrap();
    let mut w = st.link.watch_state();
    tokio::time::timeout(Duration::from_secs(5), w.wait_for(|s| *s == LinkState::Connected)).await.unwrap().unwrap();
    for _ in 0..3000 {
        if st.speech.ready() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(st.speech.ready(), "the model didn't load: {:?}", st.speech.phase());

    // 1. a quick command: the PR opens, the stick says so
    let pcm = say("Open pull request twelve thirty four.", "Samantha", &home);
    let frames = adpcm::encode(&pcm, 320);
    fake.talk(&frames);
    let screens = until_done(&fake).await;
    eprintln!("1: {screens:?}");
    let (state, heard, text) = screens.last().unwrap().clone();
    assert_eq!(state, "done");
    assert_eq!(text, "Opened GitHub PR #1234");
    assert!(heard.to_lowercase().contains("pull request"), "{heard}");
    assert_eq!(*opened.lock().unwrap(), vec![Action::Open(vec!["https://github.com/acme/app/pull/1234".into()])]);

    // 2. a request for the brain, with a word hint in it ("Tauri", not "Tory"); a lost frame on the way
    fake.clear_screens();
    let pcm = say("Ask Claude to refactor the Tauri window code.", "Daniel", &home);
    let mut frames = adpcm::encode(&pcm, 320);
    frames.remove(frames.len() / 2);
    fake.talk(&frames);
    let screens = until_done(&fake).await;
    eprintln!("2: {screens:?}");
    assert_eq!(screens[0].0, "thinking");
    let (state, heard, text) = screens.last().unwrap().clone();
    assert_eq!((state.as_str(), text.as_str()), ("error", talk::NO_BRAIN));
    assert!(heard.contains("Tauri") && heard.contains("Claude"), "{heard}");

    // 3. a short press with nothing said
    fake.clear_screens();
    fake.talk(&adpcm::encode(&vec![0i16; 16000], 320));
    assert_eq!(until_done(&fake).await, vec![("cancelled".into(), String::new(), String::new())]);

    let history = std::fs::read_to_string(st.paths.history()).unwrap();
    assert_eq!(history.lines().count(), 2);
    let _ = std::fs::remove_dir_all(&home);
}
