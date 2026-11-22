//! Deny-default Seatbelt launcher. The calling app owns the private scratch directory.
use std::os::unix::fs::PermissionsExt;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct Policy {
    pub scratch: PathBuf,
    pub read_only: Vec<PathBuf>,
    pub loopback_ports: Vec<u16>,
    pub listen_ports: Vec<u16>,
    /// One port the trusted app listens on at 127.0.0.1 only (the local brain's relay). Uses
    /// Seatbelt's "localhost:PORT" rule, which also matches the Mac's LAN address on that same
    /// port; nothing listens there, so only the app's loopback listener is reachable.
    pub app_port: Option<u16>,
}

#[cfg(target_os = "macos")]
mod listener;

pub(crate) fn quote(path: &Path) -> Result<String, String> {
    let s = path.to_str().filter(|s| !s.chars().any(char::is_control)).ok_or("invalid sandbox path")?;
    let mut quoted = String::from("\"");
    for c in s.chars() {
        if matches!(c, '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    Ok(quoted)
}

fn canonical(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("absolute sandbox path required".into());
    }
    path.canonicalize().map_err(|_| "invalid sandbox path".into())
}

const BASE: &str = r#"(version 1)
(deny default)
(allow sysctl-read)
(allow file-read* (literal "/") (literal "/dev/null") (literal "/dev/random") (literal "/dev/urandom"))
(allow file-read* file-map-executable (subpath "/System/Library") (subpath "/usr/lib"))
"#;

fn wrapped(program: &Path, args: &[OsString], profile: String, dir: &Path) -> Result<Command, String> {
    if !cfg!(target_os = "macos") || !Path::new("/usr/bin/sandbox-exec").is_file() {
        return Err("Seatbelt unavailable".into());
    }
    let mut cmd = Command::new("/usr/bin/sandbox-exec");
    cmd.env_clear()
        .env("LC_ALL", "C")
        .arg("-p")
        .arg(profile)
        .arg(program)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(cmd)
}

fn profile(program: &Path, policy: &Policy) -> Result<(String, PathBuf), String> {
    let scratch = canonical(&policy.scratch)?;
    let home = canonical(Path::new(&std::env::var_os("HOME").ok_or("missing HOME")?))?;
    let app = crate::config::default_home();
    let app = app.canonicalize().unwrap_or(app);
    if !scratch.is_dir()
        || home.starts_with(&scratch)
        || scratch.metadata().map_err(|_| "invalid scratch")?.permissions().mode() & 0o077 != 0
        || crate::assistant::excluded(&scratch, &app)
    {
        return Err("scratch must be a private directory".into());
    }
    let runtime = app.join("assistant");
    let runtime_denied = |p: &Path| {
        if let Ok(rel) = p.strip_prefix(&runtime) {
            crate::assistant::excluded(rel, &app)
        } else {
            crate::assistant::excluded(p, &app)
        }
    };
    let mut p = BASE.to_string();
    if !policy.listen_ports.is_empty() {
        p.push_str(r#"(allow iokit-get-properties)
(allow iokit-open (iokit-user-client-class "AGXDeviceUserClient")(iokit-user-client-class "IOSurfaceRootUserClient")(iokit-registry-entry-class-prefix "AGXAcceleratorG"))
(allow mach-lookup (global-name "com.apple.MTLCompilerService"))
"#);
    }
    let mut exec = |path: &Path| -> Result<(), String> {
        if !path.is_file() || path.starts_with(&scratch) || runtime_denied(path) {
            return Err("invalid executable".into());
        }
        let q = quote(path)?;
        p.push_str(&format!("(allow process-exec file-read* file-map-executable (literal {q}))\n"));
        Ok(())
    };
    exec(program)?;
    let mut dirs = Vec::new();
    for entry in &policy.read_only {
        let path = canonical(entry)?;
        if runtime_denied(&path) || home.starts_with(&path) || scratch.starts_with(&path) {
            return Err("invalid runtime path".into());
        }
        if path.is_file() && path.metadata().map_err(|_| "invalid runtime")?.permissions().mode() & 0o111 != 0 {
            exec(&path)?
        } else {
            dirs.push(path)
        }
    }
    for path in dirs {
        p.push_str(&format!(
            "(allow file-read* file-map-executable ({} {}))\n",
            if path.is_dir() { "subpath" } else { "literal" },
            quote(&path)?
        ));
    }
    // SQLite (Goose's session store) resolves the scratch path: metadata only, no listings
    for dir in scratch.ancestors().skip(1) {
        p.push_str(&format!("(allow file-read-metadata (literal {}))\n", quote(dir)?));
    }
    p.push_str(&format!(
        "(allow file-read* file-write* (subpath {}))(allow file-write-data (literal \"/dev/null\"))\n",
        quote(&scratch)?
    ));
    p.push_str(&crate::assistant::seatbelt_exclusions(true)?);
    // On hosts that reject numeric address filters, profile compilation must fail.
    // Seatbelt's "localhost" also matches the Mac's LAN address.
    for port in &policy.loopback_ports {
        if *port == 0 {
            return Err("invalid loopback port".into());
        }
        p.push_str(&format!(
            "(allow network-outbound (require-all (remote tcp \"127.0.0.1:{port}\")(socket-domain AF_INET)))\n"
        ));
    }
    if let Some(port) = policy.app_port {
        // the app must hold the port on loopback already, so no confined process can take it
        let held = matches!(std::net::TcpListener::bind(("127.0.0.1", port)),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse);
        if port == 0 || !held || policy.loopback_ports.contains(&port) || policy.listen_ports.contains(&port) {
            return Err("invalid app port".into());
        }
        p.push_str(&format!(
            "(allow network-outbound (require-all (remote tcp \"localhost:{port}\")(socket-domain AF_INET)))\n"
        ));
    }
    if !policy.listen_ports.is_empty() {
        if policy.listen_ports.contains(&0) {
            return Err("invalid listen port".into());
        }
        #[cfg(target_os = "macos")]
        {
            let socket = quote(&listener::socket_path(policy)?)?;
            p.push_str(&format!("(allow system-socket (socket-domain AF_UNIX))(allow network-bind network-inbound (local unix-socket (literal {socket})))\n"));
        }
    }
    Ok((p, scratch))
}

fn command(program: &Path, args: &[OsString], policy: &Policy) -> Result<Command, String> {
    let program = canonical(program)?;
    let (p, scratch) = profile(&program, policy)?;
    let mut cmd = wrapped(&program, args, p, &scratch)?;
    cmd.env("HOME", &scratch).env("TMPDIR", &scratch).env("GOOSE_PATH_ROOT", &scratch);
    Ok(cmd)
}

pub fn confine(program: &Path, args: &[OsString], policy: &Policy) -> Result<tokio::process::Command, String> {
    self_test(policy)?;
    tokio::task::block_in_place(|| confined_command(program, args, policy))
}

/// Asynchronous entry point, including for single-threaded Tokio callers.
pub async fn confine_async(
    program: &Path,
    args: &[OsString],
    policy: &Policy,
) -> Result<tokio::process::Command, String> {
    self_test_async(policy).await?;
    let (program, args, policy) = (program.to_owned(), args.to_owned(), policy.clone());
    tokio::task::spawn_blocking(move || confined_command(&program, &args, &policy))
        .await
        .map_err(|_| "sandbox setup failed")?
}

fn confined_command(program: &Path, args: &[OsString], policy: &Policy) -> Result<tokio::process::Command, String> {
    #[cfg(target_os = "macos")]
    if !policy.listen_ports.is_empty() {
        return listener::server(program, args, policy);
    }
    Ok(command(program, args, policy)?.into())
}

/// Synchronous compatibility entry point. On a multithread runtime Tokio hands the
/// worker's other tasks off while the caller waits. Single-thread runtimes must await.
pub fn self_test(policy: &Policy) -> Result<(), String> {
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        if rt.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread {
            return tokio::task::block_in_place(|| rt.block_on(self_test_async(policy)));
        }
        return Err("use self_test_async or confine_async on a single-thread runtime".into());
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "probe runtime unavailable")?
        .block_on(self_test_async(policy))
}

pub async fn self_test_async(policy: &Policy) -> Result<(), String> {
    let policy_copy = policy.clone();
    let (_listener, cmd) =
        tokio::task::spawn_blocking(move || probe_command(&policy_copy)).await.map_err(|_| "probe setup failed")??;
    let mut child =
        tokio::process::Command::from(cmd).kill_on_drop(true).spawn().map_err(|_| "Seatbelt probe failed to launch")?;
    let status = match tokio::time::timeout(Duration::from_secs(4), child.wait()).await {
        Ok(result) => result.map_err(|_| "Seatbelt probe failed")?,
        Err(_) => {
            let _ = child.kill().await;
            return Err("Seatbelt probe timed out".into());
        }
    };
    if status.success() {
        Ok(())
    } else if status.code() == Some(65) && !policy.loopback_ports.is_empty() {
        Err("Seatbelt cannot enforce outbound TCP to exactly 127.0.0.1 on this host".into())
    } else {
        Err(format!("Seatbelt probe failed ({status})"))
    }
}

fn probe_command(policy: &Policy) -> Result<(std::net::TcpListener, Command), String> {
    let perl = probe_runtime()?;
    let home = std::env::var_os("HOME").ok_or("missing HOME")?;
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|_| "probe listener failed")?;
    let port = listener.local_addr().map_err(|_| "probe listener failed")?.port();
    if policy.loopback_ports.contains(&port) || policy.app_port == Some(port) {
        return Err("probe port conflicts with allowed ports".into());
    }
    // O_EXCL prevents overwriting anything even on a failing host.
    let name = format!(".stickybot-probe-{}", crate::config::new_token());
    let outside = Path::new(&home).join(&name);
    let inside = canonical(&policy.scratch)?.join(&name);
    let code = r#"use Fcntl qw(O_WRONLY O_CREAT O_EXCL); my ($out,$in,$ssh,$port,@listen)=@ARGV;
    if(sysopen(my $f,$out,O_WRONLY|O_CREAT|O_EXCL,0600)){unlink $out; exit 1}
    exit 2 unless $!{EPERM} || $!{EACCES};
    exit 3 if opendir(my $d,$ssh); exit 4 unless $!{EPERM} || $!{EACCES};
    socket(my $s,2,1,6) or die; exit 5 if connect($s,pack("C C n C4 x8",16,2,$port,127,0,0,1));
    exit 6 unless $!{EPERM} || $!{EACCES};
    for my $p (@listen) { socket(my $w,2,1,6) or die;
        exit 7 if bind($w,pack("C C n C4 x8",16,2,$p,0,0,0,0));
        exit 8 unless $!{EPERM} || $!{EACCES}; }
    sysopen(my $f,$in,O_WRONLY|O_CREAT|O_EXCL,0600) or die; print $f "ok"; close $f or die; unlink $in or die;"#;
    let mut args = vec![
        "-e".into(),
        code.into(),
        outside.into(),
        inside.into(),
        Path::new(&home).join(".ssh").into(),
        port.to_string().into(),
    ];
    args.extend(policy.listen_ports.iter().map(|p| OsString::from(p.to_string())));
    Ok((listener, command(&perl, &args, policy)?))
}

/// The system Perl binary itself (/usr/bin/perl only re-executes it).
fn probe_runtime() -> Result<PathBuf, String> {
    std::fs::read_dir("/usr/bin")
        .map_err(|_| "missing probe runtime")?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with("perl5.") && s[6..].bytes().all(|b| b.is_ascii_digit()))
        })
        .ok_or_else(|| "missing probe runtime".into())
}

// Git processes repository-controlled config and attributes. Deny subprocesses, network,
// and all writes at the kernel boundary even if that config requests an external helper.
pub(crate) fn git(repo: &Path, args: &[OsString]) -> Result<tokio::process::Command, String> {
    let exe =
        ["/Library/Developer/CommandLineTools/usr/bin/git", "/Applications/Xcode.app/Contents/Developer/usr/bin/git"]
            .into_iter()
            .map(Path::new)
            .find(|p| p.is_file())
            .ok_or("Apple Git unavailable")?;
    // Git needs its own metadata, including .gitignore in visible worktree directories.
    // Otherwise it silently reports ignored files as untracked. The broker still excludes
    // every dot path from file access, names and counts returned to the assistant.
    let git_dir = quote(&repo.join(".git"))?;
    let working_tree_rules = r#"(require-all (regex #"/[.]gitignore$") (require-not (regex #"/[.][^/]+/")))"#;
    let p = format!(
        r#"{BASE}
(allow file-write-data (literal "/dev/null"))
(allow process-exec file-read* file-map-executable (literal {}))
(allow file-read* (subpath {}))
{}
(deny file-read* (require-all (regex #"/[.][^/]") (require-not (subpath {git_dir})) (require-not {working_tree_rules})))"#,
        quote(exe)?,
        quote(repo)?,
        crate::assistant::seatbelt_exclusions(false)?
    );
    let mut cmd = wrapped(exe, args, p, repo)?;
    cmd.env("GIT_ATTR_NOSYSTEM", "1")
        .env("HOME", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1");
    Ok(cmd.into())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    fn policy() -> Policy {
        use std::os::unix::fs::DirBuilderExt;
        let scratch = std::env::temp_dir().join(format!("stickybot-llama-{}", crate::config::new_token()));
        std::fs::DirBuilder::new().mode(0o700).create(&scratch).unwrap();
        Policy { scratch, read_only: vec![], loopback_ports: vec![], listen_ports: vec![], app_port: None }
    }
    #[test]
    fn outbound_policy_never_widens_a_literal_address() {
        let mut p = policy();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        p.loopback_ports.push(listener.local_addr().unwrap().port());
        // A host that cannot compile exact address filters must refuse to launch.
        match confine(Path::new("/usr/bin/nc"), &[], &p) {
            Err(error) => assert!(error.contains("exactly 127.0.0.1"), "{error}"),
            Ok(_) => assert!(self_test(&p).is_ok()),
        }
        std::fs::remove_dir(p.scratch).unwrap();
    }
    #[tokio::test]
    async fn single_thread_caller_can_await_the_probe() {
        let p = policy();
        let mut cmd = confine_async(Path::new("/usr/bin/true"), &[], &p).await.unwrap();
        assert!(cmd.status().await.unwrap().success());
        std::fs::remove_dir(p.scratch).unwrap();
    }

    /// The relay port is reachable; another loopback port and a port the app doesn't hold are not.
    #[test]
    fn app_port_reaches_only_the_apps_listener() {
        let mut p = policy();
        let app = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let other = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let (ap, op) = (app.local_addr().unwrap().port(), other.local_addr().unwrap().port());
        p.app_port = Some(ap);
        self_test(&p).unwrap();
        let probe = |port: u16| {
            let code = format!(
                "socket(my $s,2,1,6) or die; exit(connect($s,pack('C C n C4 x8',16,2,{port},127,0,0,1)) ? 0 : 9)"
            );
            let mut c = command(&probe_runtime().unwrap(), &["-e".into(), code.into()], &p).unwrap();
            c.status().unwrap().code()
        };
        assert_eq!(probe(ap), Some(0));
        assert_eq!(probe(op), Some(9));
        // a port the app doesn't hold is refused outright
        let free = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
        p.app_port = Some(free);
        assert!(confine(Path::new("/usr/bin/true"), &[], &p).is_err());
        drop(other);
        std::fs::remove_dir(p.scratch).unwrap();
    }
    #[test]
    fn scratch_only() {
        let p = policy();
        self_test(&p).unwrap();
        std::fs::remove_dir(p.scratch).unwrap()
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires an installed llama-server, Metal GPU and local GGUF"]
    async fn metal_health() {
        let binary = PathBuf::from(std::env::var_os("STICK_TEST_LLAMA").unwrap());
        let model = PathBuf::from(std::env::var_os("STICK_TEST_GGUF").unwrap());
        let mut p = policy();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        p.listen_ports.push(port);
        p.read_only = vec![binary.parent().unwrap().into(), model.clone()];
        let args = [
            "-m".into(),
            model.into(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            port.to_string().into(),
            "--log-verbosity".into(),
            "5".into(),
            "-ngl".into(),
            "99".into(),
            "-c".into(),
            "512".into(),
        ];
        let log = p.scratch.join("server.log");
        let log_file = File::create(&log).unwrap();
        let mut child = confine(&binary, &args, &p)
            .unwrap()
            .stdout(log_file.try_clone().unwrap())
            .stderr(log_file)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let url = format!("http://127.0.0.1:{port}/health");
        let mut healthy = false;
        for _ in 0..180 {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if ureq::get(&url).timeout(Duration::from_secs(1)).call().is_ok() {
                healthy = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let _ = child.kill().await;
        let log_text = std::fs::read_to_string(&log).unwrap();
        assert!(
            healthy && regex::Regex::new(r"offloaded [1-9][0-9]*/[0-9]+ layers to GPU").unwrap().is_match(&log_text),
            "health={healthy}; {log_text}"
        );
        std::fs::remove_dir_all(p.scratch).unwrap();
    }
    use std::fs::File;
}
