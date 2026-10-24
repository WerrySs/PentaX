// SPDX-License-Identifier: GPL-2.0-only
// Original bounded logical-wave geometry. Public KytyPS5 7b7f9a7
// Translate.cpp initializes EXEC from requested XYZ thread extents; unlike a
// native subgroup ballot, this construction also covers Wave64 on native32.

use crate::ir::{Builtin, Emitter, Op, Value};
use crate::{CompileError, profile::ComputeDescription};

fn valid_lane(
    e: &mut Emitter<'_>,
    linear: Value,
    local: [u32; 3],
    group_base: [Value; 3],
    extent: [Value; 3],
) -> Result<Value, CompileError> {
    let xy = local[0]
        .checked_mul(local[1])
        .ok_or(CompileError::Internal("thread extent local product"))?;
    let total = xy
        .checked_mul(local[2])
        .ok_or(CompileError::Internal("thread extent local product"))?;
    let qx = e.emit(Op::UDiv32, &[linear, Value::U32(local[0])])?;
    let x_product = e.imul(qx, Value::U32(local[0]))?;
    let x = e.isub(linear, x_product)?;
    let qy = e.emit(Op::UDiv32, &[qx, Value::U32(local[1])])?;
    let y_product = e.imul(qy, Value::U32(local[1]))?;
    let y = e.isub(qx, y_product)?;
    let z = e.emit(Op::UDiv32, &[linear, Value::U32(xy)])?;
    let mut active = e.ult(linear, Value::U32(total))?;
    for axis in 0..3 {
        let coordinate = [x, y, z][axis];
        let global = e.iadd(group_base[axis], coordinate)?;
        let inside = e.ult(global, extent[axis])?;
        active = e.land(active, inside)?;
    }
    Ok(active)
}

/// Every actual invocation in a logical wave derives the same two EXEC words
/// from uniform group/extents and its logical wave base. No lane reads another
/// lane, no subgroup feature is requested and no workgroup barrier is inserted.
/// The host admits padded XYZ products without u32 GlobalInvocationId wrap.
pub(super) fn initial_exec(
    e: &mut Emitter<'_>,
    cs: &ComputeDescription,
) -> Result<(Value, [Value; 2]), CompileError> {
    let wave = cs.common.wave_size.lanes();
    // Charge the complete fixed walk before generating it. Each IR arena
    // growth and eventual SPIR-V word also retains its ordinary independent cap.
    e.budget.charge(u64::from(wave) + 1)?;
    let mut extent = [Value::U32(0); 3];
    let mut group_base = [Value::U32(0); 3];
    for axis in 0..3 {
        extent[axis] = e.get(Op::GetDispatchThreadExtent, axis as u32)?;
        let group = e.get(Op::GetBuiltin, Builtin::WorkgroupId.aux(axis as u32))?;
        group_base[axis] = e.imul(group, Value::U32(cs.workgroup_size[axis]))?;
    }
    let local = e.get(Op::GetBuiltin, Builtin::LocalInvocationIndex.aux(0))?;
    let wave_base = e.and(local, Value::U32(!(wave - 1)))?;
    let active = valid_lane(e, local, cs.workgroup_size, group_base, extent)?;
    let mut mask = [Value::U32(0); 2];
    for lane in 0..wave {
        let linear = e.iadd(wave_base, Value::U32(lane))?;
        let valid = valid_lane(e, linear, cs.workgroup_size, group_base, extent)?;
        let bit = e.select(valid, Value::U32(1u32 << (lane & 31)), Value::U32(0))?;
        let half = (lane / 32) as usize;
        mask[half] = e.or(mask[half], bit)?;
    }
    Ok((active, mask))
}
