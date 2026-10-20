// SPDX-License-Identifier: GPL-2.0-only
//! Exact VEX FMA3 forms; arithmetic uses fixed integer storage and one rounding.
use super::NativeDispatchError;
use pentax_codegen::Code;
#[path = "fused_float/arithmetic.rs"]
mod arithmetic;
pub(super) use arithmetic::calculate;

#[derive(Clone, Copy, Debug)]
pub(super) enum Operation {
    Add,
    Sub,
    NegativeAdd,
    NegativeSub,
    AddSub,
    SubAdd,
}
impl Operation {
    pub fn signs(self, lane: usize) -> (bool, bool) {
        match self {
            Self::Add => (false, false),
            Self::Sub => (false, true),
            Self::NegativeAdd => (true, false),
            Self::NegativeSub => (true, true),
            Self::AddSub => (false, lane.is_multiple_of(2)),
            Self::SubAdd => (false, !lane.is_multiple_of(2)),
        }
    }
}
pub(super) struct Form {
    pub operation: Operation,
    pub order: u16,
    pub vector_bytes: usize,
    pub element_bytes: usize,
    pub scalar: bool,
}
impl Form {
    pub fn operands(&self, a: u64, b: u64, c: u64) -> [u64; 3] {
        match self.order {
            132 => [a, c, b],
            213 => [b, a, c],
            231 => [b, c, a],
            _ => unreachable!(),
        }
    }
}
pub(super) fn form(code: Code) -> Option<Form> {
    use Code::*;
    let (operation, order, vector_bytes, element_bytes, scalar) = match code {
        VEX_Vfmadd132ss_xmm_xmm_xmmm32 => (Operation::Add, 132, 16, 4, true),
        VEX_Vfmadd132sd_xmm_xmm_xmmm64 => (Operation::Add, 132, 16, 8, true),
        VEX_Vfmadd132ps_xmm_xmm_xmmm128 => (Operation::Add, 132, 16, 4, false),
        VEX_Vfmadd132ps_ymm_ymm_ymmm256 => (Operation::Add, 132, 32, 4, false),
        VEX_Vfmadd132pd_xmm_xmm_xmmm128 => (Operation::Add, 132, 16, 8, false),
        VEX_Vfmadd132pd_ymm_ymm_ymmm256 => (Operation::Add, 132, 32, 8, false),
        VEX_Vfmadd213ss_xmm_xmm_xmmm32 => (Operation::Add, 213, 16, 4, true),
        VEX_Vfmadd213sd_xmm_xmm_xmmm64 => (Operation::Add, 213, 16, 8, true),
        VEX_Vfmadd213ps_xmm_xmm_xmmm128 => (Operation::Add, 213, 16, 4, false),
        VEX_Vfmadd213ps_ymm_ymm_ymmm256 => (Operation::Add, 213, 32, 4, false),
        VEX_Vfmadd213pd_xmm_xmm_xmmm128 => (Operation::Add, 213, 16, 8, false),
        VEX_Vfmadd213pd_ymm_ymm_ymmm256 => (Operation::Add, 213, 32, 8, false),
        VEX_Vfmadd231ss_xmm_xmm_xmmm32 => (Operation::Add, 231, 16, 4, true),
        VEX_Vfmadd231sd_xmm_xmm_xmmm64 => (Operation::Add, 231, 16, 8, true),
        VEX_Vfmadd231ps_xmm_xmm_xmmm128 => (Operation::Add, 231, 16, 4, false),
        VEX_Vfmadd231ps_ymm_ymm_ymmm256 => (Operation::Add, 231, 32, 4, false),
        VEX_Vfmadd231pd_xmm_xmm_xmmm128 => (Operation::Add, 231, 16, 8, false),
        VEX_Vfmadd231pd_ymm_ymm_ymmm256 => (Operation::Add, 231, 32, 8, false),
        VEX_Vfmsub132ss_xmm_xmm_xmmm32 => (Operation::Sub, 132, 16, 4, true),
        VEX_Vfmsub132sd_xmm_xmm_xmmm64 => (Operation::Sub, 132, 16, 8, true),
        VEX_Vfmsub132ps_xmm_xmm_xmmm128 => (Operation::Sub, 132, 16, 4, false),
        VEX_Vfmsub132ps_ymm_ymm_ymmm256 => (Operation::Sub, 132, 32, 4, false),
        VEX_Vfmsub132pd_xmm_xmm_xmmm128 => (Operation::Sub, 132, 16, 8, false),
        VEX_Vfmsub132pd_ymm_ymm_ymmm256 => (Operation::Sub, 132, 32, 8, false),
        VEX_Vfmsub213ss_xmm_xmm_xmmm32 => (Operation::Sub, 213, 16, 4, true),
        VEX_Vfmsub213sd_xmm_xmm_xmmm64 => (Operation::Sub, 213, 16, 8, true),
        VEX_Vfmsub213ps_xmm_xmm_xmmm128 => (Operation::Sub, 213, 16, 4, false),
        VEX_Vfmsub213ps_ymm_ymm_ymmm256 => (Operation::Sub, 213, 32, 4, false),
        VEX_Vfmsub213pd_xmm_xmm_xmmm128 => (Operation::Sub, 213, 16, 8, false),
        VEX_Vfmsub213pd_ymm_ymm_ymmm256 => (Operation::Sub, 213, 32, 8, false),
        VEX_Vfmsub231ss_xmm_xmm_xmmm32 => (Operation::Sub, 231, 16, 4, true),
        VEX_Vfmsub231sd_xmm_xmm_xmmm64 => (Operation::Sub, 231, 16, 8, true),
        VEX_Vfmsub231ps_xmm_xmm_xmmm128 => (Operation::Sub, 231, 16, 4, false),
        VEX_Vfmsub231ps_ymm_ymm_ymmm256 => (Operation::Sub, 231, 32, 4, false),
        VEX_Vfmsub231pd_xmm_xmm_xmmm128 => (Operation::Sub, 231, 16, 8, false),
        VEX_Vfmsub231pd_ymm_ymm_ymmm256 => (Operation::Sub, 231, 32, 8, false),
        VEX_Vfnmadd132ss_xmm_xmm_xmmm32 => (Operation::NegativeAdd, 132, 16, 4, true),
        VEX_Vfnmadd132sd_xmm_xmm_xmmm64 => (Operation::NegativeAdd, 132, 16, 8, true),
        VEX_Vfnmadd132ps_xmm_xmm_xmmm128 => (Operation::NegativeAdd, 132, 16, 4, false),
        VEX_Vfnmadd132ps_ymm_ymm_ymmm256 => (Operation::NegativeAdd, 132, 32, 4, false),
        VEX_Vfnmadd132pd_xmm_xmm_xmmm128 => (Operation::NegativeAdd, 132, 16, 8, false),
        VEX_Vfnmadd132pd_ymm_ymm_ymmm256 => (Operation::NegativeAdd, 132, 32, 8, false),
        VEX_Vfnmadd213ss_xmm_xmm_xmmm32 => (Operation::NegativeAdd, 213, 16, 4, true),
        VEX_Vfnmadd213sd_xmm_xmm_xmmm64 => (Operation::NegativeAdd, 213, 16, 8, true),
        VEX_Vfnmadd213ps_xmm_xmm_xmmm128 => (Operation::NegativeAdd, 213, 16, 4, false),
        VEX_Vfnmadd213ps_ymm_ymm_ymmm256 => (Operation::NegativeAdd, 213, 32, 4, false),
        VEX_Vfnmadd213pd_xmm_xmm_xmmm128 => (Operation::NegativeAdd, 213, 16, 8, false),
        VEX_Vfnmadd213pd_ymm_ymm_ymmm256 => (Operation::NegativeAdd, 213, 32, 8, false),
        VEX_Vfnmadd231ss_xmm_xmm_xmmm32 => (Operation::NegativeAdd, 231, 16, 4, true),
        VEX_Vfnmadd231sd_xmm_xmm_xmmm64 => (Operation::NegativeAdd, 231, 16, 8, true),
        VEX_Vfnmadd231ps_xmm_xmm_xmmm128 => (Operation::NegativeAdd, 231, 16, 4, false),
        VEX_Vfnmadd231ps_ymm_ymm_ymmm256 => (Operation::NegativeAdd, 231, 32, 4, false),
        VEX_Vfnmadd231pd_xmm_xmm_xmmm128 => (Operation::NegativeAdd, 231, 16, 8, false),
        VEX_Vfnmadd231pd_ymm_ymm_ymmm256 => (Operation::NegativeAdd, 231, 32, 8, false),
        VEX_Vfnmsub132ss_xmm_xmm_xmmm32 => (Operation::NegativeSub, 132, 16, 4, true),
        VEX_Vfnmsub132sd_xmm_xmm_xmmm64 => (Operation::NegativeSub, 132, 16, 8, true),
        VEX_Vfnmsub132ps_xmm_xmm_xmmm128 => (Operation::NegativeSub, 132, 16, 4, false),
        VEX_Vfnmsub132ps_ymm_ymm_ymmm256 => (Operation::NegativeSub, 132, 32, 4, false),
        VEX_Vfnmsub132pd_xmm_xmm_xmmm128 => (Operation::NegativeSub, 132, 16, 8, false),
        VEX_Vfnmsub132pd_ymm_ymm_ymmm256 => (Operation::NegativeSub, 132, 32, 8, false),
        VEX_Vfnmsub213ss_xmm_xmm_xmmm32 => (Operation::NegativeSub, 213, 16, 4, true),
        VEX_Vfnmsub213sd_xmm_xmm_xmmm64 => (Operation::NegativeSub, 213, 16, 8, true),
        VEX_Vfnmsub213ps_xmm_xmm_xmmm128 => (Operation::NegativeSub, 213, 16, 4, false),
        VEX_Vfnmsub213ps_ymm_ymm_ymmm256 => (Operation::NegativeSub, 213, 32, 4, false),
        VEX_Vfnmsub213pd_xmm_xmm_xmmm128 => (Operation::NegativeSub, 213, 16, 8, false),
        VEX_Vfnmsub213pd_ymm_ymm_ymmm256 => (Operation::NegativeSub, 213, 32, 8, false),
        VEX_Vfnmsub231ss_xmm_xmm_xmmm32 => (Operation::NegativeSub, 231, 16, 4, true),
        VEX_Vfnmsub231sd_xmm_xmm_xmmm64 => (Operation::NegativeSub, 231, 16, 8, true),
        VEX_Vfnmsub231ps_xmm_xmm_xmmm128 => (Operation::NegativeSub, 231, 16, 4, false),
        VEX_Vfnmsub231ps_ymm_ymm_ymmm256 => (Operation::NegativeSub, 231, 32, 4, false),
        VEX_Vfnmsub231pd_xmm_xmm_xmmm128 => (Operation::NegativeSub, 231, 16, 8, false),
        VEX_Vfnmsub231pd_ymm_ymm_ymmm256 => (Operation::NegativeSub, 231, 32, 8, false),
        VEX_Vfmaddsub132ps_xmm_xmm_xmmm128 => (Operation::AddSub, 132, 16, 4, false),
        VEX_Vfmaddsub132ps_ymm_ymm_ymmm256 => (Operation::AddSub, 132, 32, 4, false),
        VEX_Vfmaddsub132pd_xmm_xmm_xmmm128 => (Operation::AddSub, 132, 16, 8, false),
        VEX_Vfmaddsub132pd_ymm_ymm_ymmm256 => (Operation::AddSub, 132, 32, 8, false),
        VEX_Vfmaddsub213ps_xmm_xmm_xmmm128 => (Operation::AddSub, 213, 16, 4, false),
        VEX_Vfmaddsub213ps_ymm_ymm_ymmm256 => (Operation::AddSub, 213, 32, 4, false),
        VEX_Vfmaddsub213pd_xmm_xmm_xmmm128 => (Operation::AddSub, 213, 16, 8, false),
        VEX_Vfmaddsub213pd_ymm_ymm_ymmm256 => (Operation::AddSub, 213, 32, 8, false),
        VEX_Vfmaddsub231ps_xmm_xmm_xmmm128 => (Operation::AddSub, 231, 16, 4, false),
        VEX_Vfmaddsub231ps_ymm_ymm_ymmm256 => (Operation::AddSub, 231, 32, 4, false),
        VEX_Vfmaddsub231pd_xmm_xmm_xmmm128 => (Operation::AddSub, 231, 16, 8, false),
        VEX_Vfmaddsub231pd_ymm_ymm_ymmm256 => (Operation::AddSub, 231, 32, 8, false),
        VEX_Vfmsubadd132ps_xmm_xmm_xmmm128 => (Operation::SubAdd, 132, 16, 4, false),
        VEX_Vfmsubadd132ps_ymm_ymm_ymmm256 => (Operation::SubAdd, 132, 32, 4, false),
        VEX_Vfmsubadd132pd_xmm_xmm_xmmm128 => (Operation::SubAdd, 132, 16, 8, false),
        VEX_Vfmsubadd132pd_ymm_ymm_ymmm256 => (Operation::SubAdd, 132, 32, 8, false),
        VEX_Vfmsubadd213ps_xmm_xmm_xmmm128 => (Operation::SubAdd, 213, 16, 4, false),
        VEX_Vfmsubadd213ps_ymm_ymm_ymmm256 => (Operation::SubAdd, 213, 32, 4, false),
        VEX_Vfmsubadd213pd_xmm_xmm_xmmm128 => (Operation::SubAdd, 213, 16, 8, false),
        VEX_Vfmsubadd213pd_ymm_ymm_ymmm256 => (Operation::SubAdd, 213, 32, 8, false),
        VEX_Vfmsubadd231ps_xmm_xmm_xmmm128 => (Operation::SubAdd, 231, 16, 4, false),
        VEX_Vfmsubadd231ps_ymm_ymm_ymmm256 => (Operation::SubAdd, 231, 32, 4, false),
        VEX_Vfmsubadd231pd_xmm_xmm_xmmm128 => (Operation::SubAdd, 231, 16, 8, false),
        VEX_Vfmsubadd231pd_ymm_ymm_ymmm256 => (Operation::SubAdd, 231, 32, 8, false),
        _ => return None,
    };
    Some(Form {
        operation,
        order,
        vector_bytes,
        element_bytes,
        scalar,
    })
}

#[cfg(test)]
#[path = "fused_float/tests.rs"]
mod tests;
