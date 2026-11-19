//! The local assistant's pinned parts: llama.cpp's server, Goose (ACP) and the model. Each is
//! downloaded once over HTTPS (resumable), checked against a pinned SHA-256 and kept under
//! `<home>/assistant/`. Nothing is run that doesn't match its pin.
//!
//! A model LM Studio already downloaded is used in place when its hash matches (read only).

use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// a .tar.gz unpacked into `dir`
    TarGz,
    /// a single file saved as `dir/file`
    File,
}

#[derive(Debug)]
pub struct Artifact {
    pub label: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    pub kind: Kind,
    /// folder under `<home>/assistant/`
    pub dir: &'static str,
    /// the file used inside `dir`
    pub file: &'static str,
}

pub const GOOSE: Artifact = Artifact {
    label: "Goose 1.59.2",
    url: "https://github.com/block/goose/releases/download/v1.59.2/goose-acp-aarch64-apple-darwin.tar.gz",
    sha256: "0184f802536c404b281e80599adee799517c541c97119b010a9a39e1e018f954",
    size: 9_056_661,
    kind: Kind::TarGz,
    dir: "goose-1.59.2",
    file: "goose-acp",
};

pub const LLAMA: Artifact = Artifact {
    label: "llama.cpp b12477",
    url: "https://github.com/ggml-org/llama.cpp/releases/download/b12477/llama-b12477-bin-macos-arm64.tar.gz",
    sha256: "534fb4bff92282b25b065d7622c6383f8852a3b36259de376583d093cbfd902d",
    size: 17_019_634,
    kind: Kind::TarGz,
    dir: "llama-b12477",
    file: "llama-b12477/llama-server",
};

pub const MODEL: Artifact = Artifact {
    label: "Qwen3.5 9B (Q4_K_M)",
    url: "https://huggingface.co/lmstudio-community/Qwen3.5-9B-GGUF/resolve/main/Qwen3.5-9B-Q4_K_M.gguf",
    sha256: "cd76ec205963b3b33350093e6904d9de16c4e666fd104e1f632d25c7f15f2a13",
    size: 5_627_044_256,
    kind: Kind::File,
    dir: "models",
    file: "Qwen3.5-9B-Q4_K_M.gguf",
};

pub const ALL: [&Artifact; 3] = [&GOOSE, &LLAMA, &MODEL];

/// Where LM Studio keeps the same file.
const LM_STUDIO_MODEL: &str = ".lmstudio/models/lmstudio-community/Qwen3.5-9B-GGUF/Qwen3.5-9B-Q4_K_M.gguf";

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Progress {
    /// idle / downloading / verifying / unpacking / ready / failed
    pub phase: String,
    pub item: String,
    pub done: u64,
    pub total: u64,
    pub error: String,
}

pub struct Runtime {
    root: PathBuf,
    user_home: PathBuf,
    progress: Arc<Mutex<Progress>>,
    installing: Arc<Mutex<bool>>,
}

impl Runtime {
    pub fn new(app_home: &Path, user_home: PathBuf) -> Self {
        Self {
            root: app_home.join("assistant"),
            user_home,
            progress: Arc::new(Mutex::new(Progress { phase: "idle".into(), ..Default::default() })),
            installing: Arc::new(Mutex::new(false)),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn marker(&self, a: &Artifact) -> PathBuf {
        self.root.join(format!(".{}.ok", a.dir))
    }

    /// The artifact's verified file, if it's installed (or LM Studio's copy was verified).
    pub fn path(&self, a: &Artifact) -> Option<PathBuf> {
        let mark = fs::read_to_string(self.marker(a)).ok()?;
        let mut lines = mark.lines();
        if lines.next()? != a.sha256 {
            return None; // a different pin: install again
        }
        let p = match lines.next() {
            Some(external) if !external.is_empty() => PathBuf::from(external),
            _ => self.root.join(a.dir).join(a.file),
        };
        // a moved/changed external file needs checking again
        let ok = fs::metadata(&p).is_ok_and(|m| a.kind == Kind::TarGz || m.len() == a.size);
        ok.then_some(p)
    }

    pub fn installed(&self) -> bool {
        ALL.iter().all(|a| self.path(a).is_some())
    }

    pub fn progress(&self) -> Progress {
        let mut p = self.progress.lock().unwrap().clone();
        if p.phase == "idle" && self.installed() {
            p.phase = "ready".into();
        }
        p
    }

    /// Downloads whatever is missing, on a worker thread. False if one is already running.
    pub fn install(self: &Arc<Self>) -> bool {
        {
            let mut busy = self.installing.lock().unwrap();
            if *busy {
                return false;
            }
            *busy = true;
        }
        let me = self.clone();
        std::thread::spawn(move || {
            let r = me.install_blocking();
            let mut p = me.progress.lock().unwrap();
            match r {
                Ok(()) => *p = Progress { phase: "ready".into(), ..Default::default() },
                Err(e) => {
                    log::warn!("local assistant install: {e}");
                    p.phase = "failed".into();
                    p.error = e;
                }
            }
            *me.installing.lock().unwrap() = false;
        });
        true
    }

    fn set(&self, phase: &str, a: &Artifact, done: u64) {
        *self.progress.lock().unwrap() =
            Progress { phase: phase.into(), item: a.label.into(), done, total: a.size, error: String::new() };
    }

    pub fn install_blocking(&self) -> Result<(), String> {
        fs::create_dir_all(self.root.join("downloads")).map_err(|e| e.to_string())?;
        for a in ALL {
            if self.path(a).is_some() {
                continue;
            }
            if a.kind == Kind::File && self.adopt_lm_studio(a) {
                continue;
            }
            let part = self.root.join("downloads").join(format!("{}.part", a.dir));
            self.download(a, &part)?;
            self.set("verifying", a, a.size);
            let got = sha256_file(&part).map_err(|e| e.to_string())?;
            if got != a.sha256 {
                let _ = fs::remove_file(&part);
                return Err(format!("{}: checksum mismatch, download removed", a.label));
            }
            let dest = self.root.join(a.dir);
            let tmp = self.root.join(format!("{}.tmp", a.dir));
            let _ = fs::remove_dir_all(&tmp);
            fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
            match a.kind {
                Kind::TarGz => {
                    self.set("unpacking", a, a.size);
                    unpack(&part, &tmp)?;
                    let _ = fs::remove_file(&part);
                }
                Kind::File => fs::rename(&part, tmp.join(a.file)).map_err(|e| e.to_string())?,
            }
            let _ = fs::remove_dir_all(&dest);
            fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
            if !dest.join(a.file).is_file() {
                return Err(format!("{}: {} missing after unpacking", a.label, a.file));
            }
            fs::write(self.marker(a), format!("{}\n", a.sha256)).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// LM Studio's copy, if it's byte-for-byte the pinned model.
    fn adopt_lm_studio(&self, a: &Artifact) -> bool {
        let p = self.user_home.join(LM_STUDIO_MODEL);
        if !fs::metadata(&p).is_ok_and(|m| m.is_file() && m.len() == a.size) {
            return false;
        }
        self.set("verifying", a, a.size);
        if sha256_file(&p).ok().as_deref() != Some(a.sha256) {
            return false;
        }
        fs::write(self.marker(a), format!("{}\n{}\n", a.sha256, p.display())).is_ok()
    }

    fn download(&self, a: &Artifact, part: &Path) -> Result<(), String> {
        let mut have = fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        if have > a.size {
            let _ = fs::remove_file(part);
            have = 0;
        }
        if have == a.size {
            return Ok(());
        }
        self.set("downloading", a, have);
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(20))
            .timeout_read(std::time::Duration::from_secs(60))
            .https_only(true)
            .build();
        let mut req = agent.get(a.url).set("User-Agent", "StickyBot");
        if have > 0 {
            req = req.set("Range", &format!("bytes={have}-"));
        }
        let resp = req.call().map_err(|e| format!("{}: {e}", a.label))?;
        let resumed = resp.status() == 206;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(resumed)
            .truncate(!resumed)
            .open(part)
            .map_err(|e| e.to_string())?;
        if !resumed {
            have = 0;
        }
        let mut body = resp.into_reader().take(a.size - have);
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = body.read(&mut buf).map_err(|e| format!("{}: {e}", a.label))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            have += n as u64;
            self.progress.lock().unwrap().done = have;
        }
        file.flush().map_err(|e| e.to_string())?;
        if have != a.size {
            return Err(format!(
                "{}: download stopped early ({have} of {} bytes); try again to resume",
                a.label, a.size
            ));
        }
        Ok(())
    }
}

pub fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Unpacks a .tar.gz inside `dest` only (no absolute paths, no `..`, no links out of it).
fn unpack(archive: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let f = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    tar.set_preserve_permissions(true);
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        if let Ok(Some(link)) = entry.link_name()
            && (link.is_absolute() || link.components().any(|c| matches!(c, std::path::Component::ParentDir)))
        {
            return Err(format!("archive link leaves its folder: {}", link.display()));
        }
        // unpack_in refuses paths outside `dest`
        if !entry.unpack_in(dest).map_err(|e| e.to_string())? {
            return Err("archive entry outside its folder".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stickybot-rt-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn nothing_counts_as_installed_without_a_matching_marker() {
        let d = tmp("marker");
        let rt = Runtime::new(&d, d.join("user"));
        assert!(!rt.installed());
        fs::create_dir_all(d.join("assistant/goose-1.59.2")).unwrap();
        fs::write(d.join("assistant/goose-1.59.2/goose-acp"), b"x").unwrap();
        fs::write(rt.marker(&GOOSE), "an-old-pin\n").unwrap();
        assert!(rt.path(&GOOSE).is_none());
        fs::write(rt.marker(&GOOSE), format!("{}\n", GOOSE.sha256)).unwrap();
        assert_eq!(rt.path(&GOOSE), Some(d.join("assistant/goose-1.59.2/goose-acp")));
        // a model file of the wrong size is not used
        fs::create_dir_all(d.join("assistant/models")).unwrap();
        fs::write(d.join("assistant/models").join(MODEL.file), b"short").unwrap();
        fs::write(rt.marker(&MODEL), format!("{}\n", MODEL.sha256)).unwrap();
        assert!(rt.path(&MODEL).is_none());
        assert_eq!(rt.progress().phase, "idle");
    }

    #[test]
    fn hashes_files() {
        let d = tmp("hash");
        fs::write(d.join("a"), b"abc").unwrap();
        assert_eq!(
            sha256_file(&d.join("a")).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn targz(entries: &[(&str, &[u8])], link: Option<(&str, &str)>) -> Vec<u8> {
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        for (name, data) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o755);
            h.set_cksum();
            b.append_data(&mut h, name, *data).unwrap();
        }
        if let Some((name, target)) = link {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            b.append_link(&mut h, name, target).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn unpacks_inside_its_folder_only() {
        let d = tmp("unpack");
        fs::write(d.join("ok.tgz"), targz(&[("dir/bin", b"run")], Some(("dir/lib.dylib", "lib.1.dylib")))).unwrap();
        unpack(&d.join("ok.tgz"), &d.join("out")).unwrap();
        assert_eq!(fs::read(d.join("out/dir/bin")).unwrap(), b"run");
        fs::write(d.join("bad.tgz"), targz(&[], Some(("evil", "../../etc/passwd")))).unwrap();
        assert!(unpack(&d.join("bad.tgz"), &d.join("out2")).is_err());
        fs::write(d.join("abs.tgz"), targz(&[], Some(("evil", "/etc/passwd")))).unwrap();
        assert!(unpack(&d.join("abs.tgz"), &d.join("out3")).is_err());
    }
}
