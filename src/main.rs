extern crate getopts;
extern crate libc;
extern crate once_cell;
extern crate slab;

mod control_session;
mod poll;
mod signal;
mod sys;
mod unit_manager;
mod util;

use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::ptr;
use std::time::Instant;

use control_session::ControlSession;
use poll::Reaction;
use poll::{PollEvents, Poller};

#[derive(Default, Debug)]
struct Args {
    control_socket: Option<String>,
    units: String,
    help: bool,
    kill_on_exit: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(()) => ExitCode::FAILURE,
    }
}

enum PollItem {
    SignalReceiver(signal::SignalReceiver),
    UnixListener(UnixListener),
    ControlSession(ControlSession),
}

fn run() -> Result<(), ()> {
    let args = parse_args()?;
    if args.help {
        return Ok(());
    }

    let mut pollfds = Poller::<PollItem>::new();

    let signal_receiver = signal::install_signal_handler().map_err(|err| {
        eprintln!("Error installing signal handler: {}", err);
    })?;
    let signal_receiver_ref = pollfds.add(
        signal_receiver.as_raw_fd(),
        PollEvents::INPUT,
        PollItem::SignalReceiver(signal_receiver),
    );

    let control_sock_path = get_control_sock_path(&args);
    let control_sock_listener = setup_control_sock_listener(&control_sock_path)?;
    let _control_sock_remover = util::defer(|| {
        let _ = fs::remove_file(&control_sock_path);
    });
    pollfds.add(
        control_sock_listener.as_raw_fd(),
        PollEvents::INPUT,
        PollItem::UnixListener(control_sock_listener),
    );

    let mut unit_manager = unit_manager::UnitManager::new(args.units.clone());
    let mut sigchld_count = 0;

    for unit_name in list_units(&PathBuf::from(&args.units)) {
        if unit_name.contains('.') {
            continue;
        }
        unit_manager.add(unit_name.clone()).expect("duplicate unit during initial listing");
        if let Err(err) = unit_manager.start(&unit_name) {
            eprintln!("Failed to start unit '{}': {}", unit_name, err);
        };
    }

    let mut cont_event_loop = true;
    let mut new_sessions = vec![];
    let mut timeout = None;
    while cont_event_loop {
        if let Err(err) = pollfds.poll(timeout) {
            eprintln!("Error polling: {}", err);
            continue;
        };

        if timeout.is_some() {
            timeout = unit_manager.poll_timers();
        }

        pollfds.react(|poll_entry| {
            match poll_entry.user_item {
                PollItem::SignalReceiver(signal_receiver) => {
                    let counters = signal_receiver.receive();
                    if counters.sigint > 0 || counters.sigterm > 0 {
                        cont_event_loop = false;
                    }
                    if counters.sigchld != sigchld_count {
                        sigchld_count = counters.sigchld;
                        timeout = unit_manager.poll_units();
                    }
                }
                PollItem::UnixListener(control_sock_listener) => {
                    match sys::retry_eintr(|| control_sock_listener.accept()) {
                        Ok((stream, _addr)) => {
                            if configure_unix_stream(&stream).is_ok() {
                                new_sessions.push(ControlSession::new(stream));
                            }
                        }
                        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
                        Err(err) => {
                            eprintln!("Error on control socket accept: {}", err);
                        }
                    }
                }
                PollItem::ControlSession(session) => {
                    if session.poll_output().is_err()
                        || session.poll_input(&mut unit_manager).is_err()
                    {
                        return Some(Reaction::Remove);
                    }
                    return Some(Reaction::Events(session.events()));
                }
            };
            None
        });

        for session in new_sessions.drain(..) {
            pollfds.add(session.as_raw_fd(), PollEvents::INPUT, PollItem::ControlSession(session));
        }
    }

    if args.kill_on_exit {
        let Some((_, PollItem::SignalReceiver(signal_receiver))) =
            pollfds.remove(signal_receiver_ref)
        else {
            panic!("signal receiver not found in pollfds");
        };
        kill_on_exit(signal_receiver, sigchld_count, &unit_manager);
    }

    Ok(())
}

impl TryFrom<getopts::Matches> for Args {
    type Error = ();
    fn try_from(mut matches: getopts::Matches) -> Result<Args, ()> {
        let mut args = Args {
            control_socket: matches.opt_str("control-socket"),
            help: matches.opt_present("help"),
            kill_on_exit: matches.opt_present("kill-on-exit"),
            ..Default::default()
        };

        if matches.free.len() == 1 {
            args.units = matches.free.pop().unwrap();
            Ok(args)
        } else if !args.help {
            eprintln!("Error: Expected exactly one free argument.");
            Err(())
        } else {
            Ok(args)
        }
    }
}

impl util::Args for Args {
    fn need_help(&self) -> bool {
        self.help
    }
}

fn parse_args() -> Result<Args, ()> {
    let mut opts = getopts::Options::new();
    opts.optopt(
        "s",
        "control-socket",
        "path to control socket (default: path/to/units/control.sock)",
        "CONTROL_SOCK",
    );
    opts.optflag("k", "kill-on-exit", "kill all units on exit");
    util::parse_args("Usage: revisor [options] <path/to/units>", opts)
}

fn get_control_sock_path(args: &Args) -> PathBuf {
    args.control_socket
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| [&args.units, "control.sock"].iter().collect())
}

fn setup_control_sock_listener(control_sock_path: &Path) -> Result<UnixListener, ()> {
    if let Err(err) = fs::remove_file(&control_sock_path) {
        if err.kind() != io::ErrorKind::NotFound {
            eprintln!(
                "Error removing a previously created control socket: {}: {}",
                control_sock_path.display(),
                err
            );
            return Err(());
        }
    }

    let control_sock_listener = UnixListener::bind(&control_sock_path).map_err(|err| {
        eprintln!("Error listening to control socket: {}: {}", control_sock_path.display(), err);
    })?;
    sys::set_cloexec(control_sock_listener.as_raw_fd()).map_err(|err| {
        eprintln!("Error setting O_CLOEXEC on control socket: {}", err);
    })?;
    control_sock_listener.set_nonblocking(true).map_err(|err| {
        eprintln!("Error making control socket non-blocking: {}", err);
    })?;

    Ok(control_sock_listener)
}

fn list_units(units_path: &Path) -> impl Iterator<Item = String> {
    units_path
        .read_dir()
        .inspect_err(|err| eprintln!("Error reading unit directory: {}", err))
        .into_iter()
        .flatten()
        .filter_map(|unit_dir| {
            let unit_dir = unit_dir
                .inspect_err(|err| eprintln!("Error reading unit directory entry: {}", err))
                .ok()?;
            unit_dir
                .file_name()
                .into_string()
                .inspect_err(|err| eprintln!("Error converting file name: {:?}", err))
                .ok()
        })
}

fn configure_unix_stream(stream: &UnixStream) -> Result<(), ()> {
    stream.set_nonblocking(true).map_err(|err| {
        eprintln!("Error making unix stream non-blocking: {}", err);
    })?;
    sys::set_cloexec(stream.as_raw_fd()).map_err(|err| {
        eprintln!("Error setting O_CLOEXEC on unix stream: {}", err);
    })
}

fn kill_on_exit(
    signal_receiver: signal::SignalReceiver,
    mut sigchld_count: usize,
    unit_manager: &unit_manager::UnitManager,
) {
    let mut exit_pollfds = Poller::<()>::new();
    exit_pollfds.add(signal_receiver.as_raw_fd(), PollEvents::INPUT, ());

    let mut awaited_units: HashMap<_, _> = unit_manager
        .alive_units()
        .filter(|(pid, unit_name)| {
            match sys::wrap_libc("kill", || unsafe { libc::kill(*pid, libc::SIGTERM) }) {
                Ok(_) => true,
                Err(err) => {
                    eprintln!(
                        "Failed to send SIGTERM to unit '{}' (pid {}): {}",
                        unit_name, pid, err
                    );
                    false
                }
            }
        })
        .collect();

    let mut time_left = Some(unit_manager::KILL_DELAY);
    let deadline = Instant::now() + unit_manager::KILL_DELAY;
    while !awaited_units.is_empty() && time_left.is_some() {
        if let Err(err) = exit_pollfds.poll(time_left) {
            eprintln!("Error polling during kill on exit, exiting early: {}", err);
            break;
        }

        let counters = signal_receiver.receive();
        if counters.sigchld != sigchld_count {
            sigchld_count = counters.sigchld;

            while let Ok(pid) = sys::wrap_libc("waitpid", || unsafe {
                libc::waitpid(-1, ptr::null_mut(), libc::WNOHANG)
            })
            .inspect_err(|err| {
                if err.io_err.raw_os_error() != Some(libc::ECHILD) {
                    eprintln!("Error: waitpid failed during kill on exit: {}", err);
                }
            }) {
                if pid == 0 {
                    break;
                }

                awaited_units.remove(&pid);
            }
        }

        time_left = deadline.checked_duration_since(Instant::now());
    }

    for (pid, unit_name) in &awaited_units {
        if let Err(err) = sys::wrap_libc("kill", || unsafe { libc::kill(*pid, libc::SIGKILL) }) {
            eprintln!("Failed to send SIGKILL to unit '{}' (pid {}): {}", unit_name, pid, err);
        }
    }
}
