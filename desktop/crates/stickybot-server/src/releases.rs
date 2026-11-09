//! StickyBot's releases on GitHub (a port of `releases.py`): versions (SemVer precedence), the
//! latest release per channel, and a release's firmware downloaded into
//! `~/.stickybot-rs/firmware/<firmware version>/`.
//!
//! Releases are listed with the `gh` CLI when it's installed (the user's login: higher rate limits,
//! private forks), else over plain HTTPS from api.github.com. Every firmware part is checked
//! against the sha256 in the release's `manifest.json` (and GitHub's own asset digest, when it
//! gives one) before it lands in place; a download goes to a temporary folder first, so a
//! half-finished one is never used. The sources are behind [`Source`], so tests use a fake.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const DEFAULT_REPO: &str = "ashish-kumar-noon/stickybot";
pub const API: &str = "https://api.github.com";
pub const CHANNELS: &[&str] = &["stable", "beta"];
pub const MANIFEST: &str = "manifest.json";
/// firmware versions kept in `firmware/` (plus the local build)
pub const KEEP: usize = 3;
pub const NO_RELEASE: &str = "no release yet — build the firmware locally: make -C m5-stick-s3 release";

/// `$STICKYBOT_RS_REPO`, else StickyBot's own.
pub fn repo() -> String {
    std::env::var("STICKYBOT_RS_REPO").ok().filter(|r| !r.is_empty()).unwrap_or_else(|| DEFAULT_REPO.into())
}

// --- versions ------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Ident {
    // numbers sort before words (SemVer 11.4.3)
    Num(u64),
    Word(String),
}

/// A SemVer sort key: 1.8.0-beta.2 < 1.8.0-beta.10 < 1.8.0-rc.1 < 1.8.0. Unparsable sorts first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionKey(Option<((u64, u64, u64), Vec<Ident>)>);

impl Ord for VersionKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.0, &other.0) {
            (None, None) => Ordering::Equal,
            (None, _) => Ordering::Less,
            (_, None) => Ordering::Greater,
            (Some((a, pa)), Some((b, pb))) => a.cmp(b).then_with(|| match (pa.is_empty(), pb.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater, // a release sorts after its pre-releases
                (false, true) => Ordering::Less,
                (false, false) => pa.cmp(pb), // and a longer list wins a tie (Vec's order)
            }),
        }
    }
}

impl PartialOrd for VersionKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// `v1.8.0-beta.2+build` -> (1, 8, 0) and `["beta", 2]`; `None` if it isn't a version.
fn parse(v: &str) -> Option<((u64, u64, u64), Vec<Ident>)> {
    let v = v.trim();
    let v = v.strip_prefix('v').unwrap_or(v);
    let v = v.split_once('+').map(|(a, b)| (a, Some(b))).unwrap_or((v, None));
    if let Some(build) = v.1
        && (build.is_empty() || !build.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
    {
        return None;
    }
    let (core, pre) = match v.0.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v.0, None),
    };
    let nums: Vec<u64> = core
        .split('.')
        .map(|p| if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) { p.parse().ok() } else { None })
        .collect::<Option<_>>()?;
    let [a, b, c] = nums[..] else { return None };
    let mut ids = Vec::new();
    if let Some(pre) = pre {
        if pre.is_empty() || !pre.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
            return None;
        }
        for id in pre.split('.') {
            ids.push(match id.parse::<u64>() {
                Ok(n) if id.chars().all(|c| c.is_ascii_digit()) => Ident::Num(n),
                _ => Ident::Word(id.to_string()),
            });
        }
    }
    Some(((a, b, c), ids))
}

pub fn is_version(v: &str) -> bool {
    parse(v).is_some()
}

pub fn version_key(v: &str) -> VersionKey {
    VersionKey(parse(v))
}

pub fn is_prerelease(v: &str) -> bool {
    parse(v).is_some_and(|(_, pre)| !pre.is_empty())
}

/// `a` is newer than `b` (false if `a` isn't a version; anything beats a missing `b`).
pub fn newer(a: Option<&str>, b: Option<&str>) -> bool {
    let Some(a) = a.filter(|a| is_version(a)) else { return false };
    match b.filter(|b| is_version(b)) {
        None => true,
        Some(b) => version_key(a) > version_key(b),
    }
}

// --- releases --------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ReleaseError {
    /// the repo has no (matching) release yet
    NoRelease(String),
    Failed(String),
}

impl std::fmt::Display for ReleaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReleaseError::NoRelease(m) | ReleaseError::Failed(m) => f.write_str(m),
        }
    }
}

fn fail(m: impl Into<String>) -> ReleaseError {
    ReleaseError::Failed(m.into())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub size: Option<u64>,
    pub download: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub tag: String,
    pub version: String,
    pub prerelease: bool,
    pub name: String,
    pub notes: String,
    pub url: Option<String>,
    pub published: Option<String>,
    pub assets: BTreeMap<String, Asset>,
}

impl Release {
    /// What the dashboard and the state file keep (updates.py `summary`).
    pub fn summary(&self) -> Value {
        json!({"tag": self.tag, "version": self.version, "prerelease": self.prerelease, "name": self.name,
               "notes": self.notes, "url": self.url, "published": self.published})
    }
}

/// Asset / folder names we'll write.
pub fn safe_name(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_ascii_alphanumeric())
        && s.len() <= 81
        && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// A release from the API -> the fields we use (`None` for drafts and odd tags).
pub fn normalize(r: &Value) -> Option<Release> {
    if r.get("draft").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let tag = r.get("tag_name").and_then(Value::as_str)?.to_string();
    let version = tag.strip_prefix('v').unwrap_or(&tag).to_string();
    if !is_version(&version) {
        return None;
    }
    let mut assets = BTreeMap::new();
    for a in r.get("assets").and_then(Value::as_array).into_iter().flatten() {
        let name = a.get("name").and_then(Value::as_str).unwrap_or("");
        if safe_name(name) {
            let digest = a.get("digest").and_then(Value::as_str).unwrap_or("");
            assets.insert(
                name.to_string(),
                Asset {
                    size: a.get("size").and_then(Value::as_u64),
                    download: a.get("browser_download_url").and_then(Value::as_str).map(str::to_string),
                    sha256: digest.strip_prefix("sha256:").map(str::to_string),
                },
            );
        }
    }
    let s = |k: &str| r.get(k).and_then(Value::as_str).map(str::to_string);
    Some(Release {
        prerelease: r.get("prerelease").and_then(Value::as_bool).unwrap_or(false) || is_prerelease(&version),
        name: s("name").filter(|n| !n.is_empty()).unwrap_or_else(|| tag.clone()),
        notes: s("body").unwrap_or_default(),
        url: s("html_url"),
        published: s("published_at"),
        tag,
        version,
        assets,
    })
}

/// The newest release for the channel: "stable" = no prereleases, "beta" = everything.
pub fn pick(raw: &[Value], channel: &str) -> Option<Release> {
    raw.iter()
        .filter_map(normalize)
        .filter(|r| channel == "beta" || !r.prerelease)
        .max_by(|a, b| version_key(&a.version).cmp(&version_key(&b.version)))
}

/// A way to reach GitHub.
pub trait Source: Send + Sync {
    fn name(&self) -> &str;
    /// `GET /<path>` of the API as JSON.
    fn json(&self, path: &str) -> Result<Value, ReleaseError>;
    /// One asset of the release into `dest`.
    fn download(&self, rel: &Release, name: &str, dest: &Path) -> Result<(), ReleaseError>;
}

/// The GitHub CLI with the user's own login.
pub struct Gh {
    pub gh: PathBuf,
    pub repo: String,
}

fn tail(s: &str, n: usize) -> String {
    let s = s.trim();
    let start = s.char_indices().rev().nth(n.saturating_sub(1)).map(|(i, _)| i).unwrap_or(0);
    s[start..].to_string()
}

impl Source for Gh {
    fn name(&self) -> &str {
        "gh"
    }
    fn json(&self, path: &str) -> Result<Value, ReleaseError> {
        let out = Command::new(&self.gh)
            .args(["api", "-H", "Accept: application/vnd.github+json", path])
            .output()
            .map_err(|e| fail(format!("gh api {path}: {e}")))?;
        if !out.status.success() {
            let msg = String::from_utf8_lossy(if out.stderr.is_empty() { &out.stdout } else { &out.stderr });
            return Err(fail(format!("gh api {path}: {}", tail(&msg, 200))));
        }
        serde_json::from_slice(&out.stdout).map_err(|e| fail(format!("gh api {path}: not JSON ({e})")))
    }
    fn download(&self, rel: &Release, name: &str, dest: &Path) -> Result<(), ReleaseError> {
        let dir = dest.parent().ok_or_else(|| fail("bad destination"))?;
        let out = Command::new(&self.gh)
            .args(["release", "download", &rel.tag, "--repo", &self.repo, "--pattern", name, "--dir"])
            .arg(dir)
            .arg("--clobber")
            .output()
            .map_err(|e| fail(format!("gh release download {name}: {e}")))?;
        if !out.status.success() {
            return Err(fail(format!("gh release download {name}: {}", tail(&String::from_utf8_lossy(&out.stderr), 200))));
        }
        if dest.file_name().is_some_and(|f| f != name) {
            std::fs::rename(dir.join(name), dest).map_err(|e| fail(e.to_string()))?;
        }
        Ok(())
    }
}

/// api.github.com without a login.
pub struct Https {
    pub repo: String,
}

impl Https {
    fn get(&self, url: &str, accept: &str) -> Result<ureq::Response, ReleaseError> {
        ureq::get(url)
            .set("Accept", accept)
            .set("User-Agent", "StickyBot")
            .set("X-GitHub-Api-Version", "2022-11-28")
            .timeout(std::time::Duration::from_secs(60))
            .call()
            .map_err(|e| match e {
                ureq::Error::Status(code, _) => fail(format!(
                    "{url}: HTTP {code}{}",
                    if code == 404 { " (private repo? log in with `gh auth login`)" } else { "" }
                )),
                e => fail(format!("{url}: {e}")),
            })
    }
}

impl Source for Https {
    fn name(&self) -> &str {
        "https"
    }
    fn json(&self, path: &str) -> Result<Value, ReleaseError> {
        let r = self.get(&format!("{API}/{path}"), "application/vnd.github+json")?;
        let mut body = Vec::new();
        r.into_reader().take(32 << 20).read_to_end(&mut body).map_err(|e| fail(format!("{path}: {e}")))?;
        serde_json::from_slice(&body).map_err(|e| fail(format!("{path}: not JSON ({e})")))
    }
    fn download(&self, rel: &Release, name: &str, dest: &Path) -> Result<(), ReleaseError> {
        let asset = rel.assets.get(name);
        let url = asset
            .and_then(|a| a.download.clone())
            .ok_or_else(|| fail(format!("release {} has no {name}", rel.tag)))?;
        let limit = asset.and_then(|a| a.size).unwrap_or(64 << 20);
        let r = self.get(&url, "application/octet-stream")?;
        let mut body = Vec::new();
        r.into_reader().take(limit + 1).read_to_end(&mut body).map_err(|e| fail(format!("{name}: {e}")))?;
        if body.len() as u64 > limit {
            return Err(fail(format!("{name} is bigger than the release says")));
        }
        std::fs::write(dest, body).map_err(|e| fail(format!("{}: {e}", dest.display())))
    }
}

fn gh_path() -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join("gh")).collect::<Vec<_>>())
        .unwrap_or_default();
    on_path.into_iter().chain(["/opt/homebrew/bin/gh".into(), "/usr/local/bin/gh".into()]).find(|p| p.is_file())
}

/// The ways to reach GitHub, best first: gh (when installed), then plain HTTPS.
pub fn sources(repo: &str) -> Vec<Box<dyn Source>> {
    let mut out: Vec<Box<dyn Source>> = Vec::new();
    if let Some(gh) = gh_path() {
        out.push(Box::new(Gh { gh, repo: repo.into() }));
    }
    out.push(Box::new(Https { repo: repo.into() }));
    out
}

/// The latest release for the channel and the source that answered.
pub fn latest<'a>(srcs: &'a [Box<dyn Source>], repo: &str, channel: &str) -> Result<(Release, &'a dyn Source), ReleaseError> {
    let mut errors = Vec::new();
    for src in srcs {
        match src.json(&format!("repos/{repo}/releases?per_page=30")) {
            Ok(Value::Array(raw)) => {
                return match pick(&raw, channel) {
                    Some(r) => Ok((r, src.as_ref())),
                    None if raw.is_empty() => Err(ReleaseError::NoRelease(NO_RELEASE.into())),
                    None => Err(ReleaseError::NoRelease(format!("no {channel} release yet"))),
                };
            }
            Ok(_) => errors.push(format!("{}: unexpected answer", src.name())),
            Err(e) => errors.push(format!("{}: {e}", src.name())),
        }
    }
    Err(fail(if errors.is_empty() { "no way to reach GitHub".into() } else { errors.join("; ") }))
}

// --- firmware from a release -------------------------------------------------------------------------------

fn is_hex(s: &str, len: Option<usize>) -> bool {
    !s.is_empty() && len.is_none_or(|l| s.len() == l) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Errors unless the manifest is one we can trust to write files from.
pub fn check_manifest(m: &Value, rel: Option<&Release>) -> Result<(), ReleaseError> {
    if !m.get("version").and_then(Value::as_str).is_some_and(safe_name) {
        return Err(fail("the release's manifest.json has no usable firmware version"));
    }
    let parts = m.get("parts").and_then(Value::as_array).filter(|p| !p.is_empty());
    let Some(parts) = parts else { return Err(fail("the release's manifest.json lists no firmware parts")) };
    for p in parts {
        let name = p.get("file").and_then(Value::as_str).unwrap_or("");
        if !(safe_name(name) && name.ends_with(".bin") && name != MANIFEST) {
            return Err(fail(format!("odd firmware part name in the manifest: {name:?}")));
        }
        let off = p.get("offset").and_then(Value::as_str).unwrap_or("");
        if !off.strip_prefix("0x").is_some_and(|h| is_hex(h, None)) || p.get("size").and_then(Value::as_u64).is_none() {
            return Err(fail(format!("{name}: bad offset or size in the manifest")));
        }
        let sha = p.get("sha256").and_then(Value::as_str).unwrap_or("");
        if !(is_hex(sha, Some(64)) && sha.chars().all(|c| !c.is_ascii_uppercase())) {
            return Err(fail(format!("{name}: no sha256 in the manifest")));
        }
        if let Some(r) = rel
            && !r.assets.contains_key(name)
        {
            return Err(fail(format!("release {} lists {name} but doesn't have it", r.tag)));
        }
    }
    Ok(())
}

pub fn sha256_file(p: &Path) -> Option<String> {
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The folder holds a manifest and every part it lists, with the right size and sha256.
pub fn verify_dir(d: &Path, manifest: Option<&Value>) -> bool {
    let Some(m) = std::fs::read_to_string(d.join(MANIFEST)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())
    else {
        return false;
    };
    if check_manifest(&m, None).is_err() {
        return false;
    }
    if let Some(want) = manifest
        && m.get("sha256") != want.get("sha256")
    {
        return false;
    }
    m["parts"].as_array().into_iter().flatten().all(|p| {
        let f = d.join(p["file"].as_str().unwrap_or(""));
        std::fs::metadata(&f).is_ok_and(|md| md.is_file() && Some(md.len()) == p["size"].as_u64())
            && sha256_file(&f).as_deref() == p["sha256"].as_str()
    })
}

fn fetch(src: &dyn Source, rel: &Release, name: &str, dest: &Path) -> Result<(), ReleaseError> {
    src.download(rel, name, dest)?;
    if let Some(want) = rel.assets.get(name).and_then(|a| a.sha256.as_deref())
        && sha256_file(dest).as_deref() != Some(want)
    {
        return Err(fail(format!("{name}: doesn't match GitHub's sha256 for the asset")));
    }
    Ok(())
}

/// Removes a temporary folder however the download ends.
struct TmpDir(PathBuf);
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Downloads the release's firmware into `<root>/<firmware version>/` (verified), unless it's
/// already there. -> (folder, manifest).
pub fn download_firmware(
    rel: &Release,
    src: &dyn Source,
    root: &Path,
    say: &dyn Fn(String),
) -> Result<(PathBuf, Value), ReleaseError> {
    if !rel.assets.contains_key(MANIFEST) {
        return Err(fail(format!("release {} has no firmware (no {MANIFEST})", rel.tag)));
    }
    std::fs::create_dir_all(root).map_err(|e| fail(format!("{}: {e}", root.display())))?;
    let mut rnd = [0u8; 4];
    getrandom::fill(&mut rnd).map_err(|e| fail(e.to_string()))?;
    let tmp = TmpDir(root.join(format!(".download-{}-{:08x}", std::process::id(), u32::from_le_bytes(rnd))));
    std::fs::create_dir_all(&tmp.0).map_err(|e| fail(e.to_string()))?;
    fetch(src, rel, MANIFEST, &tmp.0.join(MANIFEST))?;
    let manifest: Value = serde_json::from_slice(&std::fs::read(tmp.0.join(MANIFEST)).unwrap_or_default())
        .map_err(|e| fail(format!("the release's manifest.json isn't JSON ({e})")))?;
    check_manifest(&manifest, Some(rel))?;
    let version = manifest["version"].as_str().unwrap_or_default().to_string();
    let target = root.join(&version);
    if verify_dir(&target, Some(&manifest)) {
        say(format!("firmware {version} ({}) is already here", rel.tag));
        return Ok((target, manifest));
    }
    let parts = manifest["parts"].as_array().cloned().unwrap_or_default();
    for p in &parts {
        let (file, size) = (p["file"].as_str().unwrap_or(""), p["size"].as_u64().unwrap_or(0));
        say(format!("downloading {file} ({} KB) from {} via {}", size / 1024, rel.tag, src.name()));
        let got = tmp.0.join(file);
        fetch(src, rel, file, &got)?;
        let len = std::fs::metadata(&got).map(|m| m.len()).unwrap_or(0);
        if len != size || sha256_file(&got).as_deref() != p["sha256"].as_str() {
            return Err(fail(format!("{file}: sha256 doesn't match the manifest — not using this download")));
        }
    }
    say(format!("firmware {version}: all {} parts match the manifest's sha256", parts.len()));
    let mut old = None;
    if target.exists() {
        let o = root.join(format!(".old-{version}-{}", std::process::id()));
        std::fs::rename(&target, &o).map_err(|e| fail(e.to_string()))?;
        old = Some(o);
    }
    std::fs::rename(&tmp.0, &target).map_err(|e| fail(format!("{}: {e}", target.display())))?;
    if let Some(o) = old {
        let _ = std::fs::remove_dir_all(o);
    }
    prune(root, KEEP);
    Ok((target, manifest))
}

/// Keeps the newest `keep` downloaded firmware versions (never the local build).
pub fn prune(root: &Path, keep: usize) {
    let mut found: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name().and_then(|n| n.to_str()).is_some_and(is_version))
        .collect();
    found.sort_by_key(|p| std::cmp::Reverse(version_key(p.file_name().and_then(|n| n.to_str()).unwrap_or(""))));
    for d in found.into_iter().skip(keep) {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::Mutex;

    pub fn sha(b: &[u8]) -> String {
        Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
    }

    /// GitHub in memory: releases and their assets' bytes.
    #[derive(Default)]
    pub struct FakeGitHub {
        pub releases: Vec<Value>,
        pub files: BTreeMap<(String, String), Vec<u8>>,
        pub fail_json: Option<String>,
        pub downloads: Mutex<Vec<String>>,
    }

    impl FakeGitHub {
        /// A release `tag` with firmware `fw` (bootloader + app), `prerelease`.
        pub fn add_release(&mut self, tag: &str, fw: &str, prerelease: bool) {
            let boot = format!("boot-{fw}").into_bytes();
            let mut app = format!("app-{fw}").into_bytes();
            app.insert(0, 0xE9);
            let m = json!({"version": fw, "board": "m5stick-s3", "sha256": sha(&app), "parts": [
                {"file": "bootloader.bin", "offset": "0x0", "size": boot.len(), "sha256": sha(&boot)},
                {"file": "app.bin", "offset": "0x10000", "size": app.len(), "sha256": sha(&app)}]});
            let mbytes = serde_json::to_vec(&m).unwrap();
            let mut assets = Vec::new();
            for (n, b) in [("manifest.json", mbytes), ("bootloader.bin", boot), ("app.bin", app)] {
                assets.push(json!({"name": n, "size": b.len(), "digest": format!("sha256:{}", sha(&b)),
                                   "browser_download_url": format!("https://example.invalid/{tag}/{n}")}));
                self.files.insert((tag.into(), n.into()), b);
            }
            self.releases.push(json!({"tag_name": tag, "name": tag, "prerelease": prerelease, "draft": false,
                                      "body": format!("notes for {tag}"), "html_url": format!("https://example.invalid/{tag}"),
                                      "assets": assets}));
        }
    }

    impl Source for FakeGitHub {
        fn name(&self) -> &str {
            "fake"
        }
        fn json(&self, _path: &str) -> Result<Value, ReleaseError> {
            match &self.fail_json {
                Some(e) => Err(fail(e.clone())),
                None => Ok(Value::Array(self.releases.clone())),
            }
        }
        fn download(&self, rel: &Release, name: &str, dest: &Path) -> Result<(), ReleaseError> {
            self.downloads.lock().unwrap().push(format!("{}/{name}", rel.tag));
            let b = self.files.get(&(rel.tag.clone(), name.to_string())).ok_or_else(|| fail("404"))?;
            std::fs::write(dest, b).map_err(|e| fail(e.to_string()))
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stickybot-rel-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn semver_precedence() {
        let order = ["junk", "1.7.9", "1.8.0-alpha", "1.8.0-beta.2", "1.8.0-beta.10", "1.8.0-beta.x", "1.8.0-rc.1", "1.8.0", "v1.8.1", "2.0.0"];
        for w in order.windows(2) {
            assert!(version_key(w[0]) < version_key(w[1]), "{} < {}", w[0], w[1]);
        }
        assert_eq!(version_key("v1.2.3+abc"), version_key("1.2.3"));
        assert!(!is_version("1.2") && !is_version("1.2.3-") && !is_version("01x.2.3"));
        assert!(is_prerelease("1.0.0-beta.1") && !is_prerelease("1.0.0"));
        assert!(newer(Some("1.9.0"), Some("1.8.9")) && newer(Some("1.0.0"), None));
        assert!(!newer(Some("x"), None) && !newer(Some("1.0.0"), Some("1.0.0")));
    }

    #[test]
    fn picks_per_channel_and_skips_drafts() {
        let mut gh = FakeGitHub::default();
        gh.add_release("v2.2.0", "2.3.0", false);
        gh.add_release("v2.3.0-beta.1", "2.4.0", true);
        gh.releases.push(json!({"tag_name": "v9.0.0", "draft": true}));
        gh.releases.push(json!({"tag_name": "nightly"}));
        assert_eq!(pick(&gh.releases, "stable").unwrap().tag, "v2.2.0");
        let beta = pick(&gh.releases, "beta").unwrap();
        assert_eq!((beta.tag.as_str(), beta.prerelease), ("v2.3.0-beta.1", true));
        assert!(beta.assets["app.bin"].sha256.is_some());
        let srcs: Vec<Box<dyn Source>> = vec![Box::new(FakeGitHub::default())];
        assert_eq!(latest(&srcs, "a/b", "stable").err(), Some(ReleaseError::NoRelease(NO_RELEASE.into())));
    }

    #[test]
    fn downloads_verified_firmware_once() {
        let mut gh = FakeGitHub::default();
        gh.add_release("v2.2.0", "2.3.0", false);
        let srcs: Vec<Box<dyn Source>> = vec![Box::new(gh)];
        let (rel, src) = latest(&srcs, "a/b", "stable").unwrap();
        let root = tmp("dl");
        let lines = Mutex::new(Vec::new());
        let (dir, m) = download_firmware(&rel, src, &root, &|l| lines.lock().unwrap().push(l)).unwrap();
        assert_eq!(dir, root.join("2.3.0"));
        assert_eq!(m["version"], "2.3.0");
        assert!(verify_dir(&dir, Some(&m)));
        assert!(crate::firmware::load_bundle(&dir, "release").is_some());
        // no temporary folders left
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        // a second time: already here, only the manifest is fetched
        download_firmware(&rel, src, &root, &|l| lines.lock().unwrap().push(l)).unwrap();
        assert!(lines.lock().unwrap().last().unwrap().contains("already here"));
    }

    #[test]
    fn a_tampered_part_is_never_used() {
        let mut gh = FakeGitHub::default();
        gh.add_release("v2.2.0", "2.3.0", false);
        // the file differs from both GitHub's digest and the manifest
        gh.files.insert(("v2.2.0".into(), "app.bin".into()), b"evil".to_vec());
        let rel = pick(&gh.releases, "stable").unwrap();
        let root = tmp("tamper");
        let e = download_firmware(&rel, &gh, &root, &|_| {}).unwrap_err();
        assert!(e.to_string().contains("sha256"), "{e}");
        assert!(!root.join("2.3.0").exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0); // the temporary folder is gone
        // a manifest with a path in a part name is refused before anything is written
        let bad = json!({"version": "1.0.0", "parts": [{"file": "../x.bin", "offset": "0x0", "size": 1, "sha256": "0".repeat(64)}]});
        assert!(check_manifest(&bad, None).unwrap_err().to_string().contains("odd"));
    }

    #[test]
    fn prune_keeps_the_newest_and_the_local_build() {
        let root = tmp("prune");
        for d in ["1.0.0", "1.1.0", "2.0.0-beta.1", "2.0.0", "local", ".download-x"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        prune(&root, 3);
        let mut left: Vec<String> =
            std::fs::read_dir(&root).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into()).collect();
        left.sort();
        assert_eq!(left, [".download-x", "1.1.0", "2.0.0", "2.0.0-beta.1", "local"]);
    }
}
