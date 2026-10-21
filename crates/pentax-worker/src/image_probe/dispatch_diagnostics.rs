// SPDX-License-Identifier: GPL-2.0-only
//! Closed failure categories only; never reflect error payloads or Debug text.
use pentax_codegen::MemoryAccessError;
use pentax_platform::{
    PlatformError, native_dispatch::NativeDispatchError as D, native_memory::NativeMemoryError as M,
};

pub(super) fn classify(error: &D) -> &'static str {
    match error {
        D::Memory(error) => match error {
            M::Unmapped(_) => "memory-unmapped",
            M::PermissionDenied(_) => "memory-permission",
            M::InvalidRange => "memory-range",
            M::Unaligned => "memory-alignment",
            M::Overlap => "memory-overlap",
            M::BudgetExceeded => "memory-budget",
            M::Allocation => "memory-allocation",
            M::Poisoned => "memory-poisoned",
            M::OwnerIdentityExhausted => "memory-owner-limit",
            M::OperatingSystem { .. } => "memory-os",
            M::UnsupportedHost | M::UnsupportedPageLayout => "memory-host",
            _ => "memory-other",
        },
        D::Platform(PlatformError::UnsupportedHostFeature { .. }) => "host-feature",
        D::Platform(_) => "host-boundary",
        D::Budget(_) => "dispatch-budget",
        D::WrongOwner => "dispatch-owner",
        D::WrongSource | D::SourceMismatch | D::WritableSource => "dispatch-source",
        D::WrongPhase => "dispatch-phase",
        D::InvalidState => "invalid-cpu-state",
        D::InvalidStack => "invalid-stack",
        D::InvalidTarget(_) => "invalid-target",
        D::EmissionMismatch => "dispatch-emission",
        D::Allocation => "dispatch-allocation",
        D::MemoryFootprint { reason, .. } => match reason {
            MemoryAccessError::NotEmitted => "footprint-not-emitted",
            MemoryAccessError::UnsupportedAddress => "footprint-address",
            MemoryAccessError::UnsupportedSize => "footprint-size",
            MemoryAccessError::UnsupportedBitString => "footprint-bit-string",
            MemoryAccessError::TooManyAccesses => "footprint-access-count",
            MemoryAccessError::InvalidRange => "footprint-range",
            MemoryAccessError::UnalignedAtomic => "footprint-atomic-alignment",
            MemoryAccessError::UnalignedNonTemporal => "footprint-nt-alignment",
        },
        D::MemoryIsolationRequired => "memory-isolation",
        D::UnsupportedSegmentInstruction => "segment-form",
        D::UnconfiguredSegment => "segment-base",
        D::UnsupportedStringInstruction => "string-form",
        D::UnsupportedVectorInstruction => "vector-form",
        D::UnsupportedIntegerInstruction => "integer-form",
        D::UnsupportedCpuInstruction => "cpu-form",
        D::TimeCounter(_) => "time-counter",
        D::UnsupportedX87Exception => "x87-exception",
        D::VectorAlignment => "vector-alignment",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dispatch_labels_do_not_reflect_private_addresses_or_unlisted_text() {
        for (error, expected) in [
            (D::Memory(M::Unmapped(u64::MAX)), "memory-unmapped"),
            (
                D::Memory(M::OperatingSystem {
                    operation: "private-operation",
                    address: u64::MAX,
                    code: 998,
                }),
                "memory-os",
            ),
            (D::Budget("private-limit"), "dispatch-budget"),
            (
                D::Platform(PlatformError::UnsupportedHostFeature {
                    feature: "private-feature",
                }),
                "host-feature",
            ),
            (D::InvalidTarget(u64::MAX), "invalid-target"),
            (D::UnsupportedVectorInstruction, "vector-form"),
            (D::UnsupportedCpuInstruction, "cpu-form"),
        ] {
            assert_eq!(classify(&error), expected);
        }
        for (reason, expected) in [
            (MemoryAccessError::NotEmitted, "footprint-not-emitted"),
            (MemoryAccessError::UnsupportedAddress, "footprint-address"),
            (MemoryAccessError::UnsupportedSize, "footprint-size"),
            (
                MemoryAccessError::UnsupportedBitString,
                "footprint-bit-string",
            ),
            (MemoryAccessError::TooManyAccesses, "footprint-access-count"),
            (MemoryAccessError::InvalidRange, "footprint-range"),
            (
                MemoryAccessError::UnalignedAtomic,
                "footprint-atomic-alignment",
            ),
            (
                MemoryAccessError::UnalignedNonTemporal,
                "footprint-nt-alignment",
            ),
        ] {
            assert_eq!(
                classify(&D::MemoryFootprint {
                    reason,
                    form: pentax_codegen::Code::INVALID,
                    instruction_index: 0,
                }),
                expected
            );
        }
    }
}
