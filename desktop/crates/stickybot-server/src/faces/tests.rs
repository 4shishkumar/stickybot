//! test_face_install.py and test_face_ci.py: the .sfb check against the reference compiler's
//! output (testdata/faces/*.sfb, made by tools/faces/facepkg.py from faces/examples), the install
//! against the fake stick speaking the `faces` messages (acks, gaps, resume, refusals), `@faces`
//! paging, remove / use, the catalog cache (signature, conditional fetch, offline, damaged
//! downloads), and the API.

use super::*;
use crate::Options;
use axum::body::Body;
use axum::extract::Request;
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use http_body_util::BodyExt;
use stick_link::LinkState;
use stick_link::fake::FakeStick;
use stick_link::ota::CancelHandle;
use tower::ServiceExt;

const CATNAP: &[u8] = include_bytes!("../../testdata/faces/catnap.sfb");
const TEXTCLOCK: &[u8] = include_bytes!("../../testdata/faces/textclock.sfb");
const DIALCLOCK: &[u8] = include_bytes!("../../testdata/faces/dialclock.sfb");
const BASE: &str = "https://pages.example/stickybot/faces/";
const PORT: u16 = 8670;

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn fixture(fid: &str) -> (&'static [u8], Value) {
    let (blob, man) = match fid {
        "catnap" => (CATNAP, include_str!("../../testdata/faces/catnap.manifest.json")),
        "textclock" => (TEXTCLOCK, include_str!("../../testdata/faces/textclock.manifest.json")),
        _ => (DIALCLOCK, include_str!("../../testdata/faces/dialclock.manifest.json")),
    };
    (blob, serde_json::from_str(man).unwrap())
}

/// The same face with its manifest changed (the images after it move with it).
fn with_manifest(blob: &[u8], f: impl FnOnce(&mut Map<String, Value>)) -> Vec<u8> {
    let mlen = u32le(blob, 8);
    let mut man: Map<String, Value> = serde_json::from_slice(&blob[16..16 + mlen]).unwrap();
    f(&mut man);
    let new = serde_json::to_vec(&man).unwrap();
    let rest = &blob[16 + mlen + (4 - mlen % 4) % 4..];
    let mut out = blob[..8].to_vec();
    out.extend_from_slice(&(new.len() as u32).to_le_bytes());
    out.extend_from_slice(&blob[12..16]);
    out.extend_from_slice(&new);
    out.extend(std::iter::repeat_n(0u8, (4 - new.len() % 4) % 4));
    out.extend_from_slice(rest);
    out
}

// --- the .sfb check ---------------------------------------------------------------------------------

#[test]
fn check_sfb_reads_what_the_compiler_writes() {
    for fid in ["textclock", "dialclock", "catnap"] {
        let (blob, man) = fixture(fid);
        assert_eq!(Value::Object(check_sfb(blob).unwrap()), man, "{fid}");
        let info = sfb_info(blob).unwrap();
        assert_eq!((info["id"].as_str(), info["size"].as_u64()), (Some(fid), Some(blob.len() as u64)));
        assert_eq!(info["minFirmware"], "2.4.0");
        assert_eq!(info["sha256"], sha_hex(blob));
    }
    assert_eq!(sfb_info(CATNAP).unwrap()["name"], "Cat Nap");
}

#[test]
fn check_sfb_refuses_broken_files() {
    let blob = CATNAP.to_vec();
    let cat = |a: &[u8], b: &[u8]| [a, b].concat();
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (cat(b"PNG!", &blob[4..]), "doesn't start"),
        (blob[..40].to_vec(), "bad sizes"),
        (cat(&blob, &vec![0u8; MAX_SFB - blob.len() + 1]), "limit"),
        (cat(&cat(&blob[..4], &2u16.to_le_bytes()), &blob[6..]), "container version 2"),
        (cat(&cat(&blob[..6], &9u16.to_le_bytes()), &blob[8..]), "face format 9"),
        (blob[..blob.len() - 600].to_vec(), "image"),
    ];
    for (bad, words) in cases {
        let e = check_sfb(&bad).unwrap_err();
        assert!(e.contains(words), "{words}: {e}");
    }
}

#[test]
fn check_sfb_catches_a_pixel_past_the_palette() {
    // one 2x1 image, 1 colour at 2 bits per pixel, pixel value 3
    let man = br#"{"id": "tiny", "version": "1.0.0", "format": 1}"#;
    let mut blob = b"SFB1".to_vec();
    blob.extend_from_slice(&1u16.to_le_bytes());
    blob.extend_from_slice(&1u16.to_le_bytes());
    blob.extend_from_slice(&(man.len() as u32).to_le_bytes());
    blob.extend_from_slice(&1u16.to_le_bytes());
    blob.extend_from_slice(&0u16.to_le_bytes());
    blob.extend_from_slice(man);
    blob.extend(std::iter::repeat_n(0u8, (4 - man.len() % 4) % 4));
    // w, h, frame_w, bits, nc-1, clear, enc, reserved, dlen
    blob.extend_from_slice(&[2, 0, 1, 0, 0, 0, 2, 0, 255, 0, 0, 0, 1, 0, 0, 0]);
    blob.extend_from_slice(&[0, 0, 0, 0]); // the palette (1 colour) + padding
    blob.extend_from_slice(&[0b1100_0000, 0, 0, 0]);
    assert!(check_sfb(&blob).unwrap_err().contains("past its palette"));
}

#[test]
fn reasons_are_in_words() {
    assert!(reason(&json!({"error": "full"})).contains("12"));
    assert!(reason(&json!({"error": "busy", "why": "dictation"})).contains("dictation"));
    assert!(reason(&json!({"error": "busy"})).contains("dictation"));
    assert!(reason(&json!({"error": "invalid", "why": "image 2: bad header"})).contains("image 2: bad header"));
    assert!(reason(&json!({"error": "weird"})).contains("weird"));
}

#[test]
fn support_and_fit() {
    let old = stick_support(&json!({"fw": "2.3.0"}));
    assert_eq!(old["supported"], false);
    assert!(why_not(&json!({"minFirmware": "2.4.0", "format": 1}), Some(&old)).unwrap().contains("2.4.0"));
    assert!(why_not(&json!({"format": 1}), None).unwrap().contains("2.4.0"));
    let new = stick_support(&json!({"fw": "2.4.0", "pkgFaces": {"n": 2, "fmt": [1]}}));
    assert_eq!(new, json!({"supported": true, "fw": "2.4.0", "fmt": [1], "n": 2}));
    assert_eq!(why_not(&json!({"minFirmware": "2.4.0", "format": 1}), Some(&new)), None);
    assert!(why_not(&json!({"minFirmware": "2.6.0", "format": 1}), Some(&new)).unwrap().contains("2.6.0"));
    assert!(why_not(&json!({"minFirmware": "2.4.0", "format": 2}), Some(&new)).unwrap().contains("format 2"));
}

// --- a stick that takes faces -----------------------------------------------------------------------

fn tmp_home(name: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("stickybot-rs-faces-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    home
}

fn quick() -> Timings {
    Timings {
        ack_wait: Duration::from_millis(50),
        stall: Duration::from_secs(2),
        reconnect: Duration::from_secs(30),
        reply: Duration::from_millis(500),
        end_reply: Duration::from_millis(500),
        gap_debounce: Duration::from_millis(500),
    }
}

/// The app with a connected fake stick on firmware 2.4.0 (the face engine).
async fn app(name: &str) -> (Router, Arc<AppState>, FakeStick) {
    let opts = Options { port: PORT, home: tmp_home(name), fake_stick: true, import_from: None };
    let fake = FakeStick::new();
    fake.set_stick("fw", json!("2.4.0"));
    fake.faces(|f| f.supported = true);
    let st = AppState::new(&opts, Backend::Fake(fake.clone())).unwrap();
    *st.faces.timings.lock().unwrap() = quick();
    *st.faces.reply.lock().unwrap() = Duration::from_millis(500);
    st.link.spawn();
    st.update_config(json!({"stick": {"address": "FAKE", "name": "fake"}}).as_object().unwrap().clone()).unwrap();
    let mut w = st.link.watch_state();
    tokio::time::timeout(Duration::from_secs(5), w.wait_for(|s| *s == LinkState::Connected)).await.unwrap().unwrap();
    (crate::api::router(st.clone()), st, fake)
}

type Events = Arc<Mutex<Vec<Value>>>;

fn collect() -> (Emit, Events) {
    let ev: Events = Arc::default();
    let e = ev.clone();
    (Arc::new(move |v| e.lock().unwrap().push(v)), ev)
}

async fn install_on(st: &AppState, blob: &[u8], entry: Option<&Value>) -> (Result<Value, String>, Events) {
    let (emit, ev) = collect();
    let id = sfb_info(blob).map(|i| i["id"].as_str().unwrap().to_string()).unwrap_or_default();
    let mut s = sender(&st.link, emit.clone(), &id, quick());
    (install(&st.link, blob.to_vec(), entry, emit, &mut s).await, ev)
}

#[tokio::test]
async fn install_sends_the_face_and_gets_done() {
    let (_, st, fake) = app("install").await;
    let (r, ev) = install_on(&st, CATNAP, None).await;
    let r = r.unwrap();
    assert!(fake.ota(|o| o.committed && o.session.as_ref().is_some_and(|(_, _, d)| d == CATNAP)));
    assert_eq!(
        fake.faces(|f| f.puts[0].clone()),
        json!({"id": "catnap", "version": "1.0.0", "size": CATNAP.len(), "sha256": sha_hex(CATNAP)})
    );
    assert_eq!(fake.faces(|f| f.ends), 1);
    assert_eq!(fake.faces(|f| f.installed["catnap"].0.clone()), "1.0.0");
    assert_eq!(
        (r["id"].as_str(), r["version"].as_str(), r["name"].as_str()),
        (Some("catnap"), Some("1.0.0"), Some("Cat Nap"))
    );
    assert_eq!(r["bytes"], CATNAP.len());
    let ev = ev.lock().unwrap();
    let progress: Vec<i64> =
        ev.iter().filter(|e| e["stage"] == "send").map(|e| e["progress"].as_i64().unwrap()).collect();
    assert_eq!((progress[0], *progress.last().unwrap()), (0, 100));
    assert!(ev.iter().any(|e| e["line"].as_str().is_some_and(|l| l.contains("is on the stick"))));
    // the firmware family is untouched: an update can start once the face is in
    assert!(!fake.ota(|o| o.active("faces")));
}

#[tokio::test]
async fn install_resends_after_a_gap_and_resumes_after_a_drop() {
    let (_, st, fake) = app("resume").await;
    fake.ota(|o| {
        o.drop = [240u32 * 3].into_iter().collect();
        o.disconnect_at = Some(5000);
    });
    let (r, _) = install_on(&st, CATNAP, None).await;
    let r = r.unwrap();
    assert!(fake.faces(|f| f.installed.contains_key("catnap")));
    assert_eq!(r["reconnects"], 1);
    assert!(r["resent"].as_u64().unwrap() > 0);
    assert_eq!(fake.faces(|f| f.puts.len()), 2);
}

#[tokio::test]
async fn install_refuses_a_stick_without_the_face_engine() {
    let (_, st, fake) = app("old").await;
    fake.set_stick("fw", json!("2.3.0"));
    fake.faces(|f| f.supported = false);
    let e = install_on(&st, TEXTCLOCK, None).await.0.unwrap_err();
    assert!(e.contains("update the stick to 2.4.0"), "{e}");
    assert!(fake.faces(|f| f.puts.is_empty()));
}

#[tokio::test]
async fn install_refuses_a_face_that_needs_newer_firmware() {
    let (_, st, fake) = app("newer").await;
    let blob = with_manifest(TEXTCLOCK, |m| {
        m.insert("minFirmware".into(), json!("2.5.0"));
    });
    let e = install_on(&st, &blob, None).await.0.unwrap_err();
    assert!(e.contains("needs firmware 2.5.0"), "{e}");
    assert!(fake.faces(|f| f.puts.is_empty()));
}

#[tokio::test]
async fn install_checks_the_catalog_entry() {
    let (_, st, _) = app("entry").await;
    let entry = json!({"id": "textclock", "version": "1.0.0", "size": TEXTCLOCK.len(), "sha256": "0".repeat(64)});
    assert!(install_on(&st, TEXTCLOCK, Some(&entry)).await.0.unwrap_err().contains("damaged"));
    let entry = json!({"id": "textclock", "version": "1.0.1", "size": TEXTCLOCK.len(), "sha256": sha_hex(TEXTCLOCK)});
    assert!(install_on(&st, TEXTCLOCK, Some(&entry)).await.0.unwrap_err().contains("isn't the one"));
}

#[tokio::test]
async fn install_refusals_in_words() {
    let (_, st, fake) = app("refusals").await;
    for (code, words) in [("full", "remove one"), ("busy", "busy (dictation)"), ("ota", "firmware update is running")] {
        fake.faces(|f| f.refuse = Some(code.into()));
        let e = install_on(&st, TEXTCLOCK, None).await.0.unwrap_err();
        assert!(e.contains(words), "{code}: {e}");
    }
}

#[tokio::test]
async fn a_running_firmware_update_refuses_a_face_and_the_other_way_round() {
    let (_, st, fake) = app("one-at-a-time").await;
    fake.ota(|o| o.session = Some((100, "x".into(), Vec::new())));
    let e = install_on(&st, TEXTCLOCK, None).await.0.unwrap_err();
    assert!(e.contains("firmware update is running"), "{e}");
    // a face mid-way: a firmware begin is refused busy (faces)
    fake.ota(|o| {
        o.session = Some((100, "x".into(), Vec::new()));
        o.family = "faces";
    });
    let mut s = stick_link::ota::Sender::new(st.link.clone(), Arc::new(|_| {}));
    s.timings = quick();
    let e = s.run(&Image::new(vec![0xE9; 100], "9.9.9"), false).await.unwrap_err();
    assert!(e.to_string().contains("busy (faces)"), "{e}");
}

#[tokio::test]
async fn install_reports_a_face_the_stick_finds_invalid() {
    let (_, st, fake) = app("invalid").await;
    fake.faces(|f| f.invalid = Some("image 2: bad header".into()));
    let e = install_on(&st, CATNAP, None).await.0.unwrap_err();
    assert!(e.contains("couldn't load this face (image 2: bad header)"), "{e}");
}

#[tokio::test]
async fn install_cancel() {
    let (_, st, fake) = app("cancel").await;
    fake.ota(|o| o.window = 1024);
    let handle: Arc<Mutex<Option<CancelHandle>>> = Arc::default();
    let h = handle.clone();
    let emit: Emit = Arc::new(move |e: Value| {
        if e["stage"] == "send" && e["progress"].as_i64().unwrap_or(0) >= 20 {
            h.lock().unwrap().as_ref().unwrap().cancel();
        }
    });
    let mut s = sender(&st.link, emit.clone(), "catnap", quick());
    *handle.lock().unwrap() = Some(s.cancel_handle());
    let e = install(&st.link, CATNAP.to_vec(), None, emit, &mut s).await.unwrap_err();
    assert_eq!(e, "cancelled");
    assert!(fake.ota(|o| o.session.is_none() && !o.committed));
    assert!(fake.faces(|f| f.installed.is_empty()));
}

#[tokio::test]
async fn install_no_answer() {
    let (_, st, fake) = app("no-answer").await;
    fake.faces(|f| f.silent = true);
    let e = install_on(&st, TEXTCLOCK, None).await.0.unwrap_err();
    assert!(e.contains("didn't answer the install request"), "{e}");
}

#[tokio::test]
async fn installed_reads_every_page() {
    let (_, st, fake) = app("pages").await;
    let ids: Vec<String> = (0..9).map(|i| format!("face{i:02}")).collect();
    fake.faces(|f| {
        for (i, id) in ids.iter().enumerate() {
            f.installed.insert(id.clone(), ("1.0.0".into(), format!("{i:064x}")));
        }
    });
    let r = installed(&st.link).await.unwrap();
    let got: Vec<&str> = r["faces"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()).collect();
    assert_eq!(got, ids);
    assert_eq!((r["free"].as_i64(), r["max"].as_i64()), (Some(700), Some(12)));
    assert_eq!(fake.faces(|f| f.lines.clone()), vec!["@faces", "@faces page=1"]);
}

#[tokio::test]
async fn installed_needs_an_answer() {
    let (_, st, fake) = app("no-list").await;
    fake.faces(|f| f.supported = false);
    assert!(installed(&st.link).await.unwrap_err().contains("didn't list"));
}

#[tokio::test]
async fn remove_and_use() {
    let (_, st, fake) = app("remove").await;
    fake.faces(|f| f.installed.insert("catnap".into(), ("1.0.0".into(), "ab".repeat(32))));
    let t = Duration::from_millis(500);
    assert_eq!(remove(&st.link, "catnap", t).await.unwrap(), json!({"id": "catnap"}));
    assert_eq!(fake.faces(|f| f.removed.clone()), vec!["catnap"]);
    assert!(remove(&st.link, "catnap", t).await.unwrap_err().contains("isn't on the stick"));
    assert!(remove(&st.link, "../x", t).await.unwrap_err().contains("isn't a face id"));
    assert_eq!(use_face(&st.link, "catnap").await.unwrap(), json!({"id": "catnap"}));
    assert_eq!(fake.faces(|f| f.lines.last().cloned()), Some("@face id=catnap".into()));
    assert_eq!(fake.stick_state()["face"], "catnap");
}

#[tokio::test]
async fn remove_no_answer() {
    let (_, st, fake) = app("remove-silent").await;
    fake.faces(|f| f.silent = true);
    let e = remove(&st.link, "catnap", Duration::from_millis(300)).await.unwrap_err();
    assert!(e.contains("didn't answer"), "{e}");
}

#[test]
fn annotate_compares_by_version_and_sha() {
    let sha = sha_hex(CATNAP);
    let f = |id: &str, name: &str, v: &str, sha: &str, minfw: &str| json!({"id": id, "name": name, "version": v, "sha256": sha, "format": 1, "minFirmware": minfw});
    let cat = json!({"faces": [f("catnap", "Cat Nap", "1.1.0", &sha, "2.4.0"),
                               f("textclock", "Text Clock", "1.0.0", &"ab".repeat(32), "2.4.0"),
                               f("future", "Future", "1.0.0", &"cd".repeat(32), "2.9.0")],
                     "retired": ["oldface"]});
    let support = stick_support(&json!({"fw": "2.4.0", "pkgFaces": {"n": 3, "fmt": [1]}}));
    let on_stick = vec![
        json!({"id": "catnap", "v": "1.0.0", "sha": "12345678"}),
        json!({"id": "textclock", "v": "1.0.0", "sha": "abababab"}),
        json!({"id": "oldface", "v": "1.0.0", "sha": "00000000", "err": "image 0: bad header"}),
    ];
    let (listed, mine) = annotate(Some(&cat), Some(&support), Some(&on_stick));
    let by = |l: &[Value], id: &str| l.iter().find(|f| f["id"] == id).unwrap().clone();
    assert!(by(&listed, "catnap")["update"] == true && by(&listed, "catnap")["installed"] == "1.0.0");
    assert_eq!(by(&listed, "textclock")["update"], false);
    assert!(by(&listed, "future")["fits"].as_str().unwrap().contains("2.9.0"));
    assert!(by(&listed, "future")["installed"].is_null());
    let c = by(&mine, "catnap");
    assert!(c["update"] == true && c["name"] == "Cat Nap" && c["latest"] == "1.1.0");
    assert_eq!(by(&mine, "textclock")["update"], false);
    let o = by(&mine, "oldface");
    assert!(o["retired"] == true && o["inCatalog"] == false && o["err"].is_string());
    // same version, another sha: a rebuilt face is an update too
    let rebuilt = json!({"faces": [f("textclock", "Text Clock", "1.0.0", &"ff".repeat(32), "2.4.0")]});
    let (_, mine) = annotate(Some(&rebuilt), Some(&support), Some(&on_stick[1..2]));
    assert_eq!(mine[0]["update"], true);
}

// --- the catalog ------------------------------------------------------------------------------------

/// A test key pair and the keys list the app would carry (a junk one first, like the Python test).
fn signer() -> (SigningKey, Vec<String>) {
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let keys = vec!["bm90IGEga2V5IGF0IGFsbCBidXQgMzIgYnl0ZXMhIQ==".to_string(), b64(key.verifying_key().as_bytes())];
    (key, keys)
}

fn sign(key: &SigningKey, data: &[u8]) -> String {
    format!("{}\n", b64(&key.sign(data).to_bytes()))
}

/// A request the fake server got: the URL and its headers.
type Call = (String, Vec<(String, String)>);

/// A fake catalog server: path -> bytes; records requests.
struct Web {
    files: Mutex<HashMap<String, Vec<u8>>>,
    calls: Mutex<Vec<Call>>,
    down: AtomicBool,
}

const ETAG: &str = "\"v1\"";

impl Fetch for Web {
    fn get(&self, url: &str, headers: &[(String, String)], limit: usize) -> Result<Fetched, String> {
        self.calls.lock().unwrap().push((url.to_string(), headers.to_vec()));
        if self.down.load(Ordering::Relaxed) {
            return Err("offline".into());
        }
        if headers.iter().any(|(k, v)| k == "If-None-Match" && v == ETAG) {
            return Ok((304, HashMap::new(), Vec::new()));
        }
        let path = url.strip_prefix(BASE).ok_or("HTTP 404")?;
        let body = self.files.lock().unwrap().get(path).cloned().ok_or("HTTP 404")?;
        if body.len() > limit {
            return Err("too big".into());
        }
        let h = HashMap::from([
            ("ETag".to_string(), ETAG.to_string()),
            ("Last-Modified".to_string(), "Fri, 06 Nov 2026 12:00:00 GMT".to_string()),
        ]);
        Ok((200, h, body))
    }
}

impl Web {
    fn set(&self, path: &str, data: Vec<u8>) {
        self.files.lock().unwrap().insert(path.into(), data);
    }
    fn down(&self, d: bool) {
        self.down.store(d, Ordering::Relaxed);
    }
}

fn make_catalog(key: Option<&SigningKey>) -> Arc<Web> {
    let cat = json!({"catalog": 1, "generated": "2026-11-06T12:00:00Z", "formats": [1], "retired": [],
        "faces": [{"id": "catnap", "name": "Cat Nap", "version": "1.0.0", "about": "A cat",
                   "author": {"name": "StickyBot", "github": "x"}, "license": "CC0-1.0", "tags": ["animated"],
                   "channel": "examples", "format": 1, "minFirmware": "2.4.0", "motion": "calm",
                   "sfb": "catnap/catnap-1.0.0.sfb", "size": CATNAP.len(), "sha256": sha_hex(CATNAP),
                   "previews": {"portrait": "catnap/portrait.png", "landscape": "catnap/landscape.png"}}]});
    let data = cat.to_string().into_bytes();
    let mut files = HashMap::from([
        ("catnap/catnap-1.0.0.sfb".to_string(), CATNAP.to_vec()),
        ("catnap/portrait.png".to_string(), b"\x89PNG\r\n\x1a\nportrait".to_vec()),
        ("catnap/landscape.png".to_string(), b"\x89PNG\r\n\x1a\nland".to_vec()),
    ]);
    if let Some(k) = key {
        files.insert("catalog.json.sig".into(), sign(k, &data).into_bytes());
    }
    files.insert("catalog.json".into(), data);
    Arc::new(Web { files: Mutex::new(files), calls: Mutex::default(), down: AtomicBool::new(false) })
}

fn catalog_url() -> String {
    format!("{BASE}catalog.json")
}

#[test]
fn signature_verification() {
    let (key, keys) = signer();
    let sig = sign(&key, b"hello");
    assert!(verify_signature(b"hello", &sig, &keys));
    assert!(!verify_signature(b"hellO", &sig, &keys));
    assert!(!verify_signature(b"hello", "not base64!", &keys));
    assert!(!verify_signature(b"hello", &b64(&[b'x'; 64]), &keys));
    assert!(!verify_signature(b"hello", &sig, &[]));
    // with another key (the built-in one): no
    let builtin: Vec<String> = CATALOG_KEYS.iter().map(|k| k.to_string()).collect();
    assert!(!verify_signature(b"hello", &sig, &builtin));
}

#[test]
fn the_built_in_key_is_a_key_and_the_python_apps() {
    assert!(!CATALOG_KEYS.is_empty());
    for k in CATALOG_KEYS {
        let raw = base64::engine::general_purpose::STANDARD.decode(k).unwrap();
        let raw: [u8; 32] = raw.try_into().unwrap();
        ed25519_dalek::VerifyingKey::from_bytes(&raw).unwrap();
    }
    // the same list as faces.py (which tools/faces/sign_catalog.py checks against CI's key)
    let py = include_str!("../../../../../mac-agent/stick_agent/faces.py");
    let start = py.find("CATALOG_KEYS = [").unwrap();
    let block = &py[start..start + py[start..].find(']').unwrap()];
    let listed: Vec<&str> = block.split('"').skip(1).step_by(2).collect();
    assert_eq!(listed, CATALOG_KEYS);
}

#[test]
fn catalog_fetch_cache_and_offline() {
    let (key, keys) = signer();
    let web = make_catalog(Some(&key));
    let dir = tmp_home("cache").join("faces");
    let now = Arc::new(Mutex::new(1000.0));
    let n2 = now.clone();
    let mut cat = Catalog::new(&catalog_url(), dir.clone(), web.clone())
        .with_keys(keys.clone(), &catalog_url())
        .with_clock(Arc::new(move || *n2.lock().unwrap()));
    let have = cat.refresh(false).unwrap();
    assert_eq!(have["signed"], true);
    assert_eq!(have["catalog"]["faces"][0]["sfb"], format!("{BASE}catnap/catnap-1.0.0.sfb"));
    assert!(dir.join("catalog.json").is_file() && cat.error.is_none());
    let n = web.calls.lock().unwrap().len();
    assert_eq!(cat.refresh(false).unwrap()["catalog"]["faces"][0]["id"], "catnap");
    assert_eq!(web.calls.lock().unwrap().len(), n); // fresh enough: no request
    *now.lock().unwrap() += REFRESH_S + 1.0;
    cat.refresh(false).unwrap();
    let last = web.calls.lock().unwrap().last().cloned().unwrap();
    assert!(last.1.contains(&("If-None-Match".to_string(), ETAG.to_string()))); // conditional, answered 304
    assert!(last.1.iter().any(|(k, _)| k == "If-Modified-Since"));
    web.down(true);
    let have = cat.refresh(true);
    assert!(have.is_some() && cat.error.as_deref().unwrap().contains("last copy")); // offline: the cached copy
    // a fresh app (another Catalog) also browses offline from the cache
    let other = Catalog::new(&catalog_url(), dir, web).with_keys(keys, &catalog_url());
    assert!(!other.cached().unwrap()["catalog"]["faces"].as_array().unwrap().is_empty());
}

#[test]
fn the_default_catalog_must_be_signed() {
    let (_, keys) = signer();
    let web = make_catalog(None);
    let dir = tmp_home("signed").join("faces");
    let mut cat = Catalog::new(&catalog_url(), dir.clone(), web.clone()).with_keys(keys.clone(), &catalog_url());
    assert!(cat.refresh(false).is_none() && cat.error.as_deref().unwrap().contains("signature"));
    web.set("catalog.json.sig", b64(&[b'x'; 64]).into_bytes());
    assert!(cat.refresh(true).is_none() && cat.error.as_deref().unwrap().contains("signature"));
    // another URL (a fork, a test server) may be unsigned
    let mut other =
        Catalog::new(&catalog_url(), dir, make_catalog(None)).with_keys(keys, "https://elsewhere/catalog.json");
    let have = other.refresh(false).unwrap();
    assert_eq!(have["signed"], false);
}

#[test]
fn a_tampered_cache_is_ignored() {
    let (key, keys) = signer();
    let dir = tmp_home("tamper").join("faces");
    let mut cat = Catalog::new(&catalog_url(), dir.clone(), make_catalog(Some(&key))).with_keys(keys, &catalog_url());
    cat.refresh(false).unwrap();
    let p = dir.join("catalog.json");
    let text = std::fs::read_to_string(&p).unwrap().replace("Cat Nap", "Cat Nab");
    std::fs::write(&p, text).unwrap();
    assert!(cat.cached().is_none());
}

#[test]
fn a_newer_catalog_asks_for_an_update() {
    let web = make_catalog(None);
    web.set("catalog.json", br#"{"catalog": 2, "faces": []}"#.to_vec());
    let mut cat = Catalog::new(&catalog_url(), tmp_home("newer-cat").join("faces"), web).with_keys(vec![], "x");
    assert!(cat.refresh(false).is_none() && cat.error.as_deref().unwrap().contains("update StickyBot"));
}

#[test]
fn download_verifies_and_caches() {
    let web = make_catalog(None);
    let dir = tmp_home("download").join("faces");
    let mut cat = Catalog::new(&catalog_url(), dir.clone(), web.clone()).with_keys(vec![], "x");
    cat.refresh(false).unwrap();
    let entry = cat.entry("catnap").unwrap();
    assert_eq!(cat.download(&entry).unwrap(), CATNAP);
    let cached = dir.join("catnap/catnap-1.0.0.sfb");
    assert_eq!(std::fs::read(&cached).unwrap(), CATNAP);
    web.down(true);
    assert_eq!(cat.download(&entry).unwrap(), CATNAP); // from the cache
    web.down(false);
    std::fs::remove_file(&cached).unwrap();
    let mut bad = CATNAP.to_vec();
    *bad.last_mut().unwrap() ^= 1;
    web.set("catnap/catnap-1.0.0.sfb", bad);
    assert!(cat.download(&entry).unwrap_err().contains("damaged"));
    assert!(!cached.exists());
}

#[test]
fn previews_are_cached() {
    let web = make_catalog(None);
    let mut cat = Catalog::new(&catalog_url(), tmp_home("previews").join("faces"), web.clone()).with_keys(vec![], "x");
    cat.refresh(false).unwrap();
    assert!(cat.preview("catnap", "portrait").unwrap().ends_with(b"portrait"));
    web.down(true);
    assert!(cat.preview("catnap", "portrait").unwrap().ends_with(b"portrait"));
    assert!(cat.preview("catnap", "../../etc").is_none());
    assert!(cat.preview("nope", "portrait").is_none());
}

#[test]
fn upload_must_be_a_compiled_face() {
    assert!(read_upload(TEXTCLOCK.to_vec(), "").is_ok());
    assert!(read_upload(br#"{"id": "x"}"#.to_vec(), "manifest.json").unwrap_err().contains("validate.py --compile"));
    let big = [b"SFB1".to_vec(), vec![0; MAX_SFB]].concat();
    assert!(read_upload(big, "").unwrap_err().contains("64 KB"));
    assert!(read_upload(Vec::new(), "").unwrap_err().contains("empty"));
}

#[test]
fn config_url_must_be_https() {
    let v = |u: Value| valid_config(json!({"faces_catalog_url": u}).as_object().unwrap());
    assert_eq!(v(json!("https://x.example/catalog.json")), "");
    assert_eq!(v(json!("http://localhost:8000/catalog.json")), "");
    assert!(!v(json!("http://evil.example/c.json")).is_empty());
    assert!(!v(json!(3)).is_empty());
    assert_eq!(valid_config(&Map::new()), "");
    // and PUT /api/config refuses it
    assert!(!crate::config::invalid(json!({"faces_catalog_url": "ftp://x/y"}).as_object().unwrap()).is_empty());
    assert!(crate::config::defaults().contains_key("faces_catalog_url"));
}

// --- API ----------------------------------------------------------------------------------------------

/// The app with the fake stick, a signed fake catalog server and its URL in the settings.
async fn client(name: &str) -> (Router, Arc<AppState>, FakeStick, Arc<Web>) {
    let (router, st, fake) = app(name).await;
    let (key, keys) = signer();
    let web = make_catalog(Some(&key));
    st.faces.set_fetch(web.clone());
    st.faces.set_keys(keys, &catalog_url());
    st.update_config(json!({"faces_catalog_url": catalog_url()}).as_object().unwrap().clone()).unwrap();
    (router, st, fake, web)
}

fn req(method: &str, path: &str, body: Option<Value>, token: Option<&str>) -> Request {
    let mut b = Request::builder().method(method).uri(path).header("host", format!("localhost:{PORT}"));
    if let Some(t) = token {
        b = b.header("x-stick-token", t);
    }
    b.body(body.map(|v| Body::from(v.to_string())).unwrap_or_default()).unwrap()
}

async fn call(app: &Router, r: Request) -> (StatusCode, Value, String) {
    let resp = app.clone().oneshot(r).await.unwrap();
    let status = resp.status();
    let ctype = resp.headers().get("content-type").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, v, ctype)
}

async fn events_of(app: &Router, job: &Value) -> Vec<Value> {
    let path = format!("/api/faces/jobs/{}", job.as_str().unwrap());
    let (_, v, _) =
        tokio::time::timeout(Duration::from_secs(30), call(app, req("GET", &path, None, None))).await.unwrap();
    v.as_str()
        .unwrap()
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[tokio::test]
async fn api_needs_the_token() {
    let (app, ..) = client("api-token").await;
    for path in ["/api/faces/install", "/api/faces/remove", "/api/faces/use", "/api/faces/jobs/x/cancel"] {
        assert_eq!(call(&app, req("POST", path, Some(json!({})), None)).await.0, StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test]
async fn api_catalog_with_the_stick() {
    let (app, _, fake, _) = client("api-catalog").await;
    fake.faces(|f| f.installed.insert("catnap".into(), ("0.9.0".into(), format!("12345678{}", "0".repeat(56)))));
    let (_, d, _) = call(&app, req("GET", "/api/faces/catalog", None, None)).await;
    assert_eq!((d["signed"].as_bool(), d["error"].is_null()), (Some(true), true), "{d}");
    let f = &d["faces"][0];
    assert!(f["id"] == "catnap" && f["fits"].is_null() && f["update"] == true);
    assert_eq!(f["previews"]["portrait"], "/api/faces/preview/catnap/portrait");
    assert!(d["installed"][0]["update"] == true && d["installed"][0]["name"] == "Cat Nap");
    assert!(d["stick"]["supported"] == true && d["stick"]["max"] == 12 && d["stick"]["fw"] == "2.4.0");
    let (s, _, ctype) = call(&app, req("GET", "/api/faces/preview/catnap/landscape", None, None)).await;
    assert_eq!((s, ctype.as_str()), (StatusCode::OK, "image/png"));
    assert_eq!(call(&app, req("GET", "/api/faces/preview/catnap/sideways", None, None)).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn api_catalog_with_old_firmware() {
    let (app, _, fake, _) = client("api-old").await;
    fake.set_stick("fw", json!("2.3.0"));
    fake.faces(|f| f.supported = false);
    let (_, d, _) = call(&app, req("GET", "/api/faces/catalog", None, None)).await;
    assert_eq!(d["stick"]["supported"], false);
    assert!(d["stick"]["error"].as_str().unwrap().contains("2.4.0"));
    assert!(d["installed"].is_null() && d["faces"][0]["fits"].as_str().unwrap().contains("2.4.0"));
}

#[tokio::test]
async fn api_catalog_offline_without_a_stick() {
    let (app, st, _, web) = client("api-offline").await;
    call(&app, req("GET", "/api/faces/catalog", None, None)).await;
    web.down(true);
    st.link.pause(true);
    let mut w = st.link.watch_state();
    tokio::time::timeout(Duration::from_secs(5), w.wait_for(|s| *s != LinkState::Connected)).await.unwrap().unwrap();
    let (_, d, _) = call(&app, req("GET", "/api/faces/catalog?refresh=1", None, None)).await;
    assert!(d["offline"] == true && !d["faces"].as_array().unwrap().is_empty() && d["stick"]["connected"] == false);
}

#[tokio::test]
async fn api_install_from_the_catalog() {
    let (app, st, fake, _) = client("api-install").await;
    let tok = Some(st.token.as_str());
    let (_, r, _) = call(&app, req("POST", "/api/faces/install", Some(json!({"id": "catnap"})), tok)).await;
    let events = events_of(&app, &r["job"]).await;
    let last = events.last().unwrap();
    assert_eq!(last["ok"], true, "{last}");
    assert_eq!(last["result"]["id"], "catnap");
    assert!(events.iter().any(|e| e["stage"] == "download"));
    assert_eq!(fake.faces(|f| f.installed["catnap"].0.clone()), "1.0.0");
    assert!(!st.faces.busy());
}

#[tokio::test]
async fn api_install_from_a_file() {
    let (app, st, _, _) = client("api-file").await;
    let tok = Some(st.token.as_str());
    let body = json!({"file": b64(TEXTCLOCK), "name": "textclock.sfb"});
    let (_, r, _) = call(&app, req("POST", "/api/faces/install", Some(body), tok)).await;
    let last = events_of(&app, &r["job"]).await.pop().unwrap();
    assert!(last["ok"] == true && last["result"]["id"] == "textclock", "{last}");
    let body = json!({"file": b64(b"nope"), "name": "x.png"});
    let (s, r, _) = call(&app, req("POST", "/api/faces/install", Some(body), tok)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(r["error"].as_str().unwrap().contains("compiled face"));
    let (s, _, _) = call(&app, req("POST", "/api/faces/install", Some(json!({"file": "%%%"})), tok)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn api_install_errors() {
    let (app, st, fake, _) = client("api-errors").await;
    let tok = Some(st.token.as_str());
    let (s, _, _) = call(&app, req("POST", "/api/faces/install", Some(json!({"id": "nosuch"})), tok)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    fake.faces(|f| f.refuse = Some("full".into()));
    let (_, r, _) = call(&app, req("POST", "/api/faces/install", Some(json!({"id": "catnap"})), tok)).await;
    let last = events_of(&app, &r["job"]).await.pop().unwrap();
    assert_eq!(last["ok"], false);
    assert!(last["error"].as_str().unwrap().starts_with("Your stick has no room"), "{last}");
    st.link.pause(true);
    let mut w = st.link.watch_state();
    tokio::time::timeout(Duration::from_secs(5), w.wait_for(|s| *s != LinkState::Connected)).await.unwrap().unwrap();
    let (s, _, _) = call(&app, req("POST", "/api/faces/install", Some(json!({"id": "catnap"})), tok)).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn api_install_cancel() {
    let (app, st, fake, _) = client("api-cancel").await;
    let tok = Some(st.token.as_str());
    fake.ota(|o| o.lose_acks = 100_000); // no ack ever arrives: the job keeps resending
    let (_, r, _) = call(&app, req("POST", "/api/faces/install", Some(json!({"id": "catnap"})), tok)).await;
    let job = r["job"].as_str().unwrap().to_string();
    let (s, _, _) = call(&app, req("POST", "/api/faces/install", Some(json!({"id": "catnap"})), tok)).await;
    assert_eq!(s, StatusCode::CONFLICT); // one at a time
    let path = format!("/api/faces/jobs/{job}/cancel");
    assert_eq!(call(&app, req("POST", &path, None, tok)).await.0, StatusCode::OK);
    let last = events_of(&app, &json!(job)).await.pop().unwrap();
    assert_eq!(last["ok"], false);
    assert!(last["error"].as_str().unwrap().to_lowercase().contains("cancel"), "{last}");
    assert_eq!(call(&app, req("POST", &path, None, tok)).await.0, StatusCode::CONFLICT);
    assert_eq!(call(&app, req("POST", "/api/faces/jobs/nope/cancel", None, tok)).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn api_remove_and_use() {
    let (app, st, fake, _) = client("api-remove").await;
    let tok = Some(st.token.as_str());
    fake.faces(|f| f.installed.insert("catnap".into(), ("1.0.0".into(), "ab".repeat(32))));
    let (_, r, _) = call(&app, req("POST", "/api/faces/remove", Some(json!({"id": "catnap"})), tok)).await;
    assert_eq!(r, json!({"ok": true, "id": "catnap"}));
    let (s, _, _) = call(&app, req("POST", "/api/faces/remove", Some(json!({"id": "catnap"})), tok)).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (_, r, _) = call(&app, req("POST", "/api/faces/use", Some(json!({"id": "catnap"})), tok)).await;
    assert_eq!(r["ok"], true);
}

#[tokio::test]
async fn firmware_update_waits_for_a_face_install() {
    let (app, st, _, _) = client("api-fw").await;
    st.faces.set_busy(true);
    let (s, r, _) = call(&app, req("POST", "/api/firmware/ota", Some(json!({})), Some(&st.token))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(r["error"].as_str().unwrap().contains("face"), "{r}");
    let (s, _, _) = call(&app, req("POST", "/api/faces/remove", Some(json!({"id": "catnap"})), Some(&st.token))).await;
    assert_eq!(s, StatusCode::CONFLICT);
}
