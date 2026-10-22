// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned counts and atomic bounded guest transactions.
// Adapted from pinned kernel/semaphore.cpp/h; see NOTICE.

use crate::budget::Budget;
use crate::{CallArgs, Errno, Failure, GuestAccess, GuestWrite, Kernel, Limits, Service};

const MAX: u32 = i32::MAX as u32;

mod wait;
pub use wait::{PthreadSemaphoreWait, PthreadSemaphoreWaitAction};

#[derive(Clone, Copy)]
struct Semaphore {
    address: u64,
    count: u32,
}

pub(crate) struct SemaphoreState {
    entries: Vec<Semaphore>,
    waiters: Vec<wait::Waiter>,
    max_waiters: usize,
}

#[derive(Clone, Copy)]
enum Operation {
    Init,
    Destroy,
    Wait,
    TryWait,
    RelativeWait,
    AbsoluteWait,
    Post,
    GetValue,
}

enum Change {
    None,
    Set {
        index: Option<usize>,
        entry: Semaphore,
    },
    Remove(usize),
}

struct Payload {
    address: u64,
    bytes: [u8; 16],
    length: usize,
}

struct Action {
    change: Change,
    payloads: [Option<Payload>; 2],
    errno: Option<Errno>,
}

impl Action {
    fn result(errno: Option<Errno>) -> Self {
        Self {
            change: Change::None,
            payloads: [None, None],
            errno,
        }
    }
}

fn prefix(address: u64, count: u32, flags: Option<u32>, waiting: bool) -> Payload {
    let mut bytes = [0; 16];
    bytes[..2].copy_from_slice(&0x09fa_u16.to_le_bytes());
    bytes[4..8].copy_from_slice(&u32::from(waiting).to_le_bytes());
    bytes[8..12].copy_from_slice(&count.to_le_bytes());
    if let Some(flags) = flags {
        bytes[12..].copy_from_slice(&flags.to_le_bytes());
    }
    Payload {
        address,
        bytes,
        length: if flags.is_some() { 16 } else { 12 },
    }
}

impl SemaphoreState {
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn new(limits: Limits) -> Result<Self, Failure> {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        let mut waiters = Vec::new();
        waiters
            .try_reserve_exact(limits.max_thread_contexts)
            .map_err(|_| Failure::Allocation)?;
        Ok(Self {
            entries,
            waiters,
            max_waiters: limits.max_thread_contexts,
        })
    }

    fn plan(
        &self,
        op: Operation,
        args: CallArgs,
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<Action, Failure> {
        budget.work((self.entries.len() + 2 * self.waiters.len()) as u64 + 16)?;
        let [address, arg, value, ..] = args.0;
        // The absolute wrapper validates the timespec before looking up sem.
        let timeout = match op {
            Operation::AbsoluteWait => {
                if arg == 0 {
                    return Ok(Action::result(Some(Errno::Invalid)));
                }
                let mut bytes = [0; 16];
                budget.read(guest, arg, &mut bytes)?;
                let sec = i64::from_le_bytes(bytes[..8].try_into().unwrap());
                let ns = i64::from_le_bytes(bytes[8..].try_into().unwrap());
                if sec < 0 || !(0..1_000_000_000).contains(&ns) {
                    return Ok(Action::result(Some(Errno::Invalid)));
                }
                let now = match crate::time::realtime() {
                    Ok(now) => now,
                    Err(Failure::Errno(_)) => return Ok(Action::result(Some(Errno::Invalid))),
                    Err(error) => return Err(error),
                };
                // Preserve the source's separate signed microsecond terms,
                // including truncation toward zero of the nanosecond delta.
                let micros = (i128::from(sec) - i128::from(now.as_secs())) * 1_000_000
                    + (i128::from(ns) - i128::from(now.subsec_nanos())) / 1000;
                Some(micros.clamp(0, i128::from(u32::MAX)) as u32)
            }
            Operation::RelativeWait => Some(arg as u32),
            _ => None,
        };
        if address == 0 {
            return Ok(Action::result(Some(Errno::Invalid)));
        }
        let index = self
            .entries
            .iter()
            .position(|entry| entry.address == address);
        if matches!(op, Operation::Init) {
            let count = value as u32;
            if count > MAX {
                return Ok(Action::result(Some(Errno::Invalid)));
            }
            if self.has_waiters(address) {
                return Err(Failure::Unsupported(
                    "reinitializing a semaphore with retained waiters",
                ));
            }
            if index.is_none() {
                budget.can_add_handle()?;
            }
            let mut action = Action::result(None);
            action.payloads[0] = Some(prefix(address, count, Some(arg as u32), false));
            action.change = Change::Set {
                index,
                entry: Semaphore { address, count },
            };
            return Ok(action);
        }
        let Some(index) = index else {
            return Ok(Action::result(Some(Errno::Invalid)));
        };
        let mut entry = self.entries[index];
        let mut action = Action::result(None);
        match op {
            Operation::Destroy => {
                if self.has_waiters(address) {
                    return Err(Failure::Unsupported(
                        "destroying a semaphore with retained waiters",
                    ));
                }
                // The source does not clear nameid: no guest read is needed
                // to preserve it, and the two writes commit together.
                action.payloads[0] = Some(Payload {
                    address,
                    bytes: [0; 16],
                    length: 2,
                });
                action.payloads[1] = Some(Payload {
                    address: address
                        .checked_add(4)
                        .ok_or(crate::GuestError::AddressOverflow)?,
                    bytes: [0; 16],
                    length: 12,
                });
                action.change = Change::Remove(index);
            }
            Operation::GetValue => {
                if arg == 0 {
                    return Ok(Action::result(Some(Errno::Invalid)));
                }
                let mut bytes = [0; 16];
                bytes[..4].copy_from_slice(&entry.count.to_le_bytes());
                action.payloads[0] = Some(Payload {
                    address: arg,
                    bytes,
                    length: 4,
                });
            }
            Operation::Post if entry.count == MAX => {
                return Ok(Action::result(Some(Errno::Overflow)));
            }
            Operation::Post => {
                entry.count += 1;
                action.payloads[0] = Some(prefix(
                    address,
                    entry.count,
                    None,
                    self.has_waiters(address),
                ));
                action.change = Change::Set {
                    index: Some(index),
                    entry,
                };
            }
            Operation::Wait
            | Operation::TryWait
            | Operation::RelativeWait
            | Operation::AbsoluteWait => {
                if entry.count > 0 {
                    entry.count -= 1;
                    action.change = Change::Set {
                        index: Some(index),
                        entry,
                    };
                } else if matches!(op, Operation::TryWait) {
                    action.errno = Some(Errno::Again);
                } else if timeout == Some(0) {
                    action.errno = Some(Errno::TimedOut);
                } else {
                    return Err(Failure::Unsupported("POSIX semaphore blocking wait"));
                }
                action.payloads[0] = Some(prefix(
                    address,
                    entry.count,
                    None,
                    self.has_waiters(address),
                ));
            }
            Operation::Init => unreachable!(),
        }
        Ok(action)
    }

    fn commit(&mut self, change: Change, budget: &mut Budget) {
        match change {
            Change::None => {}
            Change::Set {
                index: Some(index),
                entry,
            } => self.entries[index] = entry,
            Change::Set { index: None, entry } => {
                self.entries.push(entry);
                budget.added_handle();
            }
            Change::Remove(index) => {
                self.entries.swap_remove(index);
                budget.removed_handle();
            }
        }
    }
}

impl Kernel {
    pub(crate) fn pthread_semaphore(
        &mut self,
        service: Service,
        mut args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        use Service::*;
        let (op, posix) = match service {
            PosixSemInit => (Operation::Init, true),
            PosixSemDestroy => (Operation::Destroy, true),
            PosixSemWait => (Operation::Wait, true),
            PosixSemTryWait => (Operation::TryWait, true),
            PosixSemRelativeWait => (Operation::RelativeWait, true),
            PosixSemTimedWait => (Operation::AbsoluteWait, true),
            PosixSemPost => (Operation::Post, true),
            PosixSemGetValue => (Operation::GetValue, true),
            PthreadSemInit => (Operation::Init, false),
            PthreadSemDestroy => (Operation::Destroy, false),
            PthreadSemWait => (Operation::Wait, false),
            PthreadSemTryWait => (Operation::TryWait, false),
            PthreadSemTimedWait => (Operation::RelativeWait, false),
            PthreadSemPost => (Operation::Post, false),
            PthreadSemGetValue => (Operation::GetValue, false),
            _ => return Err(Failure::Unsupported("pthread semaphore service")),
        };
        if posix {
            guest.preflight_posix_errno(None)?;
        }
        let action = if service == PthreadSemInit && args.0[1] as u32 != 0 {
            Action::result(Some(Errno::Invalid))
        } else {
            // The direct init name is only a source diagnostic label, not a
            // file lookup, host string or named synchronization capability.
            if service == PthreadSemInit {
                args.0[1] = 0;
            }
            self.pthread_semaphores
                .plan(op, args, guest, &mut self.budget)?
        };
        let errno_output = if posix && action.errno.is_some() {
            Some(self.errno_address(guest)?)
        } else {
            None
        };
        let errno_bytes = (action.errno.unwrap_or(Errno::Invalid) as u32).to_le_bytes();
        let empty = GuestWrite {
            address: 0,
            bytes: &[],
        };
        let mut writes = [empty; 3];
        let mut count = 0;
        for payload in action.payloads.iter().flatten() {
            writes[count] = GuestWrite {
                address: payload.address,
                bytes: &payload.bytes[..payload.length],
            };
            count += 1;
        }
        if let Some(address) = errno_output {
            writes[count] = GuestWrite {
                address,
                bytes: &errno_bytes,
            };
            count += 1;
        }
        if count > 0 {
            self.budget.write(guest, &writes[..count])?;
        }
        self.pthread_semaphores
            .commit(action.change, &mut self.budget);
        match action.errno {
            None => Ok(0),
            Some(_) if posix => Ok(u64::from(u32::MAX)),
            Some(errno) => Err(errno.into()),
        }
    }
}
