//! Seatbelt matches INADDR_ANY for a local "localhost" rule. Only trusted app code
//! owns the TCP listener. The confined server gets a single Unix socket in scratch.
use super::*;
use std::{
    io, mem,
    net::TcpListener,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::process::CommandExt,
    },
};
use tokio::{
    io::{AsyncReadExt, unix::AsyncFd},
    net::UnixStream,
    task::JoinSet,
};

pub(super) fn socket_path(policy: &Policy) -> Result<PathBuf, String> {
    let path = canonical(&policy.scratch)?.join("llama.sock");
    Ok(path)
}

pub(super) fn server(program: &Path, args: &[OsString], policy: &Policy) -> Result<tokio::process::Command, String> {
    let [port] = policy.listen_ports.as_slice() else { return Err("llama-server requires one listen port".into()) };
    if program.file_name() != Some("llama-server".as_ref()) {
        return Err("TCP listener adapter requires llama-server".into());
    }
    let dir = std::fs::File::open(canonical(&policy.scratch)?).map_err(|_| "invalid scratch")?;
    use std::os::unix::fs::MetadataExt;
    let meta = dir.metadata().map_err(|_| "invalid scratch")?;
    // Darwin's volume/inode path fits sun_path even for long TMPDIR names. Holding
    // the directory descriptor prevents inode reuse for the relay's lifetime.
    let socket = PathBuf::from(format!("/.vol/{}/{}/llama.sock", meta.dev(), meta.ino()));
    let mut args = args.to_vec();
    // llama.cpp accepts Unix paths ending in .sock. Pass the override last, including when
    // the caller relied on the default --host value. The kernel denies all child TCP binds.
    args.extend(["--host".into(), "./llama.sock".into()]);
    let mut cmd = command(program, &args, policy)?;
    let rt = tokio::runtime::Handle::try_current().map_err(|_| "listener requires a Tokio runtime")?;
    let listener = TcpListener::bind(("127.0.0.1", *port)).map_err(|_| "could not bind requested loopback port")?;
    listener.set_nonblocking(true).map_err(|_| "listener setup failed")?;
    let listener = tokio::net::TcpListener::from_std(listener).map_err(|_| "listener setup failed")?;
    let (send, recv) = std::os::unix::net::UnixStream::pair().map_err(|_| "listener setup failed")?;
    recv.set_nonblocking(true).map_err(|_| "listener setup failed")?;
    let mut recv = UnixStream::from_std(recv).map_err(|_| "listener setup failed")?;
    // The CLOEXEC pair carries the PID from the trusted pre-exec hook. No relay descriptors
    // survive exec. Dropping an unused Command closes send and releases the reserved port.
    unsafe {
        cmd.pre_exec(move || {
            let pid = libc::getpid().to_le_bytes();
            if libc::write(send.as_raw_fd(), pid.as_ptr().cast(), pid.len()) != pid.len() as isize {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    rt.spawn(async move{
        let _directory=dir;
        let Ok(pid)=recv.read_i32_le().await else{return};
        let exit=match watch_exit(pid){Ok(exit)=>exit, Err(e)=>{log::warn!("Could not monitor local model: {e}");return}};
        let mut connections=JoinSet::new();
        loop{tokio::select!{
            _=exit.readable()=>break,
            Some(_)=connections.join_next(), if !connections.is_empty()=>{},
            accepted=listener.accept(), if connections.len()<32=>{
                let Ok((mut tcp,_))=accepted else{break};
                let socket=socket.clone();
                connections.spawn(async move{
                    if !socket.symlink_metadata().is_ok_and(|m|std::os::unix::fs::FileTypeExt::is_socket(&m.file_type())){return}
                    if let Ok(mut unix)=UnixStream::connect(socket).await && peer_pid(&unix)==Some(pid){
                        let _=tokio::io::copy_bidirectional(&mut tcp,&mut unix).await;
                    }
                });
            }
        }}
    });
    Ok(cmd.into())
}

fn peer_pid(s: &UnixStream) -> Option<i32> {
    let mut pid = 0i32;
    let mut len = mem::size_of_val(&pid) as libc::socklen_t;
    let ok = unsafe {
        libc::getsockopt(s.as_raw_fd(), libc::SOL_LOCAL, libc::LOCAL_PEERPID, (&mut pid as *mut i32).cast(), &mut len)
    };
    (ok == 0 && len as usize == mem::size_of_val(&pid)).then_some(pid)
}

fn watch_exit(pid: i32) -> io::Result<AsyncFd<OwnedFd>> {
    // EVFILT_PROC tracks this process instance even if its PID is subsequently reused.
    let fd = unsafe { libc::kqueue() };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let queue = unsafe { OwnedFd::from_raw_fd(fd) };
    let event = libc::kevent {
        ident: pid as usize,
        filter: libc::EVFILT_PROC,
        flags: libc::EV_ADD | libc::EV_ONESHOT,
        fflags: libc::NOTE_EXIT,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    if unsafe { libc::kevent(fd, &event, 1, std::ptr::null_mut(), 0, std::ptr::null()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // The queue owns a unique descriptor which is never closed or replaced while registered.
    Ok(unsafe { AsyncFd::register_with_interest(queue, tokio::io::Interest::READABLE) }?)
}
