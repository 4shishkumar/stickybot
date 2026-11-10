//! Face packages on the stick (firmware 2.4.0+): the catalog, installing over Bluetooth, listing,
//! removing and choosing them (faces.py + faces_api.py; docs/faces/FORMAT.md §17-19,
//! docs/faces/CATALOG.md, PROTOCOL.md "Face packages over Bluetooth").
//!
//! - **Catalog**: `catalog.json` from GitHub Pages (config `faces_catalog_url`), fetched with
//!   If-None-Match / If-Modified-Since at most every 6 hours (or when asked) and kept in
//!   `<home>/faces/`, so browsing works offline. The default catalog must carry a valid ed25519
//!   signature (`catalog.json.sig`, made in CI) by one of [`CATALOG_KEYS`]; another URL (a fork, a
//!   test server) may be unsigned.
//! - **Verify**: every `.sfb` is checked before it goes anywhere: the catalog's size and sha256,
//!   the container parsed ([`check_sfb`]: header, budgets, every image), its id / version equal to
//!   the entry's, a format the stick reads and a minFirmware the stick has.
//! - **Install**: `{"faces":{"put":{id, version, size, sha256}}}`, then the bytes over the OTA
//!   characteristic with the firmware update's flow control ([`stick_link::ota::Sender`] with the
//!   `faces` [`Family`]), `end`, and `done`.
//! - **List**: `@faces` (and `@faces page=N` while `more`); **remove**: `{"faces":{"rm":id}}`;
//!   **use**: `@face id=<id>`.

use crate::AppState;
use crate::firmware::{Job, Jobs};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use stick_link::ota::{self, Emit, Family, Image, OtaError, Sender, Timings, Words};
use stick_link::{Backend, Link};

pub const DEFAULT_CATALOG_URL: &str = "https://4shishkumar.github.io/stickybot/faces/catalog.json";
/// The first firmware with the face engine (`pkgFaces` in `@state`).
pub const MIN_FIRMWARE: &str = "2.4.0";
/// catalog.json `"catalog"` this app understands
pub const CATALOG_VERSION: i64 = 1;
/// face formats this app can check
pub const FORMATS: &[u64] = &[1];
/// fetch the catalog at most this often unless asked
pub const REFRESH_S: f64 = 6.0 * 3600.0;
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
pub const MAX_CATALOG: usize = 2 * 1024 * 1024;
pub const MAX_PREVIEW: usize = 256 * 1024;
/// `rm` answers
pub const REPLY: Duration = Duration::from_secs(6);
/// `@faces` pages (6 faces each; the stick keeps 12)
pub const MAX_PAGES: usize = 4;

// The compiled form's limits (FORMAT.md §14, §15; tools/faces/facepkg.py)
pub const MAX_SFB: usize = 64 * 1024;
pub const MAX_IMAGES: usize = 16;
pub const MAX_IMAGE_SIDE: usize = 240;
pub const MAX_PIXELS: usize = 96 * 1024;
pub const MAX_MANIFEST: usize = 16 * 1024;
pub const SFB_VERSION: u16 = 1;

static ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9]{2,11}$").unwrap());
static SHA_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]{64}$").unwrap());

/// ed25519 public keys (base64, raw 32 bytes) whose signature on catalog.json is accepted. A list,
/// so a key can be rotated: add the new one, ship an app, switch CI's secret, then drop the old.
/// The same list as faces.py `CATALOG_KEYS`.
pub const CATALOG_KEYS: &[&str] = &["h5TygDfRbTwqYniIFDTQ0gApT0g7Znx4e1EDzgOfSYA="];

pub fn is_id(s: &str) -> bool {
    ID_RE.is_match(s)
}

/// The stick's refusal / error codes (FORMAT.md §18) in words.
pub fn reason(msg: &Value) -> String {
    let code = msg.get("error").and_then(Value::as_str).unwrap_or("?");
    let why_given = msg.get("why").and_then(Value::as_str).filter(|w| !w.is_empty());
    let why = why_given.unwrap_or(if code == "busy" { "dictation" } else { "no reason given" });
    match code {
        "busy" => format!("the stick is busy ({why}); try again when it's idle"),
        "ota" => "a firmware update is running on the stick; try again when it's done".into(),
        "size" => "this face is too big for the stick (64 KB at most)".into(),
        "full" => "your stick has no room for another face (12 at most); remove one first".into(),
        "format" => "your stick can't read this kind of face; update the stick's firmware".into(),
        "firmware" => "this face needs newer firmware on the stick; update the stick first".into(),
        "sha" => "the face arrived damaged (its checksum didn't match); nothing changed on the stick, try again".into(),
        "invalid" => format!("the stick couldn't load this face ({why})"),
        "timeout" => "the stick gave up waiting for data; try again".into(),
        "not_started" => "the stick had no install running; try again".into(),
        "incomplete" => "the stick is missing part of the face; try again".into(),
        "write" => "writing to the stick's storage failed".into(),
        "read" => "reading the face back on the stick failed".into(),
        "open" => "the stick couldn't start writing the face".into(),
        "notfound" => "that face isn't on the stick".into(),
        "id" => "this face's id belongs to a built-in face; rename it".into(),
        other => match why_given {
            Some(w) => format!("the stick said {other} ({w})"),
            None => format!("the stick said {other}"),
        },
    }
}

pub fn needs_firmware_text(fw: Option<&str>) -> String {
    match fw.filter(|f| !f.is_empty()) {
        Some(fw) => format!("update the stick to {MIN_FIRMWARE} to add faces (it runs {fw})"),
        None => format!("update the stick to {MIN_FIRMWARE} to add faces"),
    }
}

/// "" if `faces_catalog_url` in a `PUT /api/config` body is fine, else what's wrong.
pub fn valid_config(body: &Map<String, Value>) -> String {
    let Some(v) = body.get("faces_catalog_url") else { return String::new() };
    let ok = v.as_str().and_then(|s| url::Url::parse(s).ok()).is_some_and(|u| {
        let local = u.scheme() == "http" && matches!(u.host_str(), Some("localhost" | "127.0.0.1"));
        u.host_str().is_some_and(|h| !h.is_empty()) && (u.scheme() == "https" || local)
    });
    if ok {
        String::new()
    } else {
        "faces_catalog_url must be an https:// URL (or http://localhost for testing)".into()
    }
}

fn sha_hex(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

fn version_tuple(v: &str) -> Vec<u64> {
    ota::version_tuple(v)
}

// --- the compiled face (.sfb) ---------------------------------------------------------------------

/// PackBits, at most `size` bytes out.
fn unpackbits(data: &[u8], size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(size);
    let mut i = 0;
    while out.len() < size && i < data.len() {
        let c = data[i] as usize;
        i += 1;
        if c < 128 {
            let end = (i + c + 1).min(data.len());
            out.extend_from_slice(&data[i..end]);
            i += c + 1;
        } else if c > 128 {
            if i < data.len() {
                out.extend(std::iter::repeat_n(data[i], 257 - c));
            }
            i += 1;
        }
    }
    out.truncate(size);
    out
}

fn u16le(b: &[u8], at: usize) -> usize {
    u16::from_le_bytes([b[at], b[at + 1]]) as usize
}

fn u32le(b: &[u8], at: usize) -> usize {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize
}

/// Parses a compiled face and checks it against the budgets. Returns its manifest; the error
/// says what's wrong. The same reading as tools/faces/facepkg.read_sfb() (the stick checks
/// everything again).
pub fn check_sfb(blob: &[u8]) -> Result<Map<String, Value>, String> {
    let bad = |why: String| format!("not a valid face file: {why}");
    if blob.len() < 16 || &blob[..4] != b"SFB1" {
        return Err(bad("it doesn't start like a compiled face (.sfb)".into()));
    }
    if blob.len() > MAX_SFB {
        return Err(bad(format!("{} bytes, the limit is {MAX_SFB}", blob.len())));
    }
    let (ver, fmt, mlen, nimg) = (u16le(blob, 4) as u16, u16le(blob, 6) as u64, u32le(blob, 8), u16le(blob, 12));
    if ver != SFB_VERSION {
        return Err(bad(format!("container version {ver} (this StickyBot reads {SFB_VERSION}; update StickyBot)")));
    }
    if !FORMATS.contains(&fmt) {
        let f: Vec<String> = FORMATS.iter().map(u64::to_string).collect();
        return Err(bad(format!("face format {fmt} (this StickyBot reads {}; update StickyBot)", f.join(", "))));
    }
    if mlen > MAX_MANIFEST || 16 + mlen > blob.len() || nimg > MAX_IMAGES {
        return Err(bad("bad sizes in the header".into()));
    }
    let man = match std::str::from_utf8(&blob[16..16 + mlen]).ok().map(serde_json::from_str::<Value>) {
        Some(Ok(Value::Object(m))) => m,
        Some(Ok(_)) => return Err(bad("its manifest isn't an object".into())),
        _ => return Err(bad("its manifest isn't JSON".into())),
    };
    if !man.get("id").and_then(Value::as_str).is_some_and(is_id) {
        return Err(bad("no valid id".into()));
    }
    if !man.get("version").and_then(Value::as_str).is_some_and(|v| !version_tuple(v).is_empty()) {
        return Err(bad("no valid version".into()));
    }
    if man.get("format").is_some_and(|f| f.as_u64() != Some(fmt)) {
        return Err(bad("the header and the manifest disagree on the format".into()));
    }
    let mut pos = 16 + mlen + (4 - mlen % 4) % 4;
    let mut pixels = 0usize;
    for n in 0..nimg {
        if pos + 16 > blob.len() {
            return Err(bad(format!("image {n}: truncated header")));
        }
        let h = &blob[pos..pos + 16];
        let (w, ht, bits, nc1, enc, dlen) =
            (u16le(h, 0), u16le(h, 2), h[6] as usize, h[7] as usize, h[9], u32le(h, 12));
        pos += 16;
        let nc = nc1 + 1;
        if ![1, 2, 4, 8].contains(&bits)
            || !(1..=MAX_IMAGE_SIDE).contains(&w)
            || !(1..=MAX_IMAGE_SIDE).contains(&ht)
            || enc > 1
        {
            return Err(bad(format!("image {n}: bad header")));
        }
        pixels += w * ht;
        if pixels > MAX_PIXELS {
            return Err(bad(format!("more than {MAX_PIXELS} pixels of images")));
        }
        pos += 2 * nc + nc % 2 * 2;
        if pos + dlen > blob.len() {
            return Err(bad(format!("image {n}: data past the end of the file")));
        }
        let data = &blob[pos..pos + dlen];
        pos += dlen + (4 - dlen % 4) % 4;
        let stride = (w * bits).div_ceil(8);
        let packed = if enc == 1 { unpackbits(data, stride * ht) } else { data.to_vec() };
        if packed.len() != stride * ht {
            return Err(bad(format!("image {n}: data too short")));
        }
        if nc < 1 << bits {
            // every index must be in the palette
            let (per, mask) = (8 / bits, (1usize << bits) - 1);
            for y in 0..ht {
                let row = &packed[y * stride..(y + 1) * stride];
                for x in 0..w {
                    if ((row[x / per] as usize) >> (8 - bits * (x % per + 1))) & mask >= nc {
                        return Err(bad(format!("image {n}: a pixel past its palette")));
                    }
                }
            }
        }
    }
    if pos > blob.len() + 3 {
        return Err(bad("truncated".into()));
    }
    Ok(man)
}

/// `{"id", "version", "name", "minFirmware", "format", "size", "sha256", "manifest"}` of a checked
/// `.sfb`.
pub fn sfb_info(blob: &[u8]) -> Result<Value, String> {
    let man = check_sfb(blob)?;
    let s = |k: &str| man.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string);
    let id = s("id").unwrap_or_default();
    Ok(json!({"id": id, "version": s("version"), "name": s("name").unwrap_or_else(|| id.clone()),
              "minFirmware": s("minFirmware").unwrap_or_else(|| MIN_FIRMWARE.into()),
              "format": man.get("format").cloned().unwrap_or(json!(1)),
              "size": blob.len(), "sha256": sha_hex(blob), "manifest": man}))
}

/// A file the user picked: a compiled face (`.sfb`). Returns the bytes.
pub fn read_upload(data: Vec<u8>, name: &str) -> Result<Vec<u8>, String> {
    if data.is_empty() {
        return Err("the file is empty".into());
    }
    if data.len() > MAX_SFB {
        return Err(format!("the file is {} KB; a face is 64 KB at most", data.len() / 1024));
    }
    if !data.starts_with(b"SFB1") {
        let base = std::path::Path::new(name).file_name().and_then(|n| n.to_str()).unwrap_or("");
        let manifest = base == "manifest.json" || data.trim_ascii_start().first() == Some(&b'{');
        let hint = if manifest {
            " (that's a package's manifest: compile it with tools/faces/validate.py --compile DIR)"
        } else {
            ""
        };
        return Err(format!("that isn't a compiled face (.sfb){hint}"));
    }
    Ok(data)
}

// --- what the stick can take ------------------------------------------------------------------------

/// From the stick's `@state`: `{"supported", "fw", "fmt", "n"}` (supported: it has the face
/// engine, i.e. `pkgFaces`).
pub fn stick_support(state: &Value) -> Value {
    let pkg = state.get("pkgFaces").filter(|p| p.is_object());
    let fmt: Vec<i64> = pkg
        .and_then(|p| p.get("fmt"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default();
    json!({"supported": pkg.is_some(), "fw": state.get("fw").cloned().unwrap_or(Value::Null), "fmt": fmt,
           "n": pkg.and_then(|p| p.get("n")).and_then(Value::as_i64).unwrap_or(0)})
}

/// `None` if the stick can take this face (a catalog entry or [`sfb_info`]), else why not.
pub fn why_not(face: &Value, support: Option<&Value>) -> Option<String> {
    let Some(support) = support.filter(|s| s["supported"] == true) else {
        return Some(needs_firmware_text(support.and_then(|s| s["fw"].as_str())));
    };
    let need = face.get("minFirmware").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(MIN_FIRMWARE);
    if let Some(fw) = support["fw"].as_str().filter(|f| !f.is_empty())
        && version_tuple(need) > version_tuple(fw)
    {
        return Some(format!("needs firmware {need} (the stick runs {fw}); update the stick first"));
    }
    let fmts = support["fmt"].as_array().cloned().unwrap_or_default();
    let fmt = face.get("format").cloned().unwrap_or(json!(1));
    if !fmts.is_empty() && !fmts.contains(&fmt) {
        return Some(format!("uses face format {fmt}, which the stick's firmware doesn't read; update the stick"));
    }
    None
}

/// The catalog's entry differs from what's installed: a newer version, or the same version with
/// another sha (a rebuilt face).
fn newer(entry: &Value, have: &Value) -> bool {
    let vc = version_tuple(entry["version"].as_str().unwrap_or(""));
    let vh = version_tuple(have["v"].as_str().unwrap_or(""));
    if vc != vh {
        return vc > vh;
    }
    let sha = have["sha"].as_str().unwrap_or("").to_lowercase();
    !sha.is_empty() && !entry["sha256"].as_str().unwrap_or("").starts_with(&sha)
}

/// The dashboard's view: each catalog face with `fits` (null, or why the stick can't take it) and
/// `installed` / `update`; and each installed face with its catalog entry's name, `update`, `gone`
/// (left the catalog) and `err` (didn't load on the stick).
pub fn annotate(
    catalog: Option<&Value>,
    support: Option<&Value>,
    on_stick: Option<&[Value]>,
) -> (Vec<Value>, Vec<Value>) {
    let faces: Vec<Value> = catalog.and_then(|c| c["faces"].as_array()).cloned().unwrap_or_default();
    let retired: Vec<&str> = catalog
        .and_then(|c| c["retired"].as_array())
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let on_stick = on_stick.unwrap_or(&[]);
    let find_inst = |id: &Value| on_stick.iter().find(|f| f["id"] == *id);
    let mut out = Vec::new();
    for f in &faces {
        let have = find_inst(&f["id"]);
        let mut e = f.clone();
        e["fits"] = match support {
            Some(_) => why_not(f, support).map(Value::from).unwrap_or(Value::Null),
            None => Value::Null,
        };
        e["installed"] = have.map(|h| h["v"].clone()).unwrap_or(Value::Null);
        e["update"] = json!(have.is_some_and(|h| newer(f, h)));
        out.push(e);
    }
    let mut mine = Vec::new();
    for f in on_stick {
        let c = faces.iter().find(|c| c["id"] == f["id"]);
        let update = c.is_some_and(|c| newer(c, f) && !(support.is_some() && why_not(c, support).is_some()));
        mine.push(json!({"id": f["id"], "version": f.get("v"), "size": f.get("sz"), "sha": f.get("sha"),
            "err": f.get("err"), "name": c.map(|c| c["name"].clone()).unwrap_or_else(|| f["id"].clone()),
            "author": c.and_then(|c| c.get("author").cloned()), "inCatalog": c.is_some(),
            "retired": f["id"].as_str().is_some_and(|id| retired.contains(&id)), "update": update,
            "latest": c.and_then(|c| c.get("version").cloned())}));
    }
    (out, mine)
}

// --- the catalog ------------------------------------------------------------------------------------

/// `(status, headers, body)`; 304 has an empty body.
pub type Fetched = (u16, HashMap<String, String>, Vec<u8>);

/// GETs over HTTPS (blocking; call it off the runtime). Tests answer from memory.
pub trait Fetch: Send + Sync {
    /// At most `limit` bytes (more is an error); `Err` for no connection and HTTP errors.
    fn get(&self, url: &str, headers: &[(String, String)], limit: usize) -> Result<Fetched, String>;
}

/// The real one (ureq, 15 s).
pub struct UreqFetch {
    agent: ureq::Agent,
}

impl Default for UreqFetch {
    fn default() -> Self {
        Self { agent: ureq::AgentBuilder::new().timeout(HTTP_TIMEOUT).user_agent("StickyBot").build() }
    }
}

impl Fetch for UreqFetch {
    fn get(&self, url: &str, headers: &[(String, String)], limit: usize) -> Result<Fetched, String> {
        use std::io::Read;
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        let resp = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(304, r)) => r,
            Err(ureq::Error::Status(code, _)) => return Err(format!("{url}: HTTP {code}")),
            Err(e) => return Err(format!("{url}: {e}")),
        };
        let status = resp.status();
        let hdrs: HashMap<String, String> = resp
            .headers_names()
            .into_iter()
            .filter_map(|n| resp.header(&n).map(|v| (n.clone(), v.to_string())))
            .collect();
        if status == 304 {
            return Ok((304, hdrs, Vec::new()));
        }
        let mut body = Vec::new();
        resp.into_reader().take(limit as u64 + 1).read_to_end(&mut body).map_err(|e| format!("{url}: {e}"))?;
        if body.len() > limit {
            return Err(format!("{url}: more than {limit} bytes"));
        }
        Ok((status, hdrs, body))
    }
}

/// No network (the fake stick: demos and tests never reach the Internet).
pub struct Offline;

impl Fetch for Offline {
    fn get(&self, url: &str, _: &[(String, String)], _: usize) -> Result<Fetched, String> {
        Err(format!("{url}: offline"))
    }
}

/// True if `sig_text` (base64 of a raw 64-byte ed25519 signature) is a signature of `data` by one
/// of `keys` (base64 raw public keys).
pub fn verify_signature(data: &[u8], sig_text: &str, keys: &[String]) -> bool {
    use base64::Engine;
    use ed25519_dalek::{Signature, VerifyingKey};
    let b64 = base64::engine::general_purpose::STANDARD;
    let Ok(sig) = b64.decode(sig_text.trim()) else { return false };
    let Ok(sig) = <[u8; 64]>::try_from(sig.as_slice()) else { return false };
    let sig = Signature::from_bytes(&sig);
    keys.iter().any(|k| {
        b64.decode(k)
            .ok()
            .and_then(|k| <[u8; 32]>::try_from(k.as_slice()).ok())
            .and_then(|k| VerifyingKey::from_bytes(&k).ok())
            .is_some_and(|k| k.verify_strict(data, &sig).is_ok())
    })
}

fn urljoin(base: &str, rel: &str) -> String {
    url::Url::parse(base).and_then(|b| b.join(rel)).map(|u| u.to_string()).unwrap_or_else(|_| rel.to_string())
}

fn https_or_same_host(u: &str, base: &str) -> bool {
    match (url::Url::parse(u), url::Url::parse(base)) {
        (Ok(u), Ok(b)) => {
            u.scheme() == "https"
                || (u.scheme() == b.scheme()
                    && u.host_str() == b.host_str()
                    && u.port_or_known_default() == b.port_or_known_default())
        }
        _ => false,
    }
}

/// catalog.json bytes -> the catalog with absolute URLs.
pub fn parse_catalog(data: &[u8], url: &str) -> Result<Value, String> {
    let mut cat: Value = std::str::from_utf8(data)
        .ok()
        .and_then(|t| serde_json::from_str(t).ok())
        .ok_or("the faces catalog isn't valid JSON")?;
    if !cat.is_object() || !cat["faces"].is_array() {
        return Err("the faces catalog has no list of faces".into());
    }
    if cat["catalog"].as_i64() != Some(CATALOG_VERSION) || cat["catalog"].is_boolean() {
        return Err("the faces catalog is newer than this StickyBot; update StickyBot to browse faces".into());
    }
    let mut faces = Vec::new();
    for f in cat["faces"].as_array().cloned().unwrap_or_default() {
        if !f.is_object() || !f["id"].as_str().is_some_and(is_id) {
            continue;
        }
        if !f["sha256"].as_str().is_some_and(|s| SHA_RE.is_match(s)) || !(f["size"].is_i64() || f["size"].is_u64()) {
            continue;
        }
        let mut f = f;
        f["sfb"] = json!(urljoin(url, f["sfb"].as_str().unwrap_or("")));
        if !https_or_same_host(f["sfb"].as_str().unwrap_or(""), url) {
            continue;
        }
        let mut prev = Map::new();
        if let Some(p) = f["previews"].as_object() {
            for (k, v) in p {
                if k == "portrait" || k == "landscape" {
                    let rel = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                    prev.insert(k.clone(), json!(urljoin(url, &rel)));
                }
            }
        }
        f["previews"] = Value::Object(prev);
        faces.push(f);
    }
    cat["faces"] = Value::Array(faces);
    let retired: Vec<Value> =
        cat["retired"].as_array().map(|a| a.iter().filter(|r| r.is_string()).cloned().collect()).unwrap_or_default();
    cat["retired"] = Value::Array(retired);
    Ok(cat)
}

/// The catalog cache in `<home>/faces/`: catalog.json (+ .sig, + meta.json with the validators and
/// when it was fetched), each face's `.sfb` and previews under `<id>/`. Blocking.
pub struct Catalog {
    pub url: String,
    dir: PathBuf,
    fetch: Arc<dyn Fetch>,
    clock: Arc<dyn Fn() -> f64 + Send + Sync>,
    keys: Vec<String>,
    default_url: String,
    /// the last fetch's problem, in words
    pub error: Option<String>,
}

impl Catalog {
    pub fn new(url: &str, dir: PathBuf, fetch: Arc<dyn Fetch>) -> Self {
        Self {
            url: if url.is_empty() { DEFAULT_CATALOG_URL.into() } else { url.into() },
            dir,
            fetch,
            clock: Arc::new(crate::hub::now),
            keys: CATALOG_KEYS.iter().map(|k| k.to_string()).collect(),
            default_url: DEFAULT_CATALOG_URL.into(),
            error: None,
        }
    }

    /// The signing keys and which URL must be signed (tests use their own).
    pub fn with_keys(mut self, keys: Vec<String>, default_url: &str) -> Self {
        self.keys = keys;
        self.default_url = default_url.into();
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Fn() -> f64 + Send + Sync>) -> Self {
        self.clock = clock;
        self
    }

    pub fn signed_required(&self) -> bool {
        self.url == self.default_url
    }

    fn dir(&self) -> PathBuf {
        let _ = std::fs::create_dir_all(&self.dir);
        self.dir.clone()
    }

    fn meta(&self) -> Map<String, Value> {
        match std::fs::read_to_string(self.dir().join("meta.json")).ok().and_then(|t| serde_json::from_str(&t).ok()) {
            Some(Value::Object(m)) if m.get("url").and_then(Value::as_str) == Some(self.url.as_str()) => m,
            _ => Map::new(),
        }
    }

    fn save_meta(&self, meta: &Value) {
        if let Err(e) = std::fs::write(self.dir().join("meta.json"), meta.to_string()) {
            log::warn!("faces catalog: {e}");
        }
    }

    /// The cached catalog (`{"catalog", "fetched", "signed"}`) or `None` (none, another URL, or
    /// it no longer verifies).
    pub fn cached(&self) -> Option<Value> {
        let meta = self.meta();
        if meta.is_empty() {
            return None;
        }
        let data = std::fs::read(self.dir().join("catalog.json")).ok()?;
        let sig = std::fs::read_to_string(self.dir().join("catalog.json.sig")).unwrap_or_default();
        let signed = !sig.is_empty() && verify_signature(&data, &sig, &self.keys);
        if self.signed_required() && !signed {
            return None;
        }
        let cat = parse_catalog(&data, &self.url).ok()?;
        Some(json!({"catalog": cat, "fetched": meta.get("fetched").cloned().unwrap_or(Value::Null), "signed": signed}))
    }

    /// Fetches the catalog if it's due (or `force`), keeping the last good copy on any problem.
    /// Returns [`Catalog::cached`] (or `None`) and sets `self.error`.
    pub fn refresh(&mut self, force: bool) -> Option<Value> {
        let meta = self.meta();
        let have = self.cached();
        let now = (self.clock)();
        let fetched = meta.get("fetched").and_then(Value::as_f64).unwrap_or(0.0);
        if have.is_some() && !force && now - fetched < REFRESH_S {
            self.error = None;
            return have;
        }
        let mut headers = Vec::new();
        if have.is_some() {
            if let Some(e) = meta.get("etag").and_then(Value::as_str) {
                headers.push(("If-None-Match".to_string(), e.to_string()));
            }
            if let Some(m) = meta.get("modified").and_then(Value::as_str) {
                headers.push(("If-Modified-Since".to_string(), m.to_string()));
            }
        }
        match self.fetch_new(&headers, have.is_some(), meta) {
            Ok(v) => {
                self.error = None;
                v.or(have)
            }
            Err((true, e)) => {
                self.error = Some(e);
                have
            }
            Err((false, e)) => {
                log::info!("faces catalog: {e}");
                let last = if have.is_some() { " (showing the last copy)" } else { "" };
                self.error = Some(format!("can't reach the faces catalog right now{last}"));
                have
            }
        }
    }

    /// `Err((true, words))` for a catalog problem, `Err((false, why))` for a network one.
    fn fetch_new(
        &self,
        headers: &[(String, String)],
        have: bool,
        mut meta: Map<String, Value>,
    ) -> Result<Option<Value>, (bool, String)> {
        let (status, hdrs, data) = self.fetch.get(&self.url, headers, MAX_CATALOG).map_err(|e| (false, e))?;
        if status == 304 && have {
            meta.insert("fetched".into(), json!((self.clock)()));
            self.save_meta(&Value::Object(meta));
            return Ok(self.cached());
        }
        if status != 200 {
            return Err((false, format!("HTTP {status}")));
        }
        let cat = parse_catalog(&data, &self.url).map_err(|e| (true, e))?;
        let sig = match self.fetch.get(&format!("{}.sig", self.url), &[], 1024) {
            Ok((200, _, s)) => String::from_utf8_lossy(&s).into_owned(),
            _ => String::new(),
        };
        let signed = !sig.is_empty() && verify_signature(&data, &sig, &self.keys);
        if self.signed_required() && !signed {
            return Err((true, "the faces catalog's signature doesn't check out, so it was ignored".into()));
        }
        let d = self.dir();
        std::fs::write(d.join("catalog.json"), &data).map_err(|e| (false, e.to_string()))?;
        let sigf = d.join("catalog.json.sig");
        if sig.is_empty() {
            let _ = std::fs::remove_file(&sigf);
        } else {
            std::fs::write(&sigf, &sig).map_err(|e| (false, e.to_string()))?;
        }
        let low: HashMap<String, String> = hdrs.into_iter().map(|(k, v)| (k.to_lowercase(), v)).collect();
        let now = (self.clock)();
        let modified =
            low.get("last-modified").cloned().unwrap_or_else(|| httpdate::fmt_http_date(std::time::SystemTime::now()));
        self.save_meta(&json!({"url": self.url, "fetched": now, "etag": low.get("etag"), "modified": modified}));
        log::info!(
            "faces catalog: {} faces ({})",
            cat["faces"].as_array().map_or(0, Vec::len),
            if signed { "signed" } else { "unsigned" }
        );
        Ok(Some(json!({"catalog": cat, "fetched": now, "signed": signed})))
    }

    pub fn entry(&self, fid: &str) -> Option<Value> {
        let have = self.cached()?;
        have["catalog"]["faces"].as_array()?.iter().find(|f| f["id"] == fid).cloned()
    }

    /// The entry's `.sfb`, verified (size, sha256, container, id / version), from the cache or
    /// the catalog.
    pub fn download(&self, entry: &Value) -> Result<Vec<u8>, String> {
        let fid = entry["id"].as_str().unwrap_or("");
        if !is_id(fid) {
            return Err("that isn't a face id".into());
        }
        let version = entry["version"].as_str().unwrap_or("?");
        let sha = entry["sha256"].as_str().unwrap_or("");
        let d = self.dir().join(fid);
        let path = d.join(format!("{fid}-{version}.sfb"));
        let mut blob = std::fs::read(&path).ok().filter(|b| sha_hex(b) == sha);
        if blob.is_none() {
            let (status, _, b) = self.fetch.get(entry["sfb"].as_str().unwrap_or(""), &[], MAX_SFB).map_err(|e| {
                log::info!("face download: {e}");
                "couldn't download the face; check the internet connection and try again".to_string()
            })?;
            if status != 200 {
                return Err(format!("couldn't download the face (HTTP {status}); try again later"));
            }
            blob = Some(b);
        }
        let blob = blob.unwrap_or_default();
        if Some(blob.len() as u64) != entry["size"].as_u64() || sha_hex(&blob) != sha {
            return Err("the download was damaged (it doesn't match the catalog); try again".into());
        }
        let info = sfb_info(&blob)?;
        if info["id"] != fid || info["version"] != entry["version"] {
            return Err("the downloaded face isn't the one the catalog describes; try again later".into());
        }
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        for old in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let n = old.file_name().to_string_lossy().to_string();
            if n.starts_with(&format!("{fid}-")) && n.ends_with(".sfb") && old.path() != path {
                let _ = std::fs::remove_file(old.path());
            }
        }
        std::fs::write(&path, &blob).map_err(|e| e.to_string())?;
        Ok(blob)
    }

    /// A face's preview PNG, cached by version; `None` if there's none.
    pub fn preview(&self, fid: &str, side: &str) -> Option<Vec<u8>> {
        if !matches!(side, "portrait" | "landscape") || !is_id(fid) {
            return None;
        }
        let entry = self.entry(fid);
        let d = self.dir().join(fid);
        let prefix = format!("{side}-");
        let path = match &entry {
            Some(e) => Some(d.join(format!("{side}-{}.png", e["version"].as_str().unwrap_or("0")))),
            None => {
                // left the catalog: whatever we have
                let mut found: Vec<PathBuf> = std::fs::read_dir(&d)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        n.starts_with(&prefix) && n.ends_with(".png")
                    })
                    .collect();
                found.sort();
                found.pop()
            }
        };
        if let Some(p) = &path
            && let Ok(b) = std::fs::read(p)
        {
            return Some(b);
        }
        let url = entry.as_ref()?["previews"][side].as_str()?.to_string();
        let (status, _, data) = self.fetch.get(&url, &[], MAX_PREVIEW).ok()?;
        if status != 200 || !data.starts_with(b"\x89PNG\r\n\x1a\n") {
            return None;
        }
        let _ = std::fs::create_dir_all(&d);
        for old in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let n = old.file_name().to_string_lossy().to_string();
            if n.starts_with(&prefix) && n.ends_with(".png") {
                let _ = std::fs::remove_file(old.path());
            }
        }
        if let Some(p) = path {
            let _ = std::fs::write(p, &data);
        }
        Some(data)
    }
}

// --- the stick: install, list, remove, use ------------------------------------------------------

/// The `faces` message family for one face (FORMAT.md §18: `faces put` … `done`).
pub fn family(id: &str) -> Family {
    let id = id.to_string();
    Family {
        key: "faces",
        begin: Arc::new(
            move |img: &Image, _force| json!({"put": {"id": id, "version": img.version, "size": img.size(), "sha256": img.sha256}}),
        ),
        reason,
        words: Words {
            no_answer: "the stick didn't answer the install request; update the stick to 2.4.0 to add faces",
            stalled: "the stick stopped answering during the install",
            unconfirmed: "the stick didn't confirm the face",
        },
    }
}

/// A sender for one face.
pub fn sender(link: &Link<Backend>, emit: Emit, id: &str, timings: Timings) -> Sender<Backend> {
    let mut s = Sender::with_family(link.clone(), emit, family(id));
    s.timings = timings;
    s
}

/// Checks `blob` (a compiled face; against `entry` too when it's from the catalog) and what the
/// stick takes, then sends it and waits for `done`. Returns `{"id","version","name",…stats}`.
pub async fn install(
    link: &Link<Backend>,
    blob: Vec<u8>,
    entry: Option<&Value>,
    emit: Emit,
    sender: &mut Sender<Backend>,
) -> Result<Value, String> {
    if !link.connected() {
        return Err("the stick isn't connected by Bluetooth".into());
    }
    let info = sfb_info(&blob)?;
    if let Some(e) = entry {
        if info["sha256"] != e["sha256"] || info["size"] != e["size"] {
            return Err("the face doesn't match the catalog (damaged download); try again".into());
        }
        if info["id"] != e["id"] || info["version"] != e["version"] {
            return Err("the face file isn't the one the catalog describes".into());
        }
    }
    let st = link.request("@state").await.ok_or("the stick didn't answer; check it's on and connected")?;
    if let Some(why) = why_not(&info, Some(&stick_support(&st))) {
        return Err(why);
    }
    let s = |k: &str| info[k].as_str().unwrap_or("").to_string();
    let (id, name, version) = (s("id"), s("name"), s("version"));
    let stick = st.get("name").and_then(Value::as_str).unwrap_or("the stick");
    emit(json!({"line": format!("installing {name} {version} ({} bytes) on {stick}", blob.len())}));
    let img = Image::new(blob, &version);
    let stats = sender.run(&img, false).await.map_err(|e| match e {
        OtaError::Failed(m) | OtaError::Cancelled(m) => m,
    })?;
    let done = sender.done.clone().unwrap_or(Value::Null);
    if let Some(d) = done.get("id").and_then(Value::as_str).filter(|d| !d.is_empty())
        && d != id
    {
        return Err(format!("the stick confirmed {d}, not {id}"));
    }
    emit(json!({"line": format!("{name} is on the stick")}));
    let mut r = stats.to_json();
    r["id"] = json!(id);
    r["version"] =
        done.get("version").filter(|v| v.as_str().is_some_and(|v| !v.is_empty())).cloned().unwrap_or(json!(version));
    r["name"] = json!(name);
    Ok(r)
}

/// The faces on the stick from `@faces` (all pages): `{"faces": [{"id","v","sz","sha","err"?}],
/// "free", "max"}`.
pub async fn installed(link: &Link<Backend>) -> Result<Value, String> {
    let mut out = Vec::new();
    let mut extra = Map::new();
    for page in 0..MAX_PAGES {
        let line = if page == 0 { "@faces".to_string() } else { format!("@faces page={page}") };
        let r = link.request(&line).await;
        let ok = r.as_ref().is_some_and(|r| r["ok"] != false && r["faces"].is_array());
        let r = match r {
            Some(r) if ok => r,
            _ if page > 0 => break,
            r => {
                let why = r.as_ref().and_then(|r| r["error"].as_str()).map(|e| format!(" ({e})")).unwrap_or_default();
                return Err(format!("the stick didn't list its faces{why}"));
            }
        };
        out.extend(
            r["faces"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|f| f["id"].as_str().is_some_and(|i| !i.is_empty()))
                .cloned(),
        );
        extra = ["free", "max"].iter().filter_map(|k| r.get(*k).map(|v| (k.to_string(), v.clone()))).collect();
        if r["more"] != true {
            break;
        }
    }
    let mut v = json!({"faces": out});
    for (k, x) in extra {
        v[k] = x;
    }
    Ok(v)
}

/// Sends a `faces` message and returns the stick's first `{"evt":"faces"}` answer whose state is
/// in `states` (`None` on no answer).
async fn request(link: &Link<Backend>, msg: Value, states: &[&str], timeout: Duration) -> Option<Value> {
    let mut rx = link.messages();
    if !link.send(&json!({"src": "agent", "faces": msg})).await {
        return None;
    }
    tokio::time::timeout(timeout, async {
        loop {
            match rx.recv().await {
                Ok(m) => {
                    if m["src"] == "agent"
                        && m["evt"] == "faces"
                        && m["state"].as_str().is_some_and(|s| states.contains(&s))
                    {
                        return Some(m);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
}

/// Removes a face from the stick (`faces rm`).
pub async fn remove(link: &Link<Backend>, fid: &str, timeout: Duration) -> Result<Value, String> {
    if !is_id(fid) {
        return Err("that isn't a face id".into());
    }
    if !link.connected() {
        return Err("the stick isn't connected by Bluetooth".into());
    }
    let r = request(link, json!({"rm": fid}), &["removed", "refused", "error"], timeout)
        .await
        .ok_or("the stick didn't answer; check it's on and connected")?;
    if r["state"] != "removed" {
        return Err(reason(&r));
    }
    Ok(json!({"id": r.get("id").filter(|i| i.is_string()).cloned().unwrap_or(json!(fid))}))
}

/// Makes it the Home face (`@face id=`).
pub async fn use_face(link: &Link<Backend>, fid: &str) -> Result<Value, String> {
    if !is_id(fid) {
        return Err("that isn't a face id".into());
    }
    let r =
        link.request(&format!("@face id={fid}")).await.ok_or("the stick didn't answer; check it's on and connected")?;
    if r["ok"] == false || r.get("error").is_some_and(|e| !e.is_null() && e != "") {
        let why = r["error"].as_str().unwrap_or("it refused");
        return Err(format!("the stick didn't switch: {why}"));
    }
    Ok(json!({"id": fid}))
}

// --- the app's side: state, jobs, the API (faces_api.py) ------------------------------------------

/// What [`AppState`] keeps for faces.
pub struct Faces {
    /// an install owns the OTA characteristic (the firmware update checks it)
    busy: AtomicBool,
    pub jobs: Jobs,
    fetch: Mutex<Arc<dyn Fetch>>,
    keys: Mutex<Vec<String>>,
    default_url: Mutex<String>,
    pub timings: Mutex<Timings>,
    pub reply: Mutex<Duration>,
}

impl Faces {
    /// `offline`: never reach the network (the fake stick).
    pub fn new(offline: bool) -> Self {
        let fetch: Arc<dyn Fetch> = if offline { Arc::new(Offline) } else { Arc::new(UreqFetch::default()) };
        Self {
            busy: AtomicBool::new(false),
            jobs: Jobs::default(),
            fetch: Mutex::new(fetch),
            keys: Mutex::new(CATALOG_KEYS.iter().map(|k| k.to_string()).collect()),
            default_url: Mutex::new(DEFAULT_CATALOG_URL.into()),
            timings: Mutex::new(Timings::default()),
            reply: Mutex::new(REPLY),
        }
    }

    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::Relaxed)
    }

    pub fn set_busy(&self, b: bool) {
        self.busy.store(b, Ordering::Relaxed);
    }

    /// Tests: another web, other signing keys and which URL must be signed.
    pub fn set_fetch(&self, f: Arc<dyn Fetch>) {
        *self.fetch.lock().unwrap() = f;
    }

    pub fn set_keys(&self, keys: Vec<String>, default_url: &str) {
        *self.keys.lock().unwrap() = keys;
        *self.default_url.lock().unwrap() = default_url.into();
    }

    /// The catalog of config `faces_catalog_url`.
    pub fn catalog(&self, st: &AppState) -> Catalog {
        let url = st.cfg.lock().unwrap().get("faces_catalog_url").and_then(Value::as_str).unwrap_or("").to_string();
        let keys = self.keys.lock().unwrap().clone();
        let default_url = self.default_url.lock().unwrap().clone();
        Catalog::new(&url, st.paths.home.join("faces"), self.fetch.lock().unwrap().clone())
            .with_keys(keys, &default_url)
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/faces/catalog", get(get_catalog))
        .route("/api/faces/preview/{id}/{side}", get(preview))
        .route("/api/faces/install", post(install_api))
        .route("/api/faces/jobs/{id}", get(job_events))
        .route("/api/faces/jobs/{id}/cancel", post(job_cancel))
        .route("/api/faces/remove", post(remove_api))
        .route("/api/faces/use", post(use_api))
}

/// An error with its first letter capitalized (faces_api.py `err`).
fn err(text: &str, status: StatusCode) -> Response {
    let mut c = text.chars();
    let t = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
    (status, axum::Json(json!({"error": t}))).into_response()
}

fn body_obj(b: &Bytes) -> Map<String, Value> {
    match serde_json::from_slice::<Value>(b) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

#[derive(serde::Deserialize)]
struct RefreshQuery {
    refresh: Option<String>,
}

/// The catalog (cached; works offline), what the stick has and takes:
/// `{"url","faces","installed","stick","signed","fetched","generated","error","offline"}`.
async fn get_catalog(State(st): State<Arc<AppState>>, Query(q): Query<RefreshQuery>) -> axum::Json<Value> {
    let mut cat = st.faces.catalog(&st);
    let force = q.refresh.as_deref() == Some("1");
    let (have, cat) = tokio::task::spawn_blocking(move || (cat.refresh(force), cat)).await.expect("catalog task");
    let connected = st.link.connected();
    let mut info = json!({"connected": connected, "supported": false, "fw": null, "needs": MIN_FIRMWARE,
                          "n": 0, "max": null, "free": null, "error": null, "busy": st.faces.busy()});
    let (mut support, mut on_stick) = (None, None);
    if connected && !st.usb.is_flashing() {
        match st.link.request("@state").await {
            Some(s) => {
                let sup = stick_support(&s);
                for k in ["supported", "fw", "fmt", "n"] {
                    info[k] = sup[k].clone();
                }
                if sup["supported"] == true {
                    match installed(&st.link).await {
                        Ok(l) => {
                            let faces = l["faces"].as_array().cloned().unwrap_or_default();
                            info["n"] = json!(faces.len());
                            info["max"] = l.get("max").cloned().unwrap_or(Value::Null);
                            info["free"] = l.get("free").cloned().unwrap_or(Value::Null);
                            on_stick = Some(faces);
                        }
                        Err(e) => info["error"] = json!(e),
                    }
                } else {
                    info["error"] = json!(needs_firmware_text(sup["fw"].as_str()));
                }
                support = Some(sup);
            }
            None => info["error"] = json!("the stick didn't answer"),
        }
    }
    let catalog = have.as_ref().map(|h| &h["catalog"]);
    let (mut listed, mine) = annotate(catalog, support.as_ref(), on_stick.as_deref());
    for f in &mut listed {
        // previews come through this app (cached, and offline)
        let id = f["id"].as_str().unwrap_or("").to_string();
        let sides: Vec<String> = f["previews"].as_object().map(|p| p.keys().cloned().collect()).unwrap_or_default();
        f["previews"] = sides.iter().map(|k| (k.clone(), json!(format!("/api/faces/preview/{id}/{k}")))).collect();
    }
    let h = |k: &str| have.as_ref().map(|h| h[k].clone()).unwrap_or(Value::Null);
    axum::Json(json!({
        "url": cat.url, "faces": listed, "installed": if on_stick.is_some() { json!(mine) } else { Value::Null },
        "stick": info, "signed": h("signed") == true, "fetched": h("fetched"),
        "generated": have.as_ref().map(|h| h["catalog"]["generated"].clone()).unwrap_or(Value::Null),
        "error": cat.error, "offline": cat.error.is_some() && have.is_some(),
    }))
}

async fn preview(State(st): State<Arc<AppState>>, UrlPath((id, side)): UrlPath<(String, String)>) -> Response {
    let cat = st.faces.catalog(&st);
    match tokio::task::spawn_blocking(move || cat.preview(&id, &side)).await.ok().flatten() {
        Some(data) => {
            ([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "max-age=3600")], data).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `{"id"}` (from the catalog) or `{"file": base64, "name"}` -> `{"job"}`.
async fn install_api(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let Ok(Value::Object(body)) = serde_json::from_slice::<Value>(&body) else {
        return err("expected JSON", StatusCode::BAD_REQUEST);
    };
    if st.faces.busy() || st.faces.jobs.busy() {
        return err("a face is already being installed", StatusCode::CONFLICT);
    }
    if st.usb.is_flashing() || st.jobs.busy() {
        return err("the stick's firmware is being updated; try again when it's done", StatusCode::CONFLICT);
    }
    if !st.link.connected() {
        return err("the stick isn't connected by Bluetooth", StatusCode::CONFLICT);
    }
    let mut cat = st.faces.catalog(&st);
    let (entry, blob, id) = if body.get("file").is_some_and(|f| !f.is_null() && f != "" && f != false) {
        use base64::Engine;
        let text = body["file"].as_str().map(str::to_string).unwrap_or_else(|| body["file"].to_string());
        let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(text.trim()) else {
            return err("couldn't read the file", StatusCode::BAD_REQUEST);
        };
        let name = body.get("name").and_then(Value::as_str).unwrap_or("");
        let checked = read_upload(raw, name).and_then(|b| sfb_info(&b).map(|i| (b, i)));
        match checked {
            Ok((b, info)) => (None, Some(b), info["id"].as_str().unwrap_or("").to_string()),
            Err(e) => return err(&e, StatusCode::BAD_REQUEST),
        }
    } else {
        let fid =
            body.get("id").map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).unwrap_or_default();
        let fid = if fid == "null" { String::new() } else { fid };
        let f2 = fid.clone();
        let (entry, c) = tokio::task::spawn_blocking(move || {
            let e = cat.entry(&f2).or_else(|| cat.refresh(false).and_then(|_| cat.entry(&f2)));
            (e, cat)
        })
        .await
        .expect("catalog task");
        cat = c;
        match entry {
            Some(e) => (Some(e), None, fid),
            None => {
                let what = if fid.is_empty() { "that face".to_string() } else { fid };
                return err(&format!("{what} isn't in the faces catalog"), StatusCode::NOT_FOUND);
            }
        }
    };
    let job = Job::new();
    st.faces.jobs.add(job.clone());
    let j = job.clone();
    let emit: Emit = Arc::new(move |e| j.emit(e));
    let timings = st.faces.timings.lock().unwrap().clone();
    let mut sender = sender(&st.link, emit.clone(), &id, timings);
    job.set_cancel(Some(sender.cancel_handle()));
    st.faces.set_busy(true);
    let (st2, j) = (st.clone(), job.clone());
    tokio::spawn(async move {
        let what = entry.as_ref().and_then(|e| e["id"].as_str()).unwrap_or("(file)").to_string();
        let r = async {
            let data = match (&entry, blob) {
                (Some(e), _) => {
                    emit(json!({"progress": 0, "stage": "download"}));
                    let e2 = e.clone();
                    let data = tokio::task::spawn_blocking(move || cat.download(&e2))
                        .await
                        .map_err(|e| format!("unexpected error: {e}"))??;
                    emit(json!({"progress": 100, "stage": "download"}));
                    data
                }
                (None, Some(b)) => b,
                (None, None) => return Err("nothing to install".to_string()),
            };
            install(&st2.link, data, entry.as_ref(), emit.clone(), &mut sender).await
        }
        .await;
        st2.faces.set_busy(false);
        j.set_cancel(None);
        match &r {
            Ok(_) => log::info!("face install {what}: ok"),
            Err(e) => log::info!("face install {what}: {e}"),
        }
        let (ok, error, result) = match r {
            Ok(v) => (true, Value::Null, v),
            Err(e) => {
                let mut c = e.chars();
                let cap = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
                (false, json!(cap), Value::Null)
            }
        };
        j.finish(json!({"done": true, "ok": ok, "error": error, "result": result}));
    });
    axum::Json(json!({"job": job.id})).into_response()
}

async fn job_events(State(st): State<Arc<AppState>>, UrlPath(id): UrlPath<String>) -> Response {
    match st.faces.jobs.get(&id) {
        Some(job) => crate::api::stream_job(job),
        None => err("no such job", StatusCode::NOT_FOUND),
    }
}

async fn job_cancel(State(st): State<Arc<AppState>>, UrlPath(id): UrlPath<String>) -> Response {
    let Some(job) = st.faces.jobs.get(&id) else {
        return err("no such job", StatusCode::NOT_FOUND);
    };
    if job.cancel() {
        axum::Json(json!({"ok": true})).into_response()
    } else {
        err("nothing to cancel", StatusCode::CONFLICT)
    }
}

fn id_of(b: &Map<String, Value>) -> String {
    b.get("id").and_then(Value::as_str).unwrap_or("").to_string()
}

async fn remove_api(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let b = body_obj(&body);
    if st.faces.busy() {
        return err("a face is being installed; try again when it's done", StatusCode::CONFLICT);
    }
    let timeout = *st.faces.reply.lock().unwrap();
    match remove(&st.link, &id_of(&b), timeout).await {
        Ok(r) => axum::Json(json!({"ok": true, "id": r["id"]})).into_response(),
        Err(e) => err(&e, StatusCode::CONFLICT),
    }
}

async fn use_api(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let b = body_obj(&body);
    match use_face(&st.link, &id_of(&b)).await {
        Ok(r) => axum::Json(json!({"ok": true, "id": r["id"]})).into_response(),
        Err(e) => err(&e, StatusCode::CONFLICT),
    }
}

#[cfg(test)]
mod tests;
