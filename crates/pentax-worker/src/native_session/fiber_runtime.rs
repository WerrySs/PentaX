// SPDX-License-Identifier: GPL-2.0-only
//! Bounded fiber continuation banks on the existing guest executor. The active
//! pthread keeps its own root/TLS; idle fibers retain independently movable
//! CPU and callback metadata. No host fiber or guest-selected host pointer.
use super::*;
use pentax_core::process::hle::{FiberEntryLayout, PreparedFiberAction};
use pentax_kernel::{FiberKey, FiberTransferKind};
use pentax_platform::native_dispatch::NativeFiberContext;

type Frames = Vec<(SuspendedInitializer, NativeNestedCall, NativeSessionPhase)>;

struct Fiber {
    key: FiberKey,
    context: Option<NativeFiberContext>,
    frames: Frames,
    phase: NativeSessionPhase,
}
struct Root {
    thread: usize,
    context: Option<NativeFiberContext>,
    frames: Frames,
    phase: NativeSessionPhase,
}
pub(super) struct Fibers {
    fibers: Vec<Fiber>,
    roots: Vec<Root>,
    maximum_fibers: usize,
    maximum_roots: usize,
}

fn frames(work: &mut u64, maximum_work: u64) -> Result<Frames, NativeSessionError> {
    let bytes = 32_usize
        .checked_mul(std::mem::size_of::<(
            SuspendedInitializer,
            NativeNestedCall,
            NativeSessionPhase,
        )>())
        .and_then(|bytes| u64::try_from(bytes).ok())
        .ok_or(NativeSessionError::Budget("fiber callback storage"))?;
    charge(work, bytes, maximum_work, "fiber callback storage")?;
    let mut frames = Vec::new();
    frames
        .try_reserve_exact(32)
        .map_err(|_| NativeSessionError::Allocation)?;
    if frames.capacity() > 32 {
        return Err(NativeSessionError::Budget("fiber callback capacity"));
    }
    Ok(frames)
}

fn reserve<T>(
    rows: &mut Vec<T>,
    maximum: usize,
    work: &mut u64,
    maximum_work: u64,
) -> Result<(), NativeSessionError> {
    if rows.len() >= maximum {
        return Err(NativeSessionError::Budget("fiber contexts"));
    }
    if rows.len() == rows.capacity() {
        let moved = rows
            .len()
            .checked_mul(std::mem::size_of::<T>())
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or(NativeSessionError::Budget("fiber context moves"))?;
        charge(work, moved, maximum_work, "fiber context moves")?;
        rows.try_reserve_exact(1)
            .map_err(|_| NativeSessionError::Allocation)?;
    }
    if rows.capacity() > maximum {
        return Err(NativeSessionError::Budget("fiber context capacity"));
    }
    Ok(())
}

impl Fibers {
    pub(super) fn new(maximum_fibers: usize, maximum_roots: usize) -> Self {
        Self {
            fibers: Vec::new(),
            roots: Vec::new(),
            maximum_fibers,
            maximum_roots,
        }
    }

    fn root(
        &mut self,
        thread: usize,
        phase: NativeSessionPhase,
        work: &mut u64,
        maximum_work: u64,
    ) -> Result<usize, NativeSessionError> {
        if thread >= self.maximum_roots {
            return Err(NativeSessionError::Budget("fiber root thread"));
        }
        if let Some(index) = self.roots.iter().position(|row| row.thread == thread) {
            return Ok(index);
        }
        let frames = frames(work, maximum_work)?;
        reserve(&mut self.roots, self.maximum_roots, work, maximum_work)?;
        let index = self.roots.len();
        self.roots.push(Root {
            thread,
            context: None,
            frames,
            phase,
        });
        Ok(index)
    }

    #[allow(clippy::too_many_arguments)]
    fn fresh(
        &mut self,
        key: FiberKey,
        layout: FiberEntryLayout,
        session: &mut PreparedNativeKernelStartup<'_, '_>,
        engine: &mut NativeBlockStepper<'_, '_, '_>,
        phase: NativeSessionPhase,
        work: &mut u64,
        maximum_work: u64,
    ) -> Result<usize, NativeSessionError> {
        if self.fibers.iter().any(|row| row.key == key) {
            return Err(NativeSessionError::FixtureInvariant(
                "repeated fresh fiber origin",
            ));
        }
        let old = self
            .fibers
            .iter()
            .position(|row| row.key.address() == key.address());
        if old.is_some_and(|index| self.fibers[index].context.is_none()) {
            return Err(NativeSessionError::FixtureInvariant(
                "replace active fiber generation",
            ));
        }
        let frames = frames(work, maximum_work)?;
        if old.is_none() {
            reserve(&mut self.fibers, self.maximum_fibers, work, maximum_work)?;
        }
        if let Some(index) = old {
            // The kernel admitted a new generation, so this parked origin is
            // invalid. Release its bounded platform slot before replacement.
            self.fibers[index].context = None;
        }
        // Preserve actual current floating/vector/control values. First entry
        // changes only the declared entry, stack and two source arguments.
        // This does not invent an ABI guarantee for caller-saved registers.
        charge(
            work,
            std::mem::size_of::<NativeCpuState>() as u64,
            maximum_work,
            "fiber initial CPU",
        )?;
        let mut cpu = *engine.state();
        cpu.guest_pc = layout.entry;
        cpu.gpr[4] = layout.frame.return_slot;
        cpu.gpr[7] = layout.args[0];
        cpu.gpr[6] = layout.args[1];
        let context = engine.prepare_fiber(&mut session.pause()?, cpu, layout.frame)?;
        let row = Fiber {
            key,
            context: Some(context),
            frames,
            phase,
        };
        if let Some(index) = old {
            self.fibers[index] = row;
            Ok(index)
        } else {
            let index = self.fibers.len();
            self.fibers.push(row);
            Ok(index)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn transfer(
        &mut self,
        session: &mut PreparedNativeKernelStartup<'_, '_>,
        proof: &PreparedNativeSource<'_, '_>,
        engine: &mut NativeBlockStepper<'_, '_, '_>,
        pending: PreparedFiberAction,
        current_frames: &mut Frames,
        current_phase: &mut NativeSessionPhase,
        work: &mut u64,
        maximum_work: u64,
    ) -> Result<(), NativeSessionError> {
        let visits = self
            .fibers
            .len()
            .checked_mul(4)
            .and_then(|count| count.checked_add(self.roots.len()))
            .and_then(|count| count.checked_add(32))
            .and_then(|count| u64::try_from(count).ok())
            .ok_or(NativeSessionError::Budget("fiber continuation visits"))?;
        charge(work, visits, maximum_work, "fiber continuation visits")?;
        let from = pending.from();
        let target = pending.target();
        let kind = pending.kind();
        let layout = pending.target_layout();
        if !matches!(
            (kind, from, target),
            (FiberTransferKind::Run, None, Some(_))
                | (FiberTransferKind::Switch, Some(_), Some(_))
                | (FiberTransferKind::Return, Some(_), None)
        ) {
            return Err(NativeSessionError::FixtureInvariant("fiber transfer shape"));
        }
        let root = self.root(engine.thread_index(), *current_phase, work, maximum_work)?;
        let from_index = from
            .map(|key| {
                self.fibers.iter().position(|row| row.key == key).ok_or(
                    NativeSessionError::FixtureInvariant("unknown departing fiber"),
                )
            })
            .transpose()?;
        if let Some(index) = from_index {
            if self.fibers[index].context.is_some() {
                return Err(NativeSessionError::FixtureInvariant(
                    "departing fiber is parked",
                ));
            }
        } else if self.roots[root].context.is_some() {
            return Err(NativeSessionError::FixtureInvariant(
                "repeated running fiber root",
            ));
        }
        let target_index = if let Some(key) = target {
            Some(if let Some(layout) = layout {
                self.fresh(
                    key,
                    layout,
                    session,
                    engine,
                    *current_phase,
                    work,
                    maximum_work,
                )?
            } else {
                self.fibers
                    .iter()
                    .position(|row| row.key == key)
                    .ok_or(NativeSessionError::FixtureInvariant("unknown saved fiber"))?
            })
        } else {
            None
        };
        let parked = if let Some(index) = target_index {
            self.fibers[index].context.take()
        } else {
            self.roots[root].context.take()
        };
        let mut parked = parked.ok_or(NativeSessionError::FixtureInvariant(
            "missing fiber destination",
        ))?;
        let resumed = parked.pending_syscall();
        if resumed.is_none() != (target.is_some() && layout.is_some()) {
            return Err(NativeSessionError::FixtureInvariant(
                "fiber destination phase",
            ));
        }
        session.observe_fiber_target(proof, &pending, &parked)?;
        // Every allocation and destination admission precedes the exchange.
        // A failure below terminates the worker; no later guest instruction is
        // dispatched and no partially exchanged bank can be replayed.
        engine.exchange_fiber(&mut session.pause()?, &mut parked)?;
        let value = session.commit_fiber_action(proof, pending, &parked)?;

        if let Some(index) = from_index {
            self.fibers[index].context = Some(parked);
            std::mem::swap(current_frames, &mut self.fibers[index].frames);
            self.fibers[index].phase = *current_phase;
        } else {
            self.roots[root].context = Some(parked);
            std::mem::swap(current_frames, &mut self.roots[root].frames);
            self.roots[root].phase = *current_phase;
        }
        if let Some(index) = target_index {
            std::mem::swap(current_frames, &mut self.fibers[index].frames);
            *current_phase = self.fibers[index].phase;
        } else {
            std::mem::swap(current_frames, &mut self.roots[root].frames);
            *current_phase = self.roots[root].phase;
        }
        if let Some(token) = resumed {
            engine.complete_syscall(token, value)?;
        }
        Ok(())
    }
}
