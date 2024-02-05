use std::os::fd::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering::SeqCst};

use libc::{c_int, c_void};
use once_cell::sync::Lazy;

use crate::sys;

struct SignalHandlerContext {
    write_fd: AtomicI32,
    signal_counters: AtomicSignalCounters,
}

#[derive(Debug)]
pub struct SignalCountersImpl<T> {
    pub sigchld: T,
    pub sigint: T,
    pub sigterm: T,
}

impl<T> SignalCountersImpl<T> {
    fn by_posix_signal(&self, sig: c_int) -> Option<&T> {
        match sig {
            libc::SIGCHLD => Some(&self.sigchld),
            libc::SIGINT => Some(&self.sigint),
            libc::SIGTERM => Some(&self.sigterm),
            _ => None,
        }
    }
}

pub type SignalCounters = SignalCountersImpl<usize>;
type AtomicSignalCounters = SignalCountersImpl<AtomicUsize>;

#[derive(Clone)]
pub struct SignalReceiver {
    read_fd: RawFd,
}

impl SignalReceiver {
    pub fn accept(&self) {
        let mut buf = [0_i8; 32];
        while let Ok(num_read) = sys::wrap_libc_retry_eintr("read", || unsafe {
            libc::read(self.read_fd, buf.as_mut_ptr() as *mut c_void, buf.len())
        }) {
            if num_read == 0 {
                break;
            }
        }
    }

    pub fn receive(&self) -> SignalCounters {
        self.accept();
        SignalCounters::load()
    }
}

impl AsRawFd for SignalReceiver {
    fn as_raw_fd(&self) -> RawFd {
        self.read_fd
    }
}

static SIGNAL_HANDLER_CONTEXT: SignalHandlerContext = SignalHandlerContext {
    write_fd: AtomicI32::new(-1),
    signal_counters: AtomicSignalCounters {
        sigchld: AtomicUsize::new(0),
        sigint: AtomicUsize::new(0),
        sigterm: AtomicUsize::new(0),
    },
};

static SIGNAL_RECEIVER: Lazy<Result<SignalReceiver, String>> = Lazy::new(|| {
    let (read_fd, write_fd) = sys::pipe().map_err(|err| err.to_string())?;

    for (label, fd) in [("read", read_fd), ("write", write_fd)] {
        sys::set_cloexec(fd)
            .map_err(|err| format!("failed to set O_CLOEXEC on {} fd {}: {}", label, fd, err))?;
        sys::set_nonblock(fd)
            .map_err(|err| format!("failed to set O_NONBLOCK on {} fd {}: {}", label, fd, err))?;
    }

    SIGNAL_HANDLER_CONTEXT.write_fd.store(write_fd, SeqCst);

    for signum in [libc::SIGCHLD, libc::SIGINT, libc::SIGTERM] {
        unsafe { sys::sigaction(signum, signal_handler) }
            .map_err(|err| format!("failed to install handler for signal {}: {}", signum, err))?
    }

    Ok(SignalReceiver { read_fd })
});

impl SignalCounters {
    pub fn load() -> SignalCounters {
        let counters = &SIGNAL_HANDLER_CONTEXT.signal_counters;
        SignalCounters {
            sigchld: counters.sigchld.load(SeqCst),
            sigint: counters.sigint.load(SeqCst),
            sigterm: counters.sigterm.load(SeqCst),
        }
    }
}

pub fn install_signal_handler() -> Result<SignalReceiver, &'static str> {
    SIGNAL_RECEIVER.as_ref().cloned().map_err(|err| err.as_str())
}

extern "C" fn signal_handler(sig: c_int) {
    // POSIX signal handlers must be "async signal safe", so special caution must be taken to only
    // use async signal safe library calls.

    let write_fd = SIGNAL_HANDLER_CONTEXT.write_fd.load(SeqCst);
    if write_fd < 0 {
        return;
    }

    let Some(counter) = SIGNAL_HANDLER_CONTEXT.signal_counters.by_posix_signal(sig) else {
        return;
    };
    counter.fetch_add(1, SeqCst);

    let buf: [i8; 1] = [0];
    unsafe {
        libc::write(write_fd, buf.as_ptr() as *const c_void, buf.len());
    }
}
