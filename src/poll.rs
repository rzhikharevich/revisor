#![allow(dead_code)]

use std::io;
use std::ops;
use std::os::fd::RawFd;
use std::time::Duration;

use slab::Slab;

use crate::sys;

#[derive(Copy, Clone, Debug)]
pub struct PollRef(usize);

pub struct Poller<UserItem> {
    // A Vec of pollfd for passing to poll().
    pollfds: Vec<libc::pollfd>,
    // Each ith element of entries contains the slab key and user item associated with ith pollfd.
    entries: Vec<PollerEntry<UserItem>>,
    key_to_index: Slab<usize>,
}

struct PollerEntry<UserItem> {
    key: usize,
    user_item: UserItem,
}

impl<UserItem> Poller<UserItem> {
    pub fn new() -> Poller<UserItem> {
        Poller { pollfds: vec![], entries: vec![], key_to_index: Slab::new() }
    }

    pub fn num_items(&self) -> usize {
        self.pollfds.len()
    }

    pub fn add(&mut self, fd: RawFd, mode: PollEvents, user_item: UserItem) -> PollRef {
        let key = self.key_to_index.insert(self.pollfds.len());
        self.pollfds.push(libc::pollfd { fd, events: mode.bits, revents: 0 });
        self.entries.push(PollerEntry { key, user_item });
        PollRef(key)
    }

    pub fn remove(&mut self, poll_ref: PollRef) -> Option<(RawFd, UserItem)> {
        let slab_index = poll_ref.0;
        self.key_to_index.try_remove(slab_index).map(|index| {
            // swap_remove removes an element within O(1) time by swapping the given element with
            // the last.
            let fd = self.pollfds.swap_remove(index).fd;
            let entry = self.entries.swap_remove(index);
            if index < self.entries.len() {
                // The above checks if the removed element already was the last in which case we
                // don't need to adjust any pollfd_index in the slab.
                self.key_to_index[self.entries[index].key] = index;
            }
            (fd, entry.user_item)
        })
    }

    pub fn poll(&mut self, timeout: Option<Duration>) -> Result<(), sys::Error> {
        if self.pollfds.is_empty() {
            return Err(sys::Error {
                syscall_name: "poll(NULL, 0, -1)",
                io_err: io::Error::from_raw_os_error(libc::EINVAL),
            });
        }

        let timeout = timeout.map_or(-1, |timeout| timeout.as_millis() as i32);
        sys::wrap_libc_retry_eintr("poll", || unsafe {
            libc::poll(self.pollfds.as_mut_ptr(), self.pollfds.len() as libc::nfds_t, timeout)
        })?;

        Ok(())
    }

    pub fn ready<'a>(&'a self) -> impl Iterator<Item = PollEntry<'a, UserItem>> {
        self.pollfds.iter().zip(self.entries.iter()).filter_map(|(pollfd, entry)| {
            if pollfd.revents != 0 {
                Some(PollEntry {
                    poll_ref: PollRef(entry.key),
                    fd: pollfd.fd,
                    events: PollEvents { bits: pollfd.revents },
                    user_item: &entry.user_item,
                })
            } else {
                None
            }
        })
    }

    pub fn react<R>(&mut self, mut reactor: R)
    where
        R: FnMut(PollEntryMut<UserItem>) -> Option<Reaction>,
    {
        let mut index = 0;
        while index < self.pollfds.len() {
            let pollfd = &mut self.pollfds[index];
            let mut keep = true;
            if pollfd.revents != 0 {
                let entry = &mut self.entries[index];
                match reactor(PollEntryMut {
                    poll_ref: PollRef(entry.key),
                    fd: pollfd.fd,
                    events: PollEvents { bits: pollfd.events },
                    user_item: &mut entry.user_item,
                }) {
                    Some(Reaction::Events(events)) => {
                        pollfd.events = events.bits;
                    }
                    Some(Reaction::Remove) => {
                        keep = false;
                    }
                    None => {}
                }
            }

            if keep {
                index += 1;
            } else {
                self.pollfds.swap_remove(index);
                let entry = self.entries.swap_remove(index);
                self.key_to_index.remove(entry.key);

                if index < self.pollfds.len() {
                    self.key_to_index[self.entries[index].key] = index;
                }
            }
        }
    }
}

#[derive(Copy, Clone)]
pub struct PollEvents {
    bits: i16,
}

impl PollEvents {
    pub const NONE: PollEvents = PollEvents { bits: 0 };
    pub const INPUT: PollEvents = PollEvents { bits: libc::POLLIN };
    pub const OUTPUT: PollEvents = PollEvents { bits: libc::POLLOUT };

    pub fn contains(self, events: PollEvents) -> bool {
        self.bits & events.bits != 0
    }
}

impl ops::BitOr for PollEvents {
    type Output = PollEvents;
    fn bitor(self, rhs: Self) -> PollEvents {
        PollEvents { bits: self.bits | rhs.bits }
    }
}

impl ops::BitOrAssign for PollEvents {
    fn bitor_assign(&mut self, rhs: PollEvents) {
        self.bits |= rhs.bits;
    }
}

pub enum Reaction {
    Events(PollEvents),
    Remove,
}

pub struct PollEntry<'a, UserItem> {
    pub poll_ref: PollRef,
    pub fd: RawFd,
    pub events: PollEvents,
    pub user_item: &'a UserItem,
}

pub struct PollEntryMut<'a, UserItem> {
    pub poll_ref: PollRef,
    pub fd: RawFd,
    pub events: PollEvents,
    pub user_item: &'a mut UserItem,
}
