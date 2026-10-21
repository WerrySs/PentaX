// SPDX-License-Identifier: GPL-2.0-only
//! Scoped access to the guarded native record. No reference escapes a helper.
use super::{CONTEXT, DispatchGuard, NativeCpuState, VectorMode};

/// Refresh every field read by the selected bridge before hardware entry.
/// Entry writes host_rsp before using it; padding and the inactive vector bank
/// are not bridge inputs. The guard still clears the complete record on drop.
#[inline]
pub(super) fn stage(
    _guard: &DispatchGuard,
    mode: VectorMode,
    state: &NativeCpuState,
    guest_base: u64,
    guest_limit: u64,
    entry: u64,
) {
    // SAFETY: The borrowed process-wide guard owns this initialized static.
    // This private helper never enters assembly or returns a context reference.
    let context = unsafe { &mut *CONTEXT.0.get() };
    context.gpr = state.gpr;
    context.rflags = state.rflags;
    context.mxcsr = state.mxcsr;
    mode.load(context, &state.xmm, &state.ymm_upper);
    context.guest_base = guest_base;
    context.guest_limit = guest_limit;
    context.rip = entry;
}

/// Capture only guest outputs written by the selected trailer. SSE2 keeps the
/// caller's upper YMM state; ordinary execution never reads the private RIP.
#[inline]
pub(super) fn capture(_guard: &DispatchGuard, mode: VectorMode, state: &mut NativeCpuState) {
    // SAFETY: The borrowed guard excludes every other context user and the
    // bridge has returned. This reference ends before another native entry.
    let context = unsafe { &*CONTEXT.0.get() };
    state.gpr = context.gpr;
    state.rflags = context.rflags;
    state.mxcsr = context.mxcsr;
    mode.save(context, &mut state.xmm, &mut state.ymm_upper);
}

/// Read the private selector only after a separately attested trace returns.
/// The caller retains terminal phase until the captured value is validated.
#[inline]
pub(super) fn affine_iterations(_guard: &DispatchGuard) -> u64 {
    // SAFETY: Same exclusive guard and completed bridge; this copies one
    // primitive field and creates no reference surviving a later native call.
    unsafe { (*CONTEXT.0.get()).rip }
}
