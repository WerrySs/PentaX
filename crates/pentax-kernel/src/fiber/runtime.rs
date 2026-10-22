// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned, generation-bound fiber transfer admission.
// Adapted from the pinned Fiber namespace; see NOTICE.

use super::*;
use crate::Kernel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FiberKey {
    address: u64,
    generation: u64,
}
impl FiberKey {
    pub const fn address(&self) -> u64 {
        self.address
    }
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FiberTransferKind {
    Run,
    Switch,
    Return,
}
/// Pure service admission. This is not proof of any native context transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmittedFiberTransfer {
    serial: u64,
    thread: u64,
    kind: FiberTransferKind,
    from: Option<FiberKey>,
    target: Option<FiberKey>,
    argument: u64,
    output: u64,
    terminated: bool,
}
impl AdmittedFiberTransfer {
    pub const fn kind(&self) -> FiberTransferKind {
        self.kind
    }
    pub const fn from(&self) -> Option<FiberKey> {
        self.from
    }
    pub const fn target(&self) -> Option<FiberKey> {
        self.target
    }
    pub const fn argument(&self) -> u64 {
        self.argument
    }
}
#[derive(Debug)]
pub enum FiberAction {
    Returned(u64),
    Transfer(AdmittedFiberTransfer),
}
pub(super) struct Thread {
    pub(super) thread: u64,
    pub(super) current: Option<FiberKey>,
    run_output: u64,
}
impl Record {
    fn key(&self) -> FiberKey {
        FiberKey {
            address: self.address,
            generation: self.generation,
        }
    }
}
impl FiberState {
    fn record(&self, key: FiberKey) -> Result<usize, Failure> {
        self.records
            .iter()
            .position(|r| r.key() == key)
            .ok_or(Failure::Unsupported("stale fiber generation"))
    }
    fn object(
        &self,
        key: FiberKey,
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<[u8; OBJECT_BYTES], Failure> {
        let row = &self.records[self.record(key)?];
        let mut object = [0; OBJECT_BYTES];
        budget.read(guest, key.address, &mut object)?;
        let q = |offset| {
            u64::from_le_bytes(object[offset..offset + 8].try_into().expect("fixed field"))
        };
        if word(&object, 0) != MAGIC_START
            || word(&object, 204) != MAGIC_END
            || word(&object, 4) != row.state
            || q(8) != row.entry
            || q(16) != row.argument
            || q(24) != row.context
            || q(32) != row.size
        {
            return Err(Failure::Unsupported("mutated owned fiber object"));
        }
        Ok(object)
    }
    fn admit(
        &mut self,
        service: Service,
        thread: u64,
        args: CallArgs,
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<FiberAction, Failure> {
        budget.work(self.records.len() as u64 + self.threads.len() as u64 + 16)?;
        if self.pending.is_some() {
            return Err(Failure::Unsupported("pending fiber transfer"));
        }
        let current = self
            .threads
            .iter()
            .find(|r| r.thread == thread)
            .and_then(|r| r.current);
        let kind = match service {
            Service::FiberRun => FiberTransferKind::Run,
            Service::FiberSwitch => FiberTransferKind::Switch,
            Service::FiberReturnToThread => FiberTransferKind::Return,
            _ => return Err(Failure::Unsupported("fiber transfer role")),
        };
        let target = if kind != FiberTransferKind::Return {
            if args.0[0] == 0 {
                return Ok(FiberAction::Returned(INVALID));
            }
            let mut object = [0; OBJECT_BYTES];
            budget.read(guest, args.0[0], &mut object)?;
            if word(&object, 0) != MAGIC_START || word(&object, 204) != MAGIC_END {
                return Ok(FiberAction::Returned(INVALID));
            }
            let Some(row) = self.records.iter().find(|r| r.address == args.0[0]) else {
                return Ok(FiberAction::Returned(INVALID));
            };
            Some(row.key())
        } else {
            None
        };
        if (kind == FiberTransferKind::Run && current.is_some())
            || (kind != FiberTransferKind::Run && current.is_none())
        {
            return Ok(FiberAction::Returned(PERMISSION));
        }
        if let Some(target) = target {
            let index = self.record(target)?;
            if self.records[index].state != IDLE {
                return Ok(FiberAction::Returned(STATE));
            }
            self.object(target, guest, budget)?;
        }
        if let Some(current) = current {
            self.object(current, guest, budget)?;
        }
        if !self.threads.iter().any(|row| row.thread == thread) {
            if self.threads.len() >= self.maximum_threads {
                return Err(Failure::Limit("fiber thread roots"));
            }
            self.threads.push(Thread {
                thread,
                current: None,
                run_output: 0,
            });
        }
        let serial = self.next_serial;
        self.next_serial = serial
            .checked_add(1)
            .ok_or(Failure::Limit("fiber transfer serial"))?;
        let transfer = AdmittedFiberTransfer {
            serial,
            thread,
            kind,
            from: current,
            target,
            argument: if kind == FiberTransferKind::Return {
                args.0[0]
            } else {
                args.0[1]
            },
            output: if kind == FiberTransferKind::Return {
                args.0[1]
            } else {
                args.0[2]
            },
            terminated: false,
        };
        self.pending = Some(transfer);
        Ok(FiberAction::Transfer(transfer))
    }
    fn commit(
        &mut self,
        request: AdmittedFiberTransfer,
        saved: [u64; 10],
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if self.pending != Some(request) {
            return Err(Failure::Unsupported("stale fiber transfer"));
        }
        let thread = self
            .threads
            .iter()
            .position(|r| r.thread == request.thread)
            .ok_or(Failure::Unsupported("missing fiber thread root"))?;
        if self.threads[thread].current != request.from {
            return Err(Failure::Unsupported("changed current fiber"));
        }
        budget.work(self.records.len() as u64 + self.threads.len() as u64 + 64)?;
        let from_index = request.from.map(|key| self.record(key)).transpose()?;
        let target_index = request.target.map(|key| self.record(key)).transpose()?;
        let mut from_object = match request.from {
            Some(key) => self.object(key, guest, budget)?,
            None => [0; OBJECT_BYTES],
        };
        let mut target_object = match request.target {
            Some(key) => self.object(key, guest, budget)?,
            None => [0; OBJECT_BYTES],
        };
        if let Some(i) = from_index {
            if self.records[i].state != RUNNING {
                return Err(Failure::Unsupported("fiber not running"));
            }
            let state = if request.terminated { TERMINATED } else { IDLE };
            from_object[4..8].copy_from_slice(&state.to_le_bytes());
            from_object[72..80].copy_from_slice(
                &if request.terminated {
                    0
                } else {
                    end(self.records[i].address, 104)?
                }
                .to_le_bytes(),
            );
            if !request.terminated {
                for (n, value) in saved.iter().enumerate() {
                    from_object[104 + n * 8..112 + n * 8].copy_from_slice(&value.to_le_bytes());
                }
            }
            from_object[192..200].copy_from_slice(
                &if request.kind == FiberTransferKind::Return {
                    request.argument
                } else {
                    0
                }
                .to_le_bytes(),
            );
            from_object[200] = u8::from(!request.terminated);
        }
        let resumed_output = if let Some(i) = target_index {
            if self.records[i].state != IDLE {
                return Err(Failure::Unsupported("fiber target no longer idle"));
            }
            target_object[4..8].copy_from_slice(&RUNNING.to_le_bytes());
            if let Some((start, end)) = self.records[i].execution_stack {
                target_object[88..96].copy_from_slice(&start.to_le_bytes());
                target_object[96..104].copy_from_slice(&end.to_le_bytes());
            }

            target_object[184..192].copy_from_slice(&request.argument.to_le_bytes());
            target_object[192..200].fill(0);
            self.records[i].resume_output
        } else {
            None
        };
        let root_output = if request.kind == FiberTransferKind::Return {
            Some(self.threads[thread].run_output)
        } else {
            None
        };
        let argument = request.argument.to_le_bytes();
        // One native-backed transaction validates every output before effects.
        // Revisited saved outputs are checked now, never trusted from admission.
        let mut writes = Vec::new();
        writes
            .try_reserve_exact(4)
            .map_err(|_| Failure::Allocation)?;
        if let Some(key) = request.from {
            writes.push(GuestWrite {
                address: key.address,
                bytes: &from_object,
            });
        }
        if let Some(key) = request.target {
            writes.push(GuestWrite {
                address: key.address,
                bytes: &target_object,
            });
        }
        if let Some(output) = resumed_output.filter(|p| *p != 0) {
            writes.push(GuestWrite {
                address: output,
                bytes: &argument,
            });
        }
        if let Some(output) = root_output.filter(|p| *p != 0) {
            writes.push(GuestWrite {
                address: output,
                bytes: &argument,
            });
        }
        budget.write(guest, &writes)?;
        if let Some(i) = from_index {
            self.records[i].state = if request.terminated { TERMINATED } else { IDLE };
            self.records[i].resume_output = if request.terminated {
                None
            } else {
                Some(request.output)
            };
        }
        if let Some(i) = target_index {
            self.records[i].state = RUNNING;
            self.records[i].resume_output = None;
        }
        if request.kind == FiberTransferKind::Run {
            self.threads[thread].run_output = request.output;
        }
        self.threads[thread].current = request.target;
        self.pending = None;
        Ok(if request.terminated { STATE } else { 0 })
    }
}
impl Kernel {
    pub fn admit_fiber_transfer(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &impl GuestAccess,
    ) -> Result<FiberAction, Failure> {
        self.budget.attempt()?;
        self.fibers.admit(
            service,
            self.threads.active().context(),
            args,
            guest,
            &mut self.budget,
        )
    }
    /// These are retained source ABI fields, not a native execution capability.
    pub fn fiber_entry(&self, key: FiberKey) -> Result<(u64, u64, u64, u64), Failure> {
        let r = &self.fibers.records[self.fibers.record(key)?];
        Ok((r.entry, r.argument, r.context, r.size))
    }
    /// Bind checked runtime geometry; numeric bounds are not native authority.
    /// Actual owned mapping and stack/entry checks remain the core's obligation.
    pub fn bind_fiber_stack(
        &mut self,
        key: FiberKey,
        start: u64,
        size: u64,
    ) -> Result<(), Failure> {
        let index = self.fibers.record(key)?;
        if self.fibers.records[index].state != IDLE
            || start == 0
            || start & 15 != 0
            || size < 512
            || size & 15 != 0
        {
            return Err(Failure::Unsupported("fiber runtime stack geometry"));
        }
        let end = end(start, size)?;
        self.fibers.records[index].execution_stack = Some((start, end));
        Ok(())
    }
    pub fn fiber_may_resume(&self, key: FiberKey) -> bool {
        self.fibers
            .record(key)
            .is_ok_and(|i| self.fibers.records[i].state != TERMINATED)
    }
    pub fn active_fiber(&self) -> Option<FiberKey> {
        self.fibers
            .threads
            .iter()
            .find(|r| r.thread == self.threads.active().context())
            .and_then(|r| r.current)
    }
    /// Admit completion only after the native owner verifies an actual entry RET.
    pub fn admit_fiber_entry_return(&mut self) -> Result<AdmittedFiberTransfer, Failure> {
        if self.fibers.pending.is_some() {
            return Err(Failure::Unsupported("pending fiber transfer"));
        }
        let thread = self.threads.active().context();
        let from = self
            .active_fiber()
            .ok_or(Failure::Unsupported("missing running fiber"))?;
        let serial = self.fibers.next_serial;
        self.fibers.next_serial = serial
            .checked_add(1)
            .ok_or(Failure::Limit("fiber serial"))?;
        let request = AdmittedFiberTransfer {
            serial,
            thread,
            kind: FiberTransferKind::Return,
            from: Some(from),
            target: None,
            argument: 0,
            output: 0,
            terminated: true,
        };
        self.fibers.pending = Some(request);
        Ok(request)
    }
    /// Pure atomic ABI publication; callers must separately authenticate the
    /// actual CPU transfer. Request possession is never execution proof.
    pub fn commit_fiber_transfer(
        &mut self,
        request: AdmittedFiberTransfer,
        saved: [u64; 10],
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        self.fibers.commit(request, saved, guest, &mut self.budget)
    }
}
