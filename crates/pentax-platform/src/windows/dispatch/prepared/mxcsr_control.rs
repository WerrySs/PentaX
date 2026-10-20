// SPDX-License-Identifier: GPL-2.0-only
//! Guest MXCSR transfers and the checked host-compatible masked-exception profile.

use pentax_codegen::Code;
use std::sync::OnceLock;

pub(super) fn store(code: Code) -> Option<bool> {
    match code {
        Code::Stmxcsr_m32 | Code::VEX_Vstmxcsr_m32 => Some(true),
        Code::Ldmxcsr_m32 | Code::VEX_Vldmxcsr_m32 => Some(false),
        _ => None,
    }
}

pub(super) fn mask() -> u32 {
    static MASK: OnceLock<u32> = OnceLock::new();
    *MASK.get_or_init(|| {
        if !std::is_x86_feature_detected!("fxsr") {
            return 0xffbf;
        }
        #[repr(align(16))]
        struct Snapshot([u8; 512]);
        let mut snapshot = Snapshot([0; 512]);
        // SAFETY: FXSR was checked, and this initialized, exclusively owned
        // 512-byte area has the required 16-byte alignment. FXSAVE only stores
        // host state and changes no control or arithmetic register. No saved
        // state is restored, executed, logged or retained outside this frame.
        unsafe { std::arch::x86_64::_fxsave64(snapshot.0.as_mut_ptr()) };
        let reported = u32::from_le_bytes(snapshot.0[28..32].try_into().unwrap());
        // Intel specifies 0xffbf when MXCSR_MASK is reported as zero. Keep the
        // architectural low 16-bit ceiling even on a malformed virtual host.
        if reported == 0 {
            0xffbf
        } else {
            reported & 0xffff
        }
    })
}

pub(super) fn valid(value: u32) -> bool {
    value & !mask() == 0 && value & 0x1f80 == 0x1f80
}
