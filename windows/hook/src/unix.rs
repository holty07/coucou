//! The Linux side of the relay: where the socket lives, and who is on the
//! other end of it.
//!
//! The socket sits in `$XDG_RUNTIME_DIR/coucou/`, a directory only we can enter,
//! so no other account can put a socket there under our name. Once connected we
//! still check that the peer runs as our uid before sending anything, which is
//! the same defence the Windows relay applies to its pipe.

use std::path::PathBuf;

/// `$XDG_RUNTIME_DIR/coucou/hook.sock`. Must match `pipe::socket_path()` in the app.
pub fn socket_path() -> PathBuf {
    runtime_dir().join("hook.sock")
}

/// `$XDG_RUNTIME_DIR/coucou`, falling back to `/run/user/<uid>/coucou`, then to
/// `/tmp/coucou-<uid>` when there is no runtime directory at all (a bare SSH login).
pub fn runtime_dir() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join("coucou");
    }
    let run = PathBuf::from(format!("/run/user/{uid}"));
    if run.is_dir() {
        return run.join("coucou");
    }
    PathBuf::from(format!("/tmp/coucou-{uid}"))
}

/// True when the process on the other end of `fd` runs as the same user we do.
/// Anything we cannot verify counts as "not ours".
pub fn peer_is_same_user(fd: std::os::fd::RawFd) -> bool {
    let mut cred = libc::ucred { pid: 0, uid: u32::MAX, gid: u32::MAX };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    rc == 0 && cred.uid == unsafe { libc::getuid() }
}
