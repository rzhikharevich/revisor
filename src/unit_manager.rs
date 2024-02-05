use std::cmp;
use std::collections::{BinaryHeap, HashMap, hash_map};
use std::fmt;
use std::io;
use std::num::NonZero;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use libc::pid_t;
use slab::Slab;

use crate::sys;

const RESTART_DELAY: Duration = Duration::from_secs(1);

// Delay until a previously sent SIGTERM is followed by a SIGKILL.
pub const KILL_DELAY: Duration = Duration::from_secs(5);

pub struct UnitManager {
    units_path: String,
    pid_to_key: HashMap<pid_t, usize>,
    name_to_key: HashMap<String, usize>,
    key_to_unit: Slab<Unit>,
    timers: BinaryHeap<Timer>,
}

struct Unit {
    pid: Option<NonZero<pid_t>>,
    want_alive: bool,
    want_removed: bool,
    termination_requested: bool,
    name: String,
}

pub struct UnitStatus {
    pub pid: Option<NonZero<pid_t>>,
    pub want_alive: bool,
}

struct Timer {
    instant: Instant,
    key: usize,
}

pub enum Error {
    NotFound,
    System(sys::Error),
    Io(io::Error),
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug)]
pub struct UnitKey(usize);

impl UnitManager {
    pub fn new(units_path: String) -> UnitManager {
        UnitManager {
            units_path,
            pid_to_key: HashMap::new(),
            name_to_key: HashMap::new(),
            key_to_unit: Slab::new(),
            timers: BinaryHeap::new(),
        }
    }

    #[allow(dead_code)]
    pub fn unit_key_by_name(&self, name: &str) -> Option<UnitKey> {
        self.name_to_key.get(name).map(|&key| UnitKey(key))
    }

    pub fn status(&self, name: &str) -> Option<UnitStatus> {
        let key = self.name_to_key.get(name)?;
        self.key_to_unit
            .get(*key)
            .map(|unit| UnitStatus { pid: unit.pid, want_alive: unit.want_alive })
    }

    pub fn alive_units(&self) -> impl Iterator<Item = (pid_t, &str)> {
        self.key_to_unit
            .iter()
            .filter_map(|(_, unit)| unit.pid.map(|pid| (pid.get(), unit.name.as_str())))
    }

    pub fn add(&mut self, name: String) -> Result<(), ()> {
        let hash_map::Entry::Vacant(entry) = self.name_to_key.entry(name) else {
            return Err(());
        };
        let name = entry.key().clone();
        let key = self.key_to_unit.insert(Unit {
            pid: None,
            want_alive: true,
            want_removed: false,
            termination_requested: false,
            name,
        });
        entry.insert(key);
        Ok(())
    }

    #[allow(dead_code)]
    pub fn remove(&mut self, name: &str) -> Result<(), Error> {
        let Some(&key) = self.name_to_key.get(name) else {
            return Err(Error::NotFound);
        };
        let unit = self.key_to_unit.get_mut(key).expect("name_to_key maps to nonexistent key");
        if let Some(pid) = unit.pid {
            unit.want_alive = false;
            unit.want_removed = true;
            unit.termination_requested = true;
            UnitManager::terminate_unit(&mut self.timers, key, pid.get())
                .map_err(|err| Error::System(err))?;
        } else {
            self.name_to_key.remove(name);
            self.key_to_unit.remove(key);
            self.timers.retain(|timer| timer.key != key);
        }
        Ok(())
    }

    pub fn start(&mut self, name: &str) -> Result<pid_t, Error> {
        let Some(&key) = self.name_to_key.get(name) else {
            return Err(Error::NotFound);
        };

        let unit = self.key_to_unit.get_mut(key).expect("name_to_key maps to nonexistent key");
        unit.want_alive = true;
        if let Some(pid) = unit.pid {
            return Ok(pid.get() as pid_t);
        }

        match UnitManager::start_unit(&self.units_path, &mut self.pid_to_key, key, unit) {
            Ok(pid) => Ok(pid),
            Err(err) => {
                UnitManager::set_unit_timer(&mut self.timers, key, Instant::now() + RESTART_DELAY);
                Err(Error::Io(err))
            }
        }
    }

    pub fn stop(&mut self, name: &str) -> Result<(), Error> {
        let Some(&key) = self.name_to_key.get(name) else {
            return Err(Error::NotFound);
        };
        let unit = self.key_to_unit.get_mut(key).expect("name_to_key maps to a nonexistent key");
        unit.want_alive = false;
        unit.termination_requested = true;
        if let Some(pid) = unit.pid {
            UnitManager::terminate_unit(&mut self.timers, key, pid.get())
                .map_err(|err| Error::System(err))?;
        }
        Ok(())
    }

    pub fn poll_units(&mut self) -> Option<Duration> {
        let now = Instant::now();
        let mut status = 0i32;

        while let Ok(pid) =
            sys::wrap_libc("waitpid", || unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) })
                .inspect_err(|err| {
                    if err.io_err.raw_os_error() != Some(libc::ECHILD) {
                        eprintln!("Error: waitpid failed: {}", err);
                    }
                })
        {
            if pid == 0 {
                break;
            }

            self.notify_terminated(pid, now);
        }

        self.timers.peek().map(|timer| timer.instant.saturating_duration_since(now))
    }

    pub fn poll_timers(&mut self) -> Option<Duration> {
        let now = Instant::now();
        while let Some(timer) = self.timers.peek() {
            if timer.instant > now {
                break;
            }

            let key = timer.key;
            self.timers.pop();
            let unit = self.key_to_unit.get_mut(key).expect("timer refers to nonexistent key");

            if unit.want_alive && unit.pid.is_none() {
                if let Err(err) =
                    UnitManager::start_unit(&self.units_path, &mut self.pid_to_key, key, unit)
                {
                    eprintln!("Error restarting unit '{}': {}", unit.name, err);
                    self.timers.push(Timer { instant: now + RESTART_DELAY, key });
                }
            }

            if let Some(pid) = unit.pid {
                if !unit.want_alive {
                    if let Err(err) =
                        sys::wrap_libc("kill", || unsafe { libc::kill(pid.get(), libc::SIGKILL) })
                    {
                        eprintln!(
                            "Failed to send SIGKILL to unit '{}' with pid {}: {}",
                            unit.name, pid, err
                        );
                        self.timers.push(Timer { instant: now + KILL_DELAY, key })
                    } else if unit.want_removed {
                        self.name_to_key.remove(&unit.name);
                        self.key_to_unit.remove(key);
                        self.timers.retain(|timer| timer.key != key);
                    }
                }
            }
        }

        self.timers.peek().map(|timer| timer.instant.saturating_duration_since(now))
    }

    fn notify_terminated(&mut self, pid: pid_t, now: Instant) {
        let Some(key) = self.pid_to_key.remove(&pid) else {
            eprintln!("Warning: Got termination notification about unrecognized process {}", pid);
            return;
        };

        let unit = self.key_to_unit.get_mut(key).expect("pid_to_key maps to a nonexistent key");
        unit.pid = None;
        self.timers.retain(|timer| timer.key != key);
        if unit.want_alive {
            let restart_with_delay = if unit.termination_requested {
                UnitManager::start_unit(&self.units_path, &mut self.pid_to_key, key, unit).is_err()
            } else {
                true
            };

            if restart_with_delay {
                self.timers.push(Timer { instant: now + RESTART_DELAY, key });
            }
        }
        unit.termination_requested = false;
    }

    fn start_unit(
        units_path: &str,
        pid_to_key: &mut HashMap<pid_t, usize>,
        key: usize,
        unit: &mut Unit,
    ) -> Result<pid_t, io::Error> {
        let exec_path: PathBuf = [units_path, &unit.name, "run"].iter().collect();
        let mut command = Command::new(exec_path);
        let child = command.spawn()?;

        pid_to_key.insert(child.id() as pid_t, key);
        unit.pid = NonZero::new(child.id() as pid_t);

        Ok(child.id() as pid_t)
    }

    fn terminate_unit(
        timers: &mut BinaryHeap<Timer>,
        key: usize,
        pid: pid_t,
    ) -> Result<(), sys::Error> {
        UnitManager::set_unit_timer(timers, key, Instant::now() + KILL_DELAY);
        sys::wrap_libc("kill", || unsafe { libc::kill(pid, libc::SIGTERM) })?;
        Ok(())
    }

    fn set_unit_timer(timers: &mut BinaryHeap<Timer>, key: usize, instant: Instant) {
        timers.retain(|timer| timer.key != key);
        timers.push(Timer { instant, key });
    }
}

impl Eq for Timer {}

impl PartialEq for Timer {
    fn eq(&self, other: &Timer) -> bool {
        self.instant == other.instant
    }
}

impl Ord for Timer {
    fn cmp(&self, other: &Timer) -> cmp::Ordering {
        self.instant.cmp(&other.instant).reverse()
    }
}

impl PartialOrd for Timer {
    fn partial_cmp(&self, other: &Timer) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound => write!(f, "unit not found"),
            Error::System(err) => err.fmt(f),
            Error::Io(err) => err.fmt(f),
        }
    }
}
