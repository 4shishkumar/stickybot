//! The Tauri updater plugin behind the server's [`Installer`] (`stickybot-server/src/selfupdate.rs`
//! decides when and what; this does the part only the menu bar app can).
//!
//! The plugin reads `latest.json` from the endpoint the server hands it (stable: GitHub's latest
//! release; beta: the picked release's own file), downloads the `.app.tar.gz` it names, checks its
//! minisign signature over the whole download against the public key compiled in from
//! `tauri.conf.json` (`plugins.updater.pubkey`), and only then, on `install`, swaps the bundle for
//! the new one in a single rename (the old one is put back if that fails). Its default version
//! comparator (remote > current) stays on as a second guard next to the server's. https is
//! required for endpoints by the plugin in release builds.
//!
//! `unavailable()` answers before any of that: where this app runs decides whether it can replace
//! itself, and the plugin would ask for an administrator password where it can't write (a prompt
//! from a menu bar app is a surprise): so those cases fall back to the download page instead.
//! The path rules are pure functions over facts about the folder, with tests.

use futures_util::future::BoxFuture;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use stickybot_server::selfupdate::{InstallError, Installer, Progress, Staged};
use tauri::{AppHandle, Runtime};
use tauri_plugin_updater::{Error, UpdaterExt};

/// How long the answer to "can this app replace itself here" is reused: it asks the file system,
/// and the menu and the status ask every half second.
pub const UNAVAILABLE_CACHE: Duration = Duration::from_secs(30);

/// Facts about where the app sits, for the check below (gathered by [`facts`], made up in tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderFacts {
    /// the folder holding StickyBot.app can be written by this user
    pub folder_writable: bool,
    /// the bundle itself can be written (a folder is only moved to another one if it can be)
    pub bundle_writable: bool,
    /// the folder has the sticky bit (/Applications does): only an entry's owner may rename it
    pub sticky: bool,
    pub folder_uid: u32,
    pub bundle_uid: u32,
    pub euid: u32,
}

/// `/Applications/StickyBot.app` for `/Applications/StickyBot.app/Contents/MacOS/stickybot-rs`.
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    let ok = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension().is_some_and(|e| e == "app");
    ok.then(|| app.to_path_buf())
}

/// Why the app at `exe` can't replace itself, in words (`None` = it can). Pure.
pub fn problem(exe: &Path, facts: Option<&FolderFacts>) -> Option<String> {
    let Some(bundle) = bundle_of(exe) else {
        return Some("this StickyBot isn't running from an installed StickyBot.app (a developer build?) — download the update by hand".into());
    };
    // Gatekeeper opens a downloaded app from a randomised read-only copy until it is moved
    if exe.to_string_lossy().contains("/AppTranslocation/") {
        return Some(
            "StickyBot is running from a temporary read-only copy — move it to Applications and open it again".into(),
        );
    }
    let Some(f) = facts else {
        return Some(format!("StickyBot can't look at {} — download the update by hand", bundle.display()));
    };
    // In a sticky folder only the owner of the entry (or of the folder, or root) may move it away
    let may_rename = !f.sticky || f.euid == 0 || f.euid == f.bundle_uid || f.euid == f.folder_uid;
    if !(f.folder_writable && f.bundle_writable && may_rename) {
        return Some(format!(
            "StickyBot can't replace itself at {} (move it to Applications, or download it by hand)",
            bundle.display()
        ));
    }
    None
}

/// Reads [`FolderFacts`] off the file system.
#[cfg(unix)]
pub fn facts(bundle: &Path) -> Option<FolderFacts> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    let folder = bundle.parent()?;
    let (fm, bm) = (std::fs::metadata(folder).ok()?, std::fs::metadata(bundle).ok()?);
    let writable = |p: &Path| {
        let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else { return false };
        // SAFETY: `c` is a valid NUL-terminated path that outlives the call; access(2) only reads it.
        unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
    };
    Some(FolderFacts {
        folder_writable: writable(folder),
        bundle_writable: writable(bundle),
        sticky: fm.mode() & 0o1000 != 0,
        folder_uid: fm.uid(),
        bundle_uid: bm.uid(),
        // SAFETY: geteuid(2) has no arguments and cannot fail.
        euid: unsafe { libc::geteuid() },
    })
}

/// The plugin's errors as the server's kinds. `bundle` is where the app sits, for the words.
pub fn map_error(e: &Error, bundle: &Path) -> InstallError {
    use std::io::ErrorKind::{PermissionDenied, ReadOnlyFilesystem};
    match e {
        // the signature isn't ours, can't be read, or is for another version than the one announced
        Error::Minisign(_)
        | Error::Base64(_)
        | Error::SignatureUtf8(_)
        | Error::SignedVersionMismatch { .. }
        | Error::MissingSignedVersion => InstallError::Signature(e.to_string()),
        Error::Reqwest(_) | Error::Network(_) | Error::ReleaseNotFound => InstallError::Network(e.to_string()),
        Error::Io(io) if matches!(io.kind(), PermissionDenied | ReadOnlyFilesystem) => {
            InstallError::NotWritable(bundle.display().to_string())
        }
        Error::TempDirNotOnSameMountPoint => InstallError::NotWritable(bundle.display().to_string()),
        Error::TargetNotFound(_) | Error::TargetsNotFound(_) => InstallError::NoUpdate(
            "This release has no update for this Mac — download it from the release page.".into(),
        ),
        Error::PreviousAppNotRestored(kept) => InstallError::Other(format!(
            "The update could not be put in place, and the previous StickyBot was left at {} — move it back to Applications.",
            kept.display()
        )),
        other => InstallError::Other(format!("The update didn't work: {other}")),
    }
}

pub struct TauriInstaller<R: Runtime = tauri::Wry> {
    app: AppHandle<R>,
    /// the running executable (tests point it at a bundle they made)
    exe: PathBuf,
    cached: Mutex<Option<(Instant, Option<String>)>>,
}

impl<R: Runtime> TauriInstaller<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        let exe = std::env::current_exe().unwrap_or_default();
        Self::at(app, exe)
    }

    /// For the executable inside `exe`'s bundle (what `new` does with the running one).
    pub fn at(app: AppHandle<R>, exe: PathBuf) -> Self {
        Self { app, exe, cached: Mutex::new(None) }
    }

    fn bundle(&self) -> PathBuf {
        bundle_of(&self.exe).unwrap_or_else(|| self.exe.clone())
    }
}

impl<R: Runtime> Installer for TauriInstaller<R> {
    fn unavailable(&self) -> Option<String> {
        let mut c = self.cached.lock().unwrap();
        if let Some((at, answer)) = c.as_ref()
            && at.elapsed() < UNAVAILABLE_CACHE
        {
            return answer.clone();
        }
        let answer = problem(&self.exe, bundle_of(&self.exe).and_then(|b| facts(&b)).as_ref());
        *c = Some((Instant::now(), answer.clone()));
        answer
    }

    fn download(
        &self,
        endpoint: String,
        progress: Progress,
    ) -> BoxFuture<'static, Result<Box<dyn Staged>, InstallError>> {
        let (app, exe, bundle) = (self.app.clone(), self.exe.clone(), self.bundle());
        Box::pin(async move {
            let at = bundle.as_path();
            let bad = move |e: Error| map_error(&e, at); // a Copy closure: used for every step
            let url: tauri::Url =
                endpoint.parse().map_err(|e| InstallError::Other(format!("the update address isn't valid: {e}")))?;
            let updater =
                app.updater_builder().executable_path(exe).endpoints(vec![url]).map_err(bad)?.build().map_err(bad)?;
            let update = updater.check().await.map_err(bad)?.ok_or_else(|| InstallError::NoUpdate(String::new()))?;
            let mut got = 0u64;
            // the plugin checks the minisign signature over everything it got before it returns
            let bytes = update
                .download(
                    |n, total| {
                        got += n as u64;
                        progress(got, total);
                    },
                    || {},
                )
                .await
                .map_err(bad)?;
            Ok(Box::new(TauriStaged { update, bytes, bundle }) as Box<dyn Staged>)
        })
    }
}

struct TauriStaged {
    update: tauri_plugin_updater::Update,
    bytes: Vec<u8>,
    bundle: PathBuf,
}

impl Staged for TauriStaged {
    fn version(&self) -> String {
        self.update.version.clone()
    }

    /// Checks the signature again (the plugin does, cheaply) and swaps the bundle.
    fn install(self: Box<Self>) -> Result<(), InstallError> {
        self.update.install(&self.bytes).map_err(|e| map_error(&e, &self.bundle))
    }
}

/// The installer the menu bar app gives the server.
pub fn installer(app: &AppHandle) -> Arc<dyn Installer> {
    Arc::new(TauriInstaller::new(app.clone()))
}

#[cfg(test)]
#[path = "updater_tests.rs"]
mod tests;
