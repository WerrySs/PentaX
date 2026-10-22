// SPDX-License-Identifier: GPL-2.0-only
//! Optional two-iteration admission. A failed speculation never executes a body.
use super::*;
use pentax_codegen::RegisterLoopTrace;

fn add(left: u64, right: u64) -> Result<u64, NativeDispatchError> {
    left.checked_add(right)
        .ok_or(NativeDispatchError::Budget("register trace accounting"))
}
fn mul(left: u64, right: u64) -> Result<u64, NativeDispatchError> {
    left.checked_mul(right)
        .ok_or(NativeDispatchError::Budget("register trace accounting"))
}
impl<'view, 'plan, 'image> NativeBlockStepper<'view, 'plan, 'image> {
    /// Attempt at most two iterations of a separately constructed checked trace.
    /// `None` means use the unchanged ordinary block before any guest effects.
    /// Performed speculation work and access observations are never refunded.
    /// The caller must supply at most the logical span remaining before every
    /// scheduling, sampling, progress, frame-pump and external lifetime boundary.
    /// A span below two performs no trace work or cache mutation.
    ///
    /// # Safety
    /// Bind the exact source/owner/TLS proof and exclusive paused backing as for
    /// `step_checked`. No other thread may access guest backing during this call.
    /// A returned error after hardware entry is terminal and must never replay.
    pub unsafe fn try_step_register_loop_checked(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        trace: &RegisterLoopTrace<'_, 'view, 'plan, 'image>,
        max_logical_span: u64,
    ) -> Result<Option<NativeBlockStop>, NativeDispatchError> {
        if max_logical_span < 2 {
            return Ok(None);
        }
        let block = trace.block();
        if self.machine.phase != Phase::Ready {
            return Err(NativeDispatchError::WrongPhase);
        }
        if memory.owner_id() != self.machine.owner {
            return Err(NativeDispatchError::WrongOwner);
        }
        if !std::ptr::eq(block.declaration(), self.source)
            || !std::ptr::eq(block.source_plan(), self.source.source_plan())
            || block.guest_pc() != self.machine.state.guest_pc
        {
            return Err(NativeDispatchError::WrongSource);
        }
        validate_state(&self.machine.state)?;
        let (condition, target, fallthrough) = match block.exit().kind() {
            ExitKind::Conditional {
                condition,
                target,
                fallthrough,
            } if target == block.guest_pc() => (condition, target, fallthrough),
            _ => return Ok(None),
        };
        if block.emits_non_temporal_store() || block.native_admission().has_atomic_memory() {
            return Ok(None);
        }
        let count = block.native_admission().emitted_instructions();
        if count == 0 {
            return Ok(None);
        }
        let records = block.instructions().len() as u64;
        let numeric = (|| -> Result<_, NativeDispatchError> {
            let emitted = add(trace.bytes().len() as u64, 16)?;
            let normal_emitted = add(block.bytes().len() as u64, 12)?;
            let context_work = if self.machine.vector_mode.features().is_some() {
                1024
            } else {
                0
            };
            let stack_work = add(
                add(self.machine.stack.stack.size.div_ceil(NATIVE_GUEST_PAGE), 1)?,
                self.machine.stack.guards.map_or(0, |[lower, upper]| {
                    lower.size / NATIVE_GUEST_PAGE + upper.size / NATIVE_GUEST_PAGE
                }),
            )?;
            let common = add(
                add(
                    block.native_admission().dispatch_work_units(),
                    mul(self.machine.target_visits, 3)?,
                )?,
                add(
                    mul(block.usage().source_bytes, 3)?,
                    add(block.native_admission().feature_visits(), 16)?,
                )?,
            )?;
            let metadata = add(
                add(common, mul(records, 2)?)?,
                add(mul(emitted, 6)?, context_work)?,
            )?;
            let trace_upper = add(
                add(metadata, stack_work)?,
                PUBLICATION_HISTORY_WORK + RESIDENT_HINT_WORK,
            )?;
            let fallback_upper = add(
                add(common, stack_work)?,
                add(
                    mul(records, 2 + 128 + 144)?,
                    add(
                        mul(normal_emitted, 6)?,
                        context_work + PUBLICATION_HISTORY_WORK + RESIDENT_HINT_WORK,
                    )?,
                )?,
            )?;
            Ok((
                emitted,
                normal_emitted,
                metadata,
                add(trace_upper, fallback_upper)?,
                mul(count, 2)?,
            ))
        })();
        let Ok((emitted, normal_emitted, metadata, reserve, maximum_instructions)) = numeric else {
            return Ok(None);
        };
        // Reserve both the attempted trace and its unchanged ordinary fallback
        // before performing any speculative scan. No lifetime limit is raised.
        if add(self.machine.usage.work_units, reserve)
            .ok()
            .is_none_or(|value| value > self.machine.limits.max_work_units)
            || self
                .machine
                .usage
                .blocks
                .checked_add(2)
                .is_none_or(|value| value > self.machine.limits.max_blocks)
            || self
                .machine
                .usage
                .native_instructions
                .checked_add(maximum_instructions)
                .is_none()
            || self.machine.usage.native_entries.checked_add(1).is_none()
            || self
                .machine
                .usage
                .register_loop_entries
                .checked_add(1)
                .is_none()
            || self
                .machine
                .usage
                .register_loop_iterations
                .checked_add(2)
                .is_none()
            || self
                .machine
                .usage
                .source_bytes
                .checked_add(block.usage().source_bytes)
                .is_none_or(|value| value > self.machine.limits.max_source_bytes)
            || self
                .machine
                .usage
                .emitted_bytes
                .checked_add(emitted.max(normal_emitted))
                .is_none_or(|value| value > self.machine.limits.max_emitted_bytes)
            || self
                .machine
                .usage
                .cache_publications
                .checked_add(1)
                .is_none()
            || self.machine.usage.cache_reuses.checked_add(1).is_none()
        {
            return Ok(None);
        }
        let Ok(guard) = DispatchGuard::acquire() else {
            return Ok(None);
        };
        charge(
            &mut self.machine.usage.work_units,
            metadata,
            self.machine.limits.max_work_units,
            "register trace metadata",
        )?;
        // Every speculative liveness, source, feature, stack and target
        // refusal leaves CPU/data/cache effects to the ordinary path instead.
        if self
            .preflight_register_trace(memory, block, target, fallthrough)
            .is_err()
        {
            return Ok(None);
        }
        let Some(layout) = self.register_layout(trace) else {
            return Ok(None);
        };
        self.publish_register_trace(trace, &layout)?;
        self.machine.usage.source_bytes += block.usage().source_bytes;
        self.machine.usage.emitted_bytes += emitted;
        let guest_base = end(self.machine.stack.stack)?;
        native_context::stage(
            &guard,
            self.machine.vector_mode,
            &self.machine.state,
            guest_base,
            self.machine.stack.stack.address,
            trace.placement().address,
        );
        self.machine.phase = Phase::Terminal;
        // SAFETY: The exact opaque register-only trace, immutable source,
        // current rights and two mode-bound cells were
        // checked under this same pause and guard. No Rust reference crosses asm.
        unsafe {
            self.machine.vector_mode.enter();
        }
        self.machine.usage.native_entries += 1;
        self.machine.usage.register_loop_entries += 1;
        native_context::capture(&guard, self.machine.vector_mode, &mut self.machine.state);
        let iterations = native_context::affine_iterations(&guard);
        if !(1..=2).contains(&iterations) {
            return Err(NativeDispatchError::EmissionMismatch);
        }
        self.machine.usage.blocks += iterations;
        self.machine.usage.register_loop_iterations += iterations;
        self.machine.usage.native_instructions += count * iterations;
        let key = PublicationKey {
            source: block.source_plan().identity(),
            guest_pc: block.guest_pc(),
            vector_mode: self.machine.vector_mode,
        };
        self.machine.published_slots[layout.slot].touch(key, self.machine.usage.blocks);
        validate_state(&self.machine.state)?;
        check_rsp(&self.machine.stack, self.machine.state.gpr[RSP])?;
        let taken = condition_holds(condition, self.machine.state.rflags)
            .ok_or(NativeDispatchError::InvalidState)?;
        if iterations == 1 && taken {
            return Err(NativeDispatchError::EmissionMismatch);
        }
        let next = if taken { target } else { fallthrough };
        self.machine.state.guest_pc = next;
        self.machine.phase = Phase::Ready;
        Ok(Some(NativeBlockStop::Resume { guest_pc: next }))
    }

    fn preflight_register_trace(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, 'plan, 'image>,
        target: u64,
        fallthrough: u64,
    ) -> Result<(), NativeDispatchError> {
        validate_stack(
            memory,
            &self.machine.stack,
            self.machine.state.gpr[RSP],
            &mut self.machine.usage,
            &self.machine.limits,
        )?;
        let policy = block.policy();
        let vector_request = policy.requests_native_packed_float_memory()
            || policy.requests_native_streaming_stores()
            || policy.requests_native_vector_moves()
            || policy.requests_native_vector_broadcasts()
            || policy.requests_native_scalar_float()
            || policy.requests_native_float_min_max()
            || policy.requests_native_scalar_fma()
            || policy.requests_native_fma_registers();
        if vector_request && !policy.requests_native_ymm_registers()
            || (vector_request || policy.requests_native_ymm_registers())
                && self.machine.vector_mode.features().is_none()
            || (policy.requests_native_scalar_fma() || policy.requests_native_fma_registers())
                && !self.machine.vector_mode.supports(CpuidFeature::FMA)
            || !self
                .machine
                .vector_mode
                .supports_admission(block.native_admission())
        {
            return Err(NativeDispatchError::InvalidState);
        }
        attest_live_source(memory, block)?;
        if !block.attest_emission(block.placement().address, block.bytes())
            || !block.attest_exit(
                block.placement().address + block.bytes().len() as u64 - 2,
                &block.bytes()[block.bytes().len() - 2..],
            )
        {
            return Err(NativeDispatchError::EmissionMismatch);
        }
        self.check_target(memory, target)?;
        self.check_target(memory, fallthrough)?;
        if !block.native_admission().is_memory_free() {
            return Err(NativeDispatchError::MemoryIsolationRequired);
        }
        Ok(())
    }
}

struct Layout {
    base: usize,
    slot: usize,
    page: usize,
    page_bytes: usize,
    exits: [usize; 2],
    cells: [usize; 2],
    tails: [[u8; 6]; 2],
    targets: [u64; 2],
}
impl NativeBlockStepper<'_, '_, '_> {
    fn register_layout(&self, trace: &RegisterLoopTrace<'_, '_, '_, '_>) -> Option<Layout> {
        let stride = self.machine.cache.len / self.machine.limits.cache_slots;
        let base = usize::try_from(
            trace
                .placement()
                .address
                .checked_sub(self.machine.cache.address())?,
        )
        .ok()?;
        if !base.is_multiple_of(stride)
            || base / stride >= self.machine.limits.cache_slots
            || trace.placement() != self.slot_placement(base / stride)
        {
            return None;
        }
        let cap = usize::try_from(trace.placement().capacity).ok()?;
        let cells = [cap.checked_sub(16)?, cap.checked_sub(8)?];
        let exits = [trace.first_exit_offset(), trace.second_exit_offset()];
        if trace.bytes().len() > cells[0]
            || exits[1].checked_add(6)? > exits[0]
            || exits[0].checked_add(6)? != trace.bytes().len()
        {
            return None;
        }
        let mut tails = [[0xff, 0x25, 0, 0, 0, 0]; 2];
        for index in 0..2 {
            if trace.bytes().get(exits[index]..exits[index] + 6)?
                != [0x0f, 0x0b, 0x90, 0x90, 0x90, 0x90]
            {
                return None;
            }
            let relative = i32::try_from(cells[index] as i64 - exits[index] as i64 - 6).ok()?;
            tails[index][2..].copy_from_slice(&relative.to_le_bytes());
        }
        let mask = NATIVE_GUEST_PAGE as usize - 1;
        let page = base & !mask;
        let page_end = base.checked_add(stride)?.checked_add(mask)? & !mask;
        Some(Layout {
            base,
            slot: base / stride,
            page,
            page_bytes: page_end - page,
            exits,
            cells,
            tails,
            targets: self.machine.vector_mode.affine_trailers(),
        })
    }
    fn register_matches(
        &self,
        trace: &RegisterLoopTrace<'_, '_, '_, '_>,
        layout: &Layout,
    ) -> Result<bool, NativeDispatchError> {
        let cache = &self.machine.cache;
        let second = layout.exits[1];
        let first = layout.exits[0];
        Ok(cache.matches(layout.base, &trace.bytes()[..second])?
            && cache.matches(layout.base + second, &layout.tails[1])?
            && cache.matches(layout.base + second + 6, &trace.bytes()[second + 6..first])?
            && cache.matches(layout.base + first, &layout.tails[0])?
            && cache.matches(
                layout.base + layout.cells[0],
                &layout.targets[0].to_le_bytes(),
            )?
            && cache.matches(
                layout.base + layout.cells[1],
                &layout.targets[1].to_le_bytes(),
            )?)
    }
    fn publish_register_trace(
        &mut self,
        trace: &RegisterLoopTrace<'_, '_, '_, '_>,
        layout: &Layout,
    ) -> Result<(), NativeDispatchError> {
        let key = PublicationKey {
            source: trace.block().source_plan().identity(),
            guest_pc: trace.block().guest_pc(),
            vector_mode: self.machine.vector_mode,
        };
        if self
            .machine
            .cache
            .range_executable(layout.page, layout.page_bytes)?
            && self.machine.published_slots[layout.slot].length == Some(trace.bytes().len())
            && self.register_matches(trace, layout)?
        {
            charge(
                &mut self.machine.usage.work_units,
                RESIDENT_HINT_WORK,
                self.machine.limits.max_work_units,
                "cache resident hint",
            )?;
            self.machine.usage.cache_reuses += 1;
        } else {
            charge(
                &mut self.machine.usage.work_units,
                PUBLICATION_HISTORY_WORK + RESIDENT_HINT_WORK,
                self.machine.limits.max_work_units,
                "cache publication history",
            )?;
            self.machine.published_slots[layout.slot].length = None;
            self.machine
                .cache
                .arm_writable_range(layout.page, layout.page_bytes)?;
            self.machine.cache.write(layout.base, trace.bytes())?;
            for index in 0..2 {
                self.machine
                    .cache
                    .write(layout.base + layout.exits[index], &layout.tails[index])?;
                self.machine.cache.write(
                    layout.base + layout.cells[index],
                    &layout.targets[index].to_le_bytes(),
                )?;
            }
            if !self.register_matches(trace, layout)? {
                return Err(NativeDispatchError::EmissionMismatch);
            }
            self.machine
                .cache
                .seal_executable_range(layout.page, layout.page_bytes)?;
            self.machine.published_slots[layout.slot].commit(
                key,
                trace.bytes().len(),
                &mut self.machine.usage,
            );
            self.machine.usage.cache_publications += 1;
        }
        self.machine.published_slots[layout.slot].touch(key, self.machine.usage.blocks);
        Ok(())
    }
}
