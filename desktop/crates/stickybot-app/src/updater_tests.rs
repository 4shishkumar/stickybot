//! The path rules and error words, then the real plugin end to end: a key pair made here, a local
//! 127.0.0.1 server with `latest.json` and a signed `StickyBot.app.tar.gz` of a tiny fake bundle,
//! the plugin's download, signature check and bundle swap into a temporary copy (the plugin
//! installs where `executable_path` says). Nothing is read from or written to /Applications, the
//! home folder or the Keychain, and no key material is in the repo.

use super::*;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};

// --- the path rules ----------------------------------------------------------------------------------------------------

const EXE: &str = "/Applications/StickyBot.app/Contents/MacOS/stickybot-rs";

fn good() -> FolderFacts {
    FolderFacts {
        folder_writable: true,
        bundle_writable: true,
        sticky: true,
        folder_uid: 0,
        bundle_uid: 501,
        euid: 501,
    }
}

#[test]
fn the_bundle_is_the_app_around_the_executable() {
    assert_eq!(bundle_of(Path::new(EXE)), Some(PathBuf::from("/Applications/StickyBot.app")));
    assert_eq!(bundle_of(Path::new("/x/desktop/target/release/stickybot-rs")), None);
    assert_eq!(bundle_of(Path::new("/x/Contents/MacOS/stickybot-rs")), None, "no .app around it");
    assert_eq!(bundle_of(Path::new("stickybot-rs")), None);
}

#[test]
fn where_the_app_may_replace_itself() {
    assert_eq!(problem(Path::new(EXE), Some(&good())), None);
    // a developer build, not a bundle
    let dev = problem(Path::new("/x/desktop/target/debug/stickybot-rs"), None).unwrap();
    assert!(dev.contains("installed StickyBot.app") && dev.contains("by hand"), "{dev}");
    // Gatekeeper's randomised read-only copy
    let moved = "/private/var/folders/ab/xyz/T/AppTranslocation/0A1B2C/d/StickyBot.app/Contents/MacOS/stickybot-rs";
    let t = problem(Path::new(moved), Some(&good())).unwrap();
    assert!(t.contains("temporary read-only copy") && t.contains("move it to Applications"), "{t}");
    // a folder this user can't write: no administrator prompt, the download page instead
    let ro = FolderFacts { folder_writable: false, ..good() };
    let p = problem(Path::new(EXE), Some(&ro)).unwrap();
    assert!(p.contains("/Applications/StickyBot.app") && p.contains("download it by hand"), "{p}");
    assert!(problem(Path::new(EXE), Some(&FolderFacts { bundle_writable: false, ..good() })).is_some());
    // /Applications is sticky: an app another user put there can't be moved away
    let other_owner = FolderFacts { bundle_uid: 0, folder_uid: 0, ..good() };
    assert!(problem(Path::new(EXE), Some(&other_owner)).is_some());
    assert_eq!(problem(Path::new(EXE), Some(&FolderFacts { sticky: false, ..other_owner.clone() })), None);
    assert_eq!(problem(Path::new(EXE), Some(&FolderFacts { euid: 0, ..other_owner })), None);
    // the file system can't be read at all
    assert!(problem(Path::new(EXE), None).is_some());
}

#[test]
fn the_facts_of_a_real_folder() {
    let root = tmp("facts");
    let app = root.join("StickyBot.app");
    std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    let f = facts(&app).expect("facts");
    assert!(f.folder_writable && f.bundle_writable && !f.sticky);
    assert_eq!(f.euid, f.bundle_uid, "the test made it");
    assert_eq!(facts(&root.join("missing.app")), None);
    let exe = app.join("Contents/MacOS/stickybot-rs");
    assert_eq!(problem(&exe, facts(&app).as_ref()), None);
}

#[test]
fn the_plugins_errors_become_the_servers_kinds() {
    let at = Path::new("/Applications/StickyBot.app");
    let kind = |e: Error| map_error(&e, at).kind();
    assert_eq!(kind(Error::SignatureUtf8("x".into())), "signature");
    assert_eq!(kind(Error::MissingSignedVersion), "signature");
    assert_eq!(kind(Error::SignedVersionMismatch { signed: "1.0.0".into(), announced: "9.0.0".into() }), "signature");
    assert_eq!(kind(Error::ReleaseNotFound), "network");
    assert_eq!(kind(Error::Network("status 500".into())), "network");
    assert_eq!(kind(Error::Io(std::io::ErrorKind::PermissionDenied.into())), "not-writable");
    assert_eq!(kind(Error::TempDirNotOnSameMountPoint), "not-writable");
    assert_eq!(kind(Error::Io(std::io::ErrorKind::NotFound.into())), "other");
    assert_eq!(kind(Error::TargetNotFound("darwin-aarch64".into())), "no-update");
    let w = map_error(&Error::Io(std::io::ErrorKind::PermissionDenied.into()), at).words();
    assert!(w.contains("/Applications/StickyBot.app"), "{w}");
    let w =
        map_error(&Error::PreviousAppNotRestored("/Applications/StickyBot (previous version).app".into()), at).words();
    assert!(w.contains("previous version") && w.contains("move it back"), "{w}");
}

// --- the plugin against a local server -----------------------------------------------------------------------------

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("stickybot-updater-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A key pair made now: (the public key as `tauri.conf.json` holds it, the secret key).
fn keypair() -> (String, minisign::SecretKey) {
    let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let pk = kp.pk.to_box().unwrap().to_string();
    (STANDARD.encode(pk), kp.sk)
}

/// The `.sig` a build leaves, as `latest.json` carries it: the signature file's text, base64. Its
/// trusted comment names the version it was signed for, as the Tauri CLI writes it.
fn sign(sk: &minisign::SecretKey, data: &[u8], version: &str) -> String {
    let comment = format!("timestamp:1\tfile:StickyBot.app.tar.gz\tversion:{version}");
    let sig = minisign::sign(None, sk, Cursor::new(data), Some(&comment), Some("test key")).unwrap();
    STANDARD.encode(sig.to_string())
}

/// `StickyBot.app.tar.gz` as the bundler makes it: the .app folder at its root, a one-file bundle.
fn tarball(exe_content: &str) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);
    let mut h = tar::Header::new_gnu();
    h.set_size(exe_content.len() as u64);
    h.set_mode(0o755);
    h.set_cksum();
    tar.append_data(&mut h, "StickyBot.app/Contents/MacOS/stickybot-rs", exe_content.as_bytes()).unwrap();
    tar.into_inner().unwrap().finish().unwrap()
}

/// 127.0.0.1 on a free port; GETs answered from what was `put` (path -> bytes), 404 otherwise.
struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    routes: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let (addr, stop) = (listener.local_addr().unwrap(), Arc::new(AtomicBool::new(false)));
        let routes: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::default();
        let (flag, table) = (stop.clone(), routes.clone());
        std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                let Ok((mut s, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                s.set_nonblocking(false).unwrap();
                s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut req = Vec::new();
                let mut buf = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => req.extend_from_slice(&buf[..n]),
                    }
                }
                let line = String::from_utf8_lossy(&req).lines().next().unwrap_or("").to_string();
                let path = line.split(' ').nth(1).unwrap_or("/").to_string();
                let found = table.lock().unwrap().get(&path).cloned();
                let (status, body) = match found {
                    Some(b) => ("200 OK", b),
                    None => ("404 Not Found", b"not found".to_vec()),
                };
                let head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(head.as_bytes()).and_then(|()| s.write_all(&body));
            }
        });
        Self { addr, stop, routes }
    }

    fn put(&self, path: &str, body: Vec<u8>) {
        self.routes.lock().unwrap().insert(path.to_string(), body);
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// The key of `{os}-{arch}` in `latest.json`'s `platforms`.
fn target() -> &'static str {
    if cfg!(target_arch = "aarch64") { "darwin-aarch64" } else { "darwin-x86_64" }
}

fn mock_app(pubkey: &str) -> tauri::App<tauri::test::MockRuntime> {
    let mut ctx = tauri::test::mock_context(tauri::test::noop_assets());
    ctx.config_mut().plugins.0.insert(
        "updater".into(),
        // as tauri.conf.json has it
        serde_json::json!({"pubkey": pubkey, "endpoints": ["https://example.invalid/latest.json"], "requireSignedVersion": true}),
    );
    tauri::test::mock_builder().plugin(tauri_plugin_updater::Builder::new().build()).build(ctx).unwrap()
}

type Seen = Arc<Mutex<Vec<(u64, Option<u64>)>>>;

fn recorder() -> (Progress, Seen) {
    let seen: Seen = Arc::default();
    let s = seen.clone();
    (Arc::new(move |b, t| s.lock().unwrap().push((b, t))), seen)
}

/// An app that trusts `trusted_pubkey`, a fake installed bundle (`old`), and a local server that
/// announces `version` and serves `body` with `signature` over it (what a release does, or a forgery).
struct Rig {
    server: Server,
    installer: TauriInstaller<tauri::test::MockRuntime>,
    exe: PathBuf,
    _app: tauri::App<tauri::test::MockRuntime>,
}

impl Rig {
    fn new(name: &str, trusted_pubkey: &str, version: &str, body: Vec<u8>, signature: &str) -> Self {
        let server = Server::start();
        server.put("/StickyBot.app.tar.gz", body);
        let manifest = serde_json::json!({"version": version, "notes": "test", "platforms": {target(): {
            "signature": signature, "url": server.url("/StickyBot.app.tar.gz")}}});
        server.put("/latest.json", manifest.to_string().into_bytes());
        let exe = tmp(name).join("Apps/StickyBot.app/Contents/MacOS/stickybot-rs");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "old").unwrap();
        let app = mock_app(trusted_pubkey);
        let installer = TauriInstaller::at(app.handle().clone(), exe.clone());
        Self { server, installer, exe, _app: app }
    }

    fn bundle_content(&self) -> String {
        std::fs::read_to_string(&self.exe).unwrap()
    }

    async fn download(&self) -> Result<Box<dyn Staged>, InstallError> {
        self.installer.download(self.server.url("/latest.json"), recorder().0).await
    }
}

fn signed_rig(name: &str, version: &str) -> Rig {
    let (pubkey, sk) = keypair();
    let tar = tarball("new");
    let sig = sign(&sk, &tar, version);
    Rig::new(name, &pubkey, version, tar, &sig)
}

#[tokio::test]
async fn a_signed_update_is_downloaded_verified_and_swapped_into_the_bundle() {
    let r = signed_rig("e2e", "99.0.0");
    let (progress, seen) = recorder();
    let staged = r.installer.download(r.server.url("/latest.json"), progress).await.expect("downloaded and verified");
    assert_eq!(staged.version(), "99.0.0");
    // progress came with the size the server said
    let seen = seen.lock().unwrap().clone();
    let size = tarball("new").len() as u64;
    assert_eq!(seen.last().copied(), Some((size, Some(size))), "{seen:?}");
    assert!(seen.windows(2).all(|w| w[0].0 < w[1].0), "bytes only grow: {seen:?}");
    // verified is not replaced
    assert_eq!(r.bundle_content(), "old");
    tokio::task::spawn_blocking(move || staged.install()).await.unwrap().expect("installed");
    assert_eq!(r.bundle_content(), "new", "the bundle now holds the downloaded one");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&r.exe).unwrap().permissions().mode() & 0o111, 0o111, "still executable");
}

#[tokio::test]
async fn a_tampered_download_is_refused_with_the_signature_kind_and_nothing_is_replaced() {
    let (pubkey, sk) = keypair();
    let tar = tarball("new");
    let sig = sign(&sk, &tar, "99.0.0");
    let mut evil = tar.clone();
    let mid = evil.len() / 2;
    evil[mid] ^= 0xff;
    let r = Rig::new("tamper", &pubkey, "99.0.0", evil, &sig);
    let e = r.download().await.err().expect("refused");
    assert_eq!(e.kind(), "signature", "{e}");
    assert!(e.words().contains("signing key"));
    assert_eq!(r.bundle_content(), "old");
    // the same bytes, untouched, are fine
    r.server.put("/StickyBot.app.tar.gz", tar);
    assert!(r.download().await.is_ok());
}

#[tokio::test]
async fn an_update_signed_with_another_key_is_refused() {
    let (pubkey, _mine) = keypair();
    let (_theirs_pub, theirs) = keypair();
    let tar = tarball("new");
    let sig = sign(&theirs, &tar, "99.0.0");
    let r = Rig::new("wrongkey", &pubkey, "99.0.0", tar, &sig);
    let e = r.download().await.err().expect("refused");
    assert_eq!(e.kind(), "signature", "{e}");
    assert_eq!(r.bundle_content(), "old");
}

#[tokio::test]
async fn an_older_release_cannot_be_passed_off_as_a_newer_one() {
    // genuinely signed, but for 1.0.0: an endpoint that announces it as 99.0.0 doesn't get it installed
    let (pubkey, sk) = keypair();
    let tar = tarball("new");
    let sig = sign(&sk, &tar, "1.0.0");
    let r = Rig::new("downgrade", &pubkey, "99.0.0", tar.clone(), &sig);
    let e = r.download().await.err().expect("refused");
    assert_eq!(e.kind(), "signature", "{e}");
    assert_eq!(r.bundle_content(), "old");
    // a signature with no version at all is refused too (the config asks for one)
    let comment = "timestamp:1\tfile:StickyBot.app.tar.gz";
    let bare = minisign::sign(None, &sk, Cursor::new(&tar), Some(comment), Some("t")).unwrap();
    let r = Rig::new("noversion", &pubkey, "99.0.0", tar, &STANDARD.encode(bare.to_string()));
    assert_eq!(r.download().await.err().expect("refused").kind(), "signature");
}

#[tokio::test]
async fn garbage_where_a_signature_belongs_is_refused() {
    let (pubkey, _sk) = keypair();
    let r = Rig::new("garbage", &pubkey, "99.0.0", tarball("new"), "bm90IGEgc2lnbmF0dXJl");
    assert_eq!(r.download().await.err().expect("refused").kind(), "signature");
    let r = Rig::new("empty", &pubkey, "99.0.0", tarball("new"), "");
    assert_eq!(r.download().await.err().expect("refused").kind(), "signature");
    assert_eq!(r.bundle_content(), "old");
}

#[tokio::test]
async fn the_plugin_itself_will_not_install_a_lower_or_equal_version() {
    // 0.1.0 is what the mock app is; the server's own check comes first, this is the second guard
    for version in ["0.1.0", "0.0.9"] {
        let r = signed_rig("noupdate", version);
        let e = r.download().await.err().expect("nothing to install");
        assert_eq!(e.kind(), "no-update", "{version}: {e}");
        assert_eq!(r.bundle_content(), "old");
    }
}

#[tokio::test]
async fn no_update_file_on_the_server_is_a_connection_problem() {
    let r = signed_rig("nofile", "99.0.0");
    let e = r.installer.download(r.server.url("/missing.json"), recorder().0).await.err().expect("refused");
    assert_eq!(e.kind(), "network", "{e}");
    assert!(e.words().contains("no connection"));
    // nothing listening at all
    let dead = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let e = r.installer.download(format!("http://{dead}/latest.json"), recorder().0).await.err().expect("refused");
    assert_eq!(e.kind(), "network", "{e}");
}

#[tokio::test]
async fn an_address_that_is_not_one_is_refused_in_words() {
    let r = signed_rig("badurl", "99.0.0");
    let e = r.installer.download("not a url".into(), recorder().0).await.err().expect("refused");
    assert_eq!(e.kind(), "other");
    assert!(e.words().contains("address"), "{}", e.words());
}

#[test]
fn the_installer_reports_where_the_app_runs() {
    let app = mock_app(&keypair().0);
    // a bundle this user owns: it can replace itself (and the answer is reused)
    let root = tmp("avail");
    let exe = root.join("StickyBot.app/Contents/MacOS/stickybot-rs");
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    let i = TauriInstaller::at(app.handle().clone(), exe.clone());
    assert_eq!(i.unavailable(), None);
    std::fs::remove_dir_all(root.join("StickyBot.app")).unwrap();
    assert_eq!(i.unavailable(), None, "the answer is kept for a moment");
    // not inside a .app (a `cargo run` build), and the translocated copy
    let dev = TauriInstaller::at(app.handle().clone(), PathBuf::from("/x/target/debug/stickybot-rs"));
    assert!(dev.unavailable().unwrap().contains("installed StickyBot.app"));
    let moved = "/private/var/folders/ab/T/AppTranslocation/0A/d/StickyBot.app/Contents/MacOS/stickybot-rs";
    let t = TauriInstaller::at(app.handle().clone(), PathBuf::from(moved));
    assert!(t.unavailable().unwrap().contains("temporary read-only copy"));
}

// --- tauri.conf.json -------------------------------------------------------------------------------------------------

fn conf() -> serde_json::Value {
    serde_json::from_str(include_str!("../tauri.conf.json")).unwrap()
}

#[test]
fn the_compiled_in_update_settings_are_the_public_key_and_the_stable_endpoint() {
    let c = conf();
    let u = &c["plugins"]["updater"];
    // the public key is a minisign one (the private key is only ever an Actions secret)
    let key = String::from_utf8(STANDARD.decode(u["pubkey"].as_str().unwrap()).unwrap()).unwrap();
    assert!(key.starts_with("untrusted comment: minisign public key"), "{key}");
    assert!(!key.to_lowercase().contains("secret"), "{key}");
    let stable = "https://github.com/4shishkumar/stickybot/releases/latest/download/latest.json";
    assert_eq!(u["endpoints"], serde_json::json!([stable]));
    assert_eq!(u["requireSignedVersion"], true);
    // no way to switch https off from the file
    assert!(u.get("dangerousInsecureTransportProtocol").is_none() && u.get("allowDowngrades").is_none());
    // the build script turns updater artifacts on only when it has the key
    assert!(c["bundle"].get("createUpdaterArtifacts").is_none());
    // and the plugin accepts exactly this block
    let mut ctx = tauri::test::mock_context(tauri::test::noop_assets());
    ctx.config_mut().plugins.0.insert("updater".into(), u.clone());
    let built = tauri::test::mock_builder().plugin(tauri_plugin_updater::Builder::new().build()).build(ctx);
    assert!(built.is_ok(), "the plugin refused tauri.conf.json's block");
}

#[test]
fn the_plugin_in_the_app_is_set_up_from_that_file() {
    // main.rs registers the plugin with no key of its own: the file's block is the only source
    let main = include_str!("main.rs");
    assert!(main.contains("tauri_plugin_updater::Builder::new().build()"));
    assert!(!main.contains(".pubkey("), "the key comes from tauri.conf.json");
    assert!(!main.contains("dangerous"));
}
