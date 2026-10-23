// SPDX-License-Identifier: GPL-2.0-only
//! Owned asynchronous compute submissions. Only queue fronts execute; a false
//! memory predicate retains its exact command index, never a replayable root.
use super::*;
#[path = "compute_execution/exclusions.rs"]
mod exclusions;
use pentax_platform::native_memory::NativeOwnerId;
use std::{collections::VecDeque, mem::size_of};

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "compute_execution/tests.rs"]
mod tests;

pub(crate) struct ComputePump {
    pub pending: usize,
}

pub(super) struct ComputeQueues {
    queues: [VecDeque<Submission>; 56],
    owner: Option<NativeOwnerId>,
    pending: usize,
    next: usize,
    retained: usize,
    ranges: Vec<compute::CommandRange>,
    owners: Vec<u64>,
    generation: u64,
    slots: usize,
}
impl Default for ComputeQueues {
    fn default() -> Self {
        Self {
            queues: std::array::from_fn(|_| VecDeque::new()),
            owner: None,
            pending: 0,
            next: 0,
            retained: 0,
            ranges: Vec::new(),
            owners: Vec::new(),
            generation: 0,
            slots: 0,
        }
    }
}
struct Submission {
    generation: u64,
    batch: u64,
    accepted: bool,
    had_effects: bool,
    slots: usize,
    root: compute::CommandRange,
    diagnostic_root: compute::CommandRange,
    initial: Option<compute::ComputeState>,
    sources: draw::CapturedComputeCommands,
    active: Option<Active>,
    limits: compute::CommandLimits,
    retained: usize,
}
struct Active {
    plan: register_execution::ComputeCommands,
    staging: Vec<u8>,
    pending_wait: Option<usize>,
}
enum Advance {
    Step,
    Waiting,
    Complete,
}

impl Driver {
    fn advance_compute(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        submission: &mut Submission,
        commands: &mut Vec<compute::CommandRange>,
        kernel: &mut Option<&mut pentax_kernel::Kernel>,
        queue: u32,
    ) -> Result<Advance, Failure> {
        let active = submission.active.as_mut().ok_or(Failure::Poisoned)?;
        let step = match active.pending_wait {
            Some(index) => register_execution::ComputeStep::Command(index),
            None => match active.plan.next_step()? {
                Some(step) => step,
                None => {
                    self.queues[(queue - 0x20) as usize] = active.plan.final_state()?;
                    return Ok(Advance::Complete);
                }
            },
        };
        let owned_dispatch;
        let command_index;
        let command = match step {
            register_execution::ComputeStep::Array => {
                let before = active.plan.usage();
                let count = active.plan.command_ranges().len();
                active.plan.resolve_array(memory)?;
                let after = active.plan.usage();
                self.compute_charge(after.work.checked_sub(before.work).ok_or(Failure::Limit)?)?;
                if after.host_bytes > before.host_bytes {
                    return Err(Failure::Limit);
                }
                for range in &active.plan.command_ranges()[count..] {
                    if commands.len() == commands.capacity()
                        || self.compute_queues.owners.len() == self.compute_queues.owners.capacity()
                    {
                        return Err(Failure::Limit);
                    }
                    commands.push(*range);
                    self.compute_queues.owners.push(submission.generation);
                }
                return Ok(Advance::Step);
            }
            register_execution::ComputeStep::Command(index) => {
                command_index = Some(index);
                active.plan.command(index)?
            }
            register_execution::ComputeStep::Dispatch(dispatch) => {
                command_index = None;
                owned_dispatch = dispatch;
                compute::Command::Dispatch(&owned_dispatch)
            }
        };
        let dispatch = match command {
            compute::Command::Synchronize(sync) => {
                self.synchronize_commands(sync)?;
                return Ok(Advance::Step);
            }
            compute::Command::Annotation(marker) => {
                self.annotation_command(marker)?;
                return Ok(Advance::Step);
            }
            compute::Command::Drain => {
                self.drain_commands()?;
                return Ok(Advance::Step);
            }
            compute::Command::Release(release) => {
                self.with_command_exclusions(commands, 0, |driver, protected| {
                    driver.retire_release(memory, release, protected, kernel, queue)
                })?;
                return Ok(Advance::Step);
            }
            compute::Command::Write(write) => {
                self.with_command_exclusions(commands, 0, |driver, protected| {
                    driver.memory_write(memory, write, protected)
                })?;
                return Ok(Advance::Step);
            }
            compute::Command::Transfer(transfer) => {
                // Shared accounting already includes this active plan/staging.
                let local = submission
                    .limits
                    .max_host_bytes
                    .checked_sub(submission.retained)
                    .and_then(|n| n.checked_sub(self.gds_retained_bytes()))
                    .ok_or(Failure::Limit)?;
                self.with_command_exclusions(commands, 0, |driver, protected| {
                    driver.transfer_command(
                        memory,
                        transfer,
                        protected,
                        &mut active.staging,
                        0,
                        local,
                    )
                })?;
                return Ok(Advance::Step);
            }
            compute::Command::Wait(wait) => {
                self.compute_charge(wait.byte_count() as u64 + 1)?;
                if active.pending_wait.is_none() {
                    self.usage.memory_commands = self.usage.memory_commands.saturating_add(1);
                }
                if !wait
                    .satisfied(&super::super::Memory(memory))
                    .map_err(Failure::Preparation)?
                {
                    active.pending_wait = command_index;
                    return Ok(Advance::Waiting);
                }
                active.pending_wait = None;
                return Ok(Advance::Step);
            }
            compute::Command::Dispatch(dispatch) => dispatch,
        };
        self.with_command_exclusions(commands, 0, |driver, protected| {
            driver.compute_dispatch(memory, dispatch, protected)
        })
    }
}

impl Driver {
    fn activate_compute(
        &mut self,
        submission: &mut Submission,
        index: usize,
    ) -> Result<(), Failure> {
        if submission.active.is_some() {
            return Ok(());
        }
        let mut limits = submission.limits;
        limits.max_work = limits.max_work.min(
            self.limits
                .max_preparation_work
                .saturating_sub(self.usage.preparation_work),
        );
        limits.max_dispatches = limits.max_dispatches.min(
            usize::try_from(
                self.limits
                    .max_dispatches
                    .saturating_sub(self.usage.dispatches),
            )
            .unwrap_or(usize::MAX),
        );
        let command_host = limits
            .max_host_bytes
            .min(self.limits.commands.max_host_bytes)
            .checked_sub(self.gds_retained_bytes())
            .and_then(|n| n.checked_sub(submission.retained))
            .ok_or(Failure::Preparation(compute::Error::Limit(
                "command host bytes",
            )))?;
        let owner_host = self
            .graphics_host_ceiling()?
            .min(self.limits.compute.max_host_bytes)
            .checked_sub(self.graphics_held_bytes()?)
            .ok_or(Failure::Limit)?;
        let owner_work = self
            .limits
            .max_preparation_work
            .saturating_sub(self.usage.preparation_work);
        let owner_host_binds = owner_host < command_host;
        let owner_work_binds = owner_work < submission.limits.max_work;
        limits.max_host_bytes = command_host.min(owner_host);
        let initial = self.queues[index];
        submission.initial = Some(initial);
        let plan = register_execution::ComputeCommands::prepare_source(
            &submission.sources,
            submission.root,
            initial,
            &limits,
        )
        .map_err(|error| compute_plan_error(error, owner_host_binds, owner_work_binds))?;
        self.compute_charge(plan.usage().work)?;
        let staging = self.transfer_staging(
            plan.usage().staging_bytes,
            plan.usage().write_bytes,
            plan.usage().host_bytes,
            limits.max_host_bytes,
        )?;
        let held = plan
            .usage()
            .host_bytes
            .checked_add(staging.capacity())
            .ok_or(Failure::Limit)?;
        let retained = self
            .compute_queues
            .retained
            .checked_add(held)
            .ok_or(Failure::Limit)?;
        self.graphics_held_bytes()?
            .checked_add(held)
            .filter(|n| {
                *n <= self.graphics_host_ceiling().unwrap_or(0)
                    && *n <= self.limits.compute.max_host_bytes
            })
            .ok_or(Failure::Limit)?;
        submission.retained = submission
            .retained
            .checked_add(held)
            .ok_or(Failure::Limit)?;
        self.compute_queues.retained = retained;
        submission.active = Some(Active {
            plan,
            staging,
            pending_wait: None,
        });
        Ok(())
    }

    /// A fixed scheduling quantum, not additional packet/work authority.
    /// Every predicate observation, including a false one, spends finite work.
    pub(crate) fn pump_compute_queues(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        mut kernel: Option<&mut pentax_kernel::Kernel>,
    ) -> Result<ComputePump, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        if self.compute_queues.pending == 0 {
            return Ok(ComputePump { pending: 0 });
        }
        self.compute_owner(memory)?;
        let mut idle = 0usize;
        let mut steps = 0;
        while steps < 64 {
            if self.compute_queues.pending == 0 || idle == 56 {
                break;
            }
            let index = self.compute_queues.next;
            self.compute_queues.next = (index + 1) % 56;
            let Some(mut submission) = self.compute_queues.queues[index].pop_front() else {
                idle += 1;
                continue;
            };
            steps += 1;
            let mut ranges = std::mem::take(&mut self.compute_queues.ranges);
            let result = (|| {
                self.compute_charge(1)?;
                // Permissions remain current; command contents remain sealed.
                for range in &ranges {
                    self.compute_charge(1)?;
                    memory
                        .preflight_gpu_read(range.address, range.size)
                        .map_err(|_| Failure::Memory)?;
                }
                self.activate_compute(&mut submission, index)?;
                self.advance_compute(
                    memory,
                    &mut submission,
                    &mut ranges,
                    &mut kernel,
                    index as u32 + 0x20,
                )
            })();
            self.compute_queues.ranges = ranges;
            submission.had_effects |= self.poisoned;
            if matches!(&result, Ok(Advance::Complete))
                && submission
                    .initial
                    .is_some_and(|initial| initial != self.queues[index])
            {
                submission.had_effects = true;
            }
            match result {
                Ok(Advance::Complete) => {
                    // Never publish a static plan's future tail while waiting.
                    self.remove_compute_submission(&submission)?;
                    if let Some(next) = self.compute_queues.queues[index].front_mut()
                        && next.batch == submission.batch
                    {
                        next.had_effects |= submission.had_effects;
                    }
                    idle = 0;
                    self.poisoned = false;
                }
                Ok(advance) => {
                    if matches!(advance, Advance::Step) {
                        idle = 0;
                    } else {
                        idle += 1;
                    }
                    self.compute_queues.queues[index].push_front(submission);
                    // Successful effects and the precise next cursor are retained.
                    self.poisoned = false;
                }
                Err(error) => {
                    private_compute_failure::capture_from_env(
                        memory,
                        submission.diagnostic_root,
                        submission.initial.unwrap_or(self.queues[index]),
                        &error,
                    );
                    if submission.had_effects || submission.accepted {
                        self.compute_queues.queues[index].push_front(submission);
                        self.poisoned = true;
                    } else {
                        // No accepted return or committed prefix: reject this
                        // batch, preserving independent accepted queue work.
                        self.remove_compute_submission(&submission)?;
                        while self.compute_queues.queues[index]
                            .front()
                            .is_some_and(|next| next.batch == submission.batch)
                        {
                            let next = self.compute_queues.queues[index]
                                .pop_front()
                                .ok_or(Failure::Poisoned)?;
                            self.remove_compute_submission(&next)?;
                        }
                        self.poisoned = false;
                    }
                    return Err(error);
                }
            }
        }
        Ok(ComputePump {
            pending: self.compute_queues.pending,
        })
    }
}

fn compute_capture_error(error: draw::Error) -> Failure {
    Failure::Preparation(match error {
        draw::Error::Memory(error) | draw::Error::SourceMemory { error, .. } => {
            compute::Error::Memory(error)
        }
        draw::Error::Allocation => compute::Error::Allocation,
        draw::Error::Limit(reason) => compute::Error::Limit(reason),
        draw::Error::Packet { dword, reason, .. }
        | draw::Error::SourcePacket { dword, reason, .. } => {
            compute::Error::Packet { dword, reason }
        }
        draw::Error::Unsupported(reason) => compute::Error::Unsupported(reason),
        draw::Error::MissingRegister { register, .. } => compute::Error::MissingRegister(register),
        draw::Error::Compile => compute::Error::Compile,
    })
}

fn compute_plan_error(error: Failure, owner_host: bool, owner_work: bool) -> Failure {
    match error {
        Failure::Preparation(compute::Error::Limit(
            "command host bytes" | "graphics capture host bytes",
        )) if owner_host => Failure::Limit,
        Failure::Preparation(compute::Error::Limit("command work" | "graphics capture work"))
            if owner_work =>
        {
            Failure::Limit
        }
        error => error,
    }
}

impl Driver {
    fn remove_compute_submission(&mut self, submission: &Submission) -> Result<(), Failure> {
        self.compute_queues.retained = self
            .compute_queues
            .retained
            .checked_sub(submission.retained)
            .ok_or(Failure::Poisoned)?;
        self.compute_queues.pending = self
            .compute_queues
            .pending
            .checked_sub(1)
            .ok_or(Failure::Poisoned)?;
        self.compute_queues.slots = self
            .compute_queues
            .slots
            .checked_sub(submission.slots)
            .ok_or(Failure::Poisoned)?;
        let mut at = 0;
        while at < self.compute_queues.owners.len() {
            if self.compute_queues.owners[at] == submission.generation {
                self.compute_queues.owners.swap_remove(at);
                self.compute_queues.ranges.swap_remove(at);
            } else {
                at += 1;
            }
        }
        Ok(())
    }

    pub(super) fn compute_owner_matches(&self, owner: NativeOwnerId) -> bool {
        self.compute_queues.owner.is_none_or(|old| old == owner)
    }

    pub(super) fn compute_retained_bytes(&self) -> Result<usize, Failure> {
        self.compute_queues
            .retained
            .checked_add(self.command_exclusion_bytes)
            .ok_or(Failure::Limit)
    }

    pub(crate) fn pending_compute_queues(&self) -> usize {
        self.compute_queues.pending
    }

    /// Cleanup authenticates the owner even after another engine has poisoned
    /// execution. It spends no guest work and cannot make a failed driver ready.
    pub(crate) fn revoke_compute_owner(&mut self, owner: NativeOwnerId) -> Result<(), Failure> {
        self.video.check_owner(owner)?;
        if self.compute_queues.owner.is_some_and(|old| old != owner)
            || self.graphics_owner.is_some_and(|old| old != owner)
            || self
                .graphics_execution
                .as_ref()
                .is_some_and(|execution| execution.owner != owner)
        {
            return Err(Failure::Memory);
        }
        if self.compute_queues.pending != 0 {
            self.poisoned = true;
        }
        self.compute_queues = ComputeQueues::default();
        self.command_exclusions = Vec::new();
        self.command_exclusion_bytes = 0;
        Ok(())
    }
}

impl Driver {
    fn compute_owner(&self, memory: &PausedNativeMemory<'_>) -> Result<(), Failure> {
        if self
            .compute_queues
            .owner
            .is_some_and(|old| old != memory.owner_id())
        {
            return Err(Failure::Memory);
        }
        self.graphics_check_owner(memory)?;
        memory.preflight_gpu_read(0, 0).map_err(|_| Failure::Memory)
    }

    pub(super) fn execute_compute_ranges_inner(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        queue: u32,
        roots: Vec<NativeRange>,
        kernel: Option<&mut pentax_kernel::Kernel>,
    ) -> Result<u64, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.compute_owner(memory)?;
        let index = queue
            .checked_sub(0x20)
            .filter(|n| *n < 56)
            .ok_or(Failure::UnsupportedQueue)? as usize;
        let diagnostic_root = roots
            .iter()
            .find(|r| r.address != 0 && r.size != 0)
            .map(|r| compute::CommandRange {
                address: r.address,
                size: r.size,
            });
        let batch = self
            .compute_queues
            .generation
            .checked_add(1)
            .ok_or(Failure::Limit)?;
        for root in roots.into_iter().filter(|r| r.address != 0 && r.size != 0) {
            if let Err(error) = self.admit_compute(
                memory,
                index,
                compute::CommandRange {
                    address: root.address,
                    size: root.size,
                },
                diagnostic_root.ok_or(Failure::UnsupportedQueue)?,
                batch,
            ) {
                let terminal = self.poisoned
                    || self
                        .compute_queues
                        .queues
                        .iter()
                        .flat_map(|queue| queue.iter())
                        .any(|job| job.accepted || job.had_effects);
                // Admission itself made no guest effect. An earlier accepted
                // cancellation stays terminal; a wholly unpublished attempt
                // releases all possibly grown pools and remains retryable.
                self.revoke_compute_owner(memory.owner_id())?;
                self.poisoned = terminal;
                return Err(error);
            }
        }
        self.pump_compute_queues(memory, kernel)?;
        for submission in &mut self.compute_queues.queues[index] {
            if submission.batch == batch {
                submission.accepted = true;
            }
        }
        Ok(0)
    }

    fn compute_charge(&mut self, work: u64) -> Result<(), Failure> {
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            work,
            self.limits.max_preparation_work,
        )?;
        Ok(())
    }

    fn admit_compute(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        index: usize,
        root: compute::CommandRange,
        diagnostic_root: compute::CommandRange,
        batch: u64,
    ) -> Result<(), Failure> {
        self.usage.command_buffers = add(
            self.usage.command_buffers,
            1,
            self.limits.max_command_buffers,
        )?;
        // Preserve the existing buffer-admission debit before parser refusal.
        self.usage.command_bytes = add(
            self.usage.command_bytes,
            root.size,
            self.limits.max_command_bytes,
        )?;
        let mut limits = self.limits.commands;
        limits.max_command_bytes = limits.max_command_bytes.min(
            usize::try_from(self.limits.max_command_bytes - self.usage.command_bytes + root.size)
                .unwrap_or(usize::MAX),
        );
        limits.max_work = limits.max_work.min(
            self.limits
                .max_preparation_work
                .saturating_sub(self.usage.preparation_work),
        );
        let command_ceiling = limits.max_host_bytes;
        let command_host = command_ceiling
            .checked_sub(self.gds_retained_bytes())
            .ok_or(Failure::Preparation(compute::Error::Limit(
                "command host bytes",
            )))?;
        let owner_host = self
            .graphics_host_ceiling()?
            .min(self.limits.compute.max_host_bytes)
            .checked_sub(self.graphics_held_bytes()?)
            .ok_or(Failure::Limit)?;
        let owner_host_binds = owner_host < command_host;
        let owner_work_binds = limits.max_work < self.limits.commands.max_work;
        // Preserve the original driver's root walk/copy admission before the
        // capture parser can turn a shared-work failure into a packet limit.
        if root
            .size
            .checked_mul(3)
            .is_none_or(|work| work > limits.max_work)
        {
            return Err(Failure::Limit);
        }
        limits.max_host_bytes = command_host.min(owner_host);
        let captured = draw::CapturedComputeCommands::capture_attempt(
            root,
            &super::super::Memory(memory),
            &limits,
        );
        let sources = match captured {
            Ok(sources) => sources,
            Err(failure) => {
                self.compute_charge(failure.usage.work)?;
                return Err(compute_plan_error(
                    compute_capture_error(failure.error),
                    owner_host_binds,
                    owner_work_binds,
                ));
            }
        };
        self.compute_charge(sources.usage().work)?;
        self.usage.command_bytes = add(
            self.usage.command_bytes,
            (sources.usage().command_bytes as u64)
                .checked_sub(root.size)
                .ok_or(Failure::Limit)?,
            self.limits.max_command_bytes,
        )?;
        self.usage.indirect_buffers = self
            .usage
            .indirect_buffers
            .saturating_add(sources.usage().indirect_buffers as u64);
        let generation = self
            .compute_queues
            .generation
            .checked_add(1)
            .ok_or(Failure::Limit)?;
        // Reserve reusable exclusion slots for every captured range and every
        // possible live register-array resolution before this job can execute.
        let slots = sources
            .command_ranges()
            .len()
            .checked_add(limits.max_actions)
            .ok_or(Failure::Limit)?;
        let required_slots = self
            .compute_queues
            .slots
            .checked_add(slots)
            .ok_or(Failure::Limit)?;
        let old_storage = self.compute_storage_bytes()?;
        let growth = required_slots
            .checked_mul(size_of::<compute::CommandRange>() + size_of::<u64>())
            .and_then(|n| {
                n.checked_add(
                    self.compute_queues.queues[index]
                        .len()
                        .checked_add(1)?
                        .checked_next_power_of_two()?
                        .checked_mul(size_of::<Submission>())?,
                )
            })
            .ok_or(Failure::Limit)?;
        let held = sources.usage().host_bytes;
        self.graphics_held_bytes()?
            .checked_add(held)
            .and_then(|n| n.checked_add(growth))
            .filter(|n| {
                *n <= self.graphics_host_ceiling().unwrap_or(0)
                    && *n <= self.limits.compute.max_host_bytes
            })
            .ok_or(Failure::Limit)?;
        self.compute_queues
            .ranges
            .try_reserve_exact(
                required_slots
                    .checked_sub(self.compute_queues.ranges.len())
                    .ok_or(Failure::Limit)?,
            )
            .map_err(|_| Failure::Allocation)?;
        self.compute_queues
            .owners
            .try_reserve_exact(
                required_slots
                    .checked_sub(self.compute_queues.owners.len())
                    .ok_or(Failure::Limit)?,
            )
            .map_err(|_| Failure::Allocation)?;
        self.compute_queues.queues[index]
            .try_reserve_exact(1)
            .map_err(|_| Failure::Allocation)?;
        let new_storage = self.compute_storage_bytes()?;
        let delta = new_storage.checked_sub(old_storage).ok_or(Failure::Limit)?;
        self.compute_charge(delta as u64)?;
        let retained = self
            .compute_queues
            .retained
            .checked_add(held)
            .and_then(|n| n.checked_add(delta))
            .ok_or(Failure::Limit)?;
        let other = self
            .graphics_held_bytes()?
            .checked_sub(self.compute_queues.retained)
            .ok_or(Failure::Limit)?;
        other
            .checked_add(retained)
            .filter(|n| {
                *n <= self.graphics_host_ceiling().unwrap_or(0)
                    && *n <= self.limits.compute.max_host_bytes
            })
            .ok_or(Failure::Limit)?;
        for range in sources.command_ranges() {
            self.compute_queues.ranges.push(*range);
            self.compute_queues.owners.push(generation);
        }
        // Store the total command ceiling. Recompute its net GDS allowance
        // at activation/transfer; never subtract the same retained GDS twice.
        limits.max_host_bytes = command_ceiling;
        self.compute_queues.queues[index].push_back(Submission {
            generation,
            batch,
            accepted: false,
            had_effects: false,
            slots,
            root,
            diagnostic_root,
            initial: None,
            sources,
            active: None,
            limits,
            retained: held,
        });
        self.compute_queues.retained = retained;
        self.compute_queues.owner = Some(memory.owner_id());
        self.compute_queues.generation = generation;
        self.compute_queues.slots = required_slots;
        self.compute_queues.pending += 1;
        Ok(())
    }

    fn compute_storage_bytes(&self) -> Result<usize, Failure> {
        let mut bytes = self
            .compute_queues
            .ranges
            .capacity()
            .checked_mul(size_of::<compute::CommandRange>())
            .and_then(|n| {
                n.checked_add(
                    self.compute_queues
                        .owners
                        .capacity()
                        .checked_mul(size_of::<u64>())?,
                )
            })
            .ok_or(Failure::Limit)?;
        for queue in &self.compute_queues.queues {
            bytes = bytes
                .checked_add(
                    queue
                        .capacity()
                        .checked_mul(size_of::<Submission>())
                        .ok_or(Failure::Limit)?,
                )
                .ok_or(Failure::Limit)?;
        }
        Ok(bytes)
    }
}

impl Driver {
    fn compute_dispatch(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        dispatch: &compute::Dispatch,
        commands: &[compute::CommandRange],
    ) -> Result<Advance, Failure> {
        let remaining_invocations = self
            .limits
            .max_invocations
            .saturating_sub(self.usage.invocations);
        let mut limits = self.limits.compute;
        limits.max_host_bytes = limits
            .max_host_bytes
            .min(self.graphics_host_ceiling()?)
            .checked_sub(self.graphics_held_bytes()?)
            .ok_or(Failure::Limit)?;
        if self.shaders.lookup_work() > 1
            && let Ok(address) = dispatch.shader_address()
        {
            self.usage.preparation_work = add(
                self.usage.preparation_work,
                self.shaders.lookup_work(),
                self.limits.max_preparation_work,
            )?;
            if let Some(mapped) = self.shaders.mapping(memory.owner_id(), address) {
                if mapped.kind != 0 {
                    return Err(Failure::Shader(crate::gpu::shaders::ShaderFailure::Stage));
                }
                // Zero size follows the source's unspecified-size metadata
                // behavior. A nonzero declared extent bounds real decoding.
                if mapped.code_bytes != 0 {
                    limits.max_shader_words =
                        limits.max_shader_words.min(mapped.code_bytes as usize / 4);
                }
            }
        }
        limits.max_invocations = limits.max_invocations.min(remaining_invocations);
        limits.max_work = limits.max_work.min(
            self.limits
                .max_preparation_work
                .saturating_sub(self.usage.preparation_work),
        );
        self.usage.dispatches = add(self.usage.dispatches, 1, self.limits.max_dispatches)?;
        let report = execute_dispatch_admitted(
            memory,
            dispatch,
            &limits,
            &self.limits.gpu,
            commands,
            |prepared, preflight, may_execute| {
                let invocations = add(
                    self.usage.invocations,
                    prepared.invocations,
                    self.limits.max_invocations,
                )
                .map_err(|_| Error::Limit)?;
                let work = prepared.work.checked_add(preflight).ok_or(Error::Limit)?;
                let work = add(
                    self.usage.preparation_work,
                    work,
                    self.limits.max_preparation_work,
                )
                .map_err(|_| Error::Limit)?;
                self.usage.invocations = invocations;
                self.usage.preparation_work = work;
                // Once GPU work could have happened, a later failure is
                // terminal. Never replay a partially completed stream with
                // an earlier register snapshot.
                self.poisoned |= may_execute;
                Ok(())
            },
        )
        .map_err(|e| match e {
            Error::Prepare(e) => Failure::Preparation(e),
            Error::Memory(_) => Failure::Memory,
            Error::Vulkan(e) => Failure::Vulkan(e),
            Error::Allocation => Failure::Allocation,
            Error::Limit => Failure::Limit,
        })?;
        // max_dispatches * max_buffer_bytes can overflow for caller limits;
        // accounting is saturated, never a post-effect fallible operation.
        self.usage.written_bytes = self
            .usage
            .written_bytes
            .saturating_add(report.written_bytes as u64);
        Ok(Advance::Step)
    }
}
