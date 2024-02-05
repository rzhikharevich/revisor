use std::fmt;
use std::io;
use std::mem::{self, MaybeUninit};
use std::os::fd::RawFd;
use std::ptr;

use libc::c_int;

use crate::util;

#[derive(Debug)]
pub struct Error {
    pub syscall_name: &'static str,
    pub io_err: io::Error,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} failed: {}", self.syscall_name, self.io_err)
    }
}

pub fn wrap_libc<Ret, F>(syscall_name: &'static str, f: F) -> Result<Ret, Error>
where
    Ret: util::IsNegative,
    F: FnOnce() -> Ret,
{
    let result = f();
    if result.is_negative() {
        Err(Error { syscall_name, io_err: io::Error::last_os_error() })
    } else {
        Ok(result)
    }
}

pub fn wrap_libc_retry_eintr<Ret, F>(syscall_name: &'static str, mut f: F) -> Result<Ret, Error>
where
    Ret: util::IsNegative,
    F: FnMut() -> Ret,
{
    loop {
        match wrap_libc(syscall_name, &mut f) {
            Ok(ret) => return Ok(ret),
            Err(err) => {
                if err.io_err.kind() != io::ErrorKind::Interrupted {
                    return Err(err);
                }
            }
        }
    }
}

pub fn retry_eintr<Ret, F>(mut f: F) -> io::Result<Ret>
where
    F: FnMut() -> io::Result<Ret>,
{
    loop {
        match f() {
            Ok(ret) => return Ok(ret),
            Err(err) => {
                if err.kind() != io::ErrorKind::Interrupted {
                    return Err(err);
                }
            }
        }
    }
}

pub fn pipe() -> Result<(RawFd, RawFd), Error> {
    let mut fds = [-1 as RawFd, -1];
    wrap_libc("pipe", || unsafe { libc::pipe(fds.as_mut_ptr()) })?;
    Ok((fds[0], fds[1]))
}

pub fn set_cloexec(fd: RawFd) -> Result<(), Error> {
    unsafe {
        let flags = wrap_libc("fcntl(F_GETFD)", || libc::fcntl(fd, libc::F_GETFD))?;
        wrap_libc("fcntl(F_SETFD)", || libc::fcntl(fd, libc::F_SETFD, flags | libc::O_CLOEXEC))?;
    }

    Ok(())
}

pub fn set_nonblock(fd: RawFd) -> Result<(), Error> {
    unsafe {
        let flags = wrap_libc("fcntl(F_GETFL)", || libc::fcntl(fd, libc::F_GETFL))?;
        wrap_libc("fcntl(F_SETFL)", || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK))?;
    }

    Ok(())
}

pub type SignalHandler = extern "C" fn(c_int);

// SAFETY: f must be an async signal safe function.
pub unsafe fn sigaction(signum: c_int, f: SignalHandler) -> Result<(), Error> {
    let mut sa = unsafe { mem::zeroed::<libc::sigaction>() };
    sa.sa_sigaction = f as usize;
    sa.sa_mask = sigemptyset()?;
    wrap_libc("sigaction", || unsafe {
        libc::sigaction(signum, &mut sa as *mut _, ptr::null_mut())
    })?;
    Ok(())
}

fn sigemptyset() -> Result<libc::sigset_t, Error> {
    unsafe {
        let mut sigset = MaybeUninit::<libc::sigset_t>::zeroed();
        wrap_libc("sigemptyset", || libc::sigemptyset(sigset.as_mut_ptr()))?;
        Ok(sigset.assume_init())
    }
}
