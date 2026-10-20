// SPDX-License-Identifier: GPL-2.0-only

//! Explicit runtime memory policy; compact fixture defaults stay unchanged.

use super::{NativeImageFileBacking, NativeImageLimits};
use pentax_platform::host_memory::HostMemoryAvailability;
use pentax_platform::native_memory::NATIVE_ADDRESS_LIMIT;

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
// Pinned 13.5 GiB guest memory profile plus 512 MiB native startup allowance.
// Pagefile mode selects this ceiling only when both host snapshots retain
// headroom. Explicit file mode uses it for logical capacity and keeps the
// smaller host-derived allowance as an independent private-commitment cap.
const RUNTIME_BACKING_CEILING: u64 = 14 * GIB;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeResourceError {
    InsufficientHostMemory,
}
impl NativeResourceError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InsufficientHostMemory => "host-memory-budget",
        }
    }
}

impl NativeImageLimits {
    /// Apply the pinned Kyty direct-capacity profile with a separate host cap.
    /// The snapshot is volatile: actual OS allocations can still fail. Native
    /// backing and mapped aliases each retain the selected aggregate cap.
    /// Input buffers, compiler state and Vulkan allocations are outside it.
    pub fn with_runtime_memory(
        mut self,
        host: HostMemoryAvailability,
    ) -> Result<Self, NativeResourceError> {
        // Keep half of scarce physical memory, or a 6 GiB snapshot margin on
        // larger hosts. The former 8 GiB ceiling could not admit the pinned
        // guest pools even with ample host resources. Commitment independently
        // retains half when scarce, or a 2 GiB margin when more is available.
        // Round down, never up; no multi-GiB commitment occurs here.
        let physical_budget =
            (host.physical_bytes / 2).max(host.physical_bytes.saturating_sub(6 * GIB));
        let commit_budget = (host.commit_bytes / 2).max(host.commit_bytes.saturating_sub(2 * GIB));
        let bytes = physical_budget
            .min(commit_budget)
            .min(RUNTIME_BACKING_CEILING);
        let bytes = bytes / MIB * MIB;
        if bytes < GIB {
            return Err(NativeResourceError::InsufficientHostMemory);
        }
        // Pinned source 209dffe: 13824 MiB total minus default 1024 MiB flexible.
        // Addressable guest capacity does not authorize equal host commitment.
        self.startup.kernel.direct_memory_bytes = 12_800 * MIB;
        // Address headroom for direct aliases and flexible mappings. This is a
        // PentaX virtual-window policy, not a claim about firmware layout.
        self.kernel_window_bytes = 32 * GIB;
        // Reserve address space for requested thread stacks/TLS; commitment is
        // still lazy and shares the same aggregate physical/commitment cap.
        self.thread_window_bytes = 2 * GIB;
        self.startup.startup.static_tls.max_storage_bytes = bytes;
        self.startup.kernel.virtual_bytes = self.kernel_window_bytes;
        // The pinned nonfixed search honors a nonzero hint as its lower bound;
        // it is not restricted to the initial arena. The native owner already
        // reserves missing exact placeholder slots atomically before mapping,
        // refuses collisions and retains rollback/poisoning and aggregate caps.
        self.startup.kernel.nonfixed_virtual_end = Some(NATIVE_ADDRESS_LIMIT);
        self.startup.kernel.max_mapped_bytes = bytes - 512 * MIB;
        // Memory effects currently charge bytes for allocation and mapping.
        // Admit two allocation/map cycles; attempts, I/O and copied-byte caps
        // retain their independent compact values.
        self.startup.kernel.max_work = 4 * bytes + 32 * MIB;
        self.native.session.mapped_limit = bytes;
        self.native.session.resident_limit = bytes;
        self.defer_physical_commit = true;
        self.file_backing = None;
        // Permit at most one host-capped mapping allowance outside the initial
        // arena. This is an address-space ceiling, never eager commitment.
        self.native.reserved_limit =
            self.kernel_window_bytes + self.thread_window_bytes + GIB + bytes;
        Ok(self)
    }

    /// Explicit sparse-file physical storage: logical backing and alias capacity
    /// follow the guest pools while private pagefile commitment keeps the same
    /// host-derived ceiling. No file is opened here; preparation captures the
    /// selected directory and validates disk resources before guest execution.
    pub fn with_file_backed_runtime_memory(
        self,
        host: HostMemoryAvailability,
        directory: std::path::PathBuf,
    ) -> Result<Self, NativeResourceError> {
        let mut result = self.with_runtime_memory(host)?;
        let private_commit_limit_bytes = result.native.session.resident_limit;
        result.native.session.mapped_limit = RUNTIME_BACKING_CEILING;
        result.native.session.resident_limit = RUNTIME_BACKING_CEILING;
        result.native.reserved_limit =
            result.kernel_window_bytes + result.thread_window_bytes + GIB + RUNTIME_BACKING_CEILING;
        result.startup.kernel.max_mapped_bytes = RUNTIME_BACKING_CEILING - 512 * MIB;
        result.startup.kernel.max_work = 4 * RUNTIME_BACKING_CEILING + 32 * MIB;
        result.defer_physical_commit = false;
        result.file_backing = Some(NativeImageFileBacking {
            directory,
            capacity_bytes: result.startup.kernel.direct_memory_bytes,
            private_commit_limit_bytes,
        });
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_backed_runtime_keeps_private_commit_separate_from_guest_pool_capacity() {
        for (physical, commit, private) in [
            (24 * GIB, 3 * GIB, 1536 * MIB),
            (3 * GIB, 24 * GIB, 1536 * MIB),
            (12 * GIB, 24 * GIB, 6 * GIB),
            (32 * GIB, 24 * GIB, 14 * GIB),
        ] {
            let host = HostMemoryAvailability {
                physical_bytes: physical,
                commit_bytes: commit,
            };
            let limits = NativeImageLimits::default()
                .with_file_backed_runtime_memory(
                    host,
                    std::path::PathBuf::from("original-unused-directory"),
                )
                .unwrap();
            let backing = limits.file_backing.as_ref().unwrap();
            assert_eq!(backing.capacity_bytes, 12_800 * MIB);
            assert_eq!(backing.private_commit_limit_bytes, private);
            assert_eq!(limits.native.session.resident_limit, 14 * GIB);
            assert_eq!(limits.native.session.mapped_limit, 14 * GIB);
            assert_eq!(limits.startup.startup.static_tls.max_storage_bytes, private);
            assert_eq!(limits.startup.kernel.max_mapped_bytes, 14 * GIB - 512 * MIB);
            assert_eq!(limits.native.reserved_limit, 49 * GIB);
            assert!(!limits.defer_physical_commit);
            let restored = limits.with_runtime_memory(host).unwrap();
            assert!(restored.file_backing.is_none());
            assert!(restored.defer_physical_commit);
            assert_eq!(restored.native.session.resident_limit, private);
        }
        assert_eq!(
            NativeImageLimits::default()
                .with_file_backed_runtime_memory(
                    HostMemoryAvailability {
                        physical_bytes: 0,
                        commit_bytes: 0
                    },
                    std::path::PathBuf::new()
                )
                .unwrap_err(),
            NativeResourceError::InsufficientHostMemory
        );
    }

    #[test]
    fn runtime_memory_bounds_host_commit_separately_from_guest_capacity() {
        for (physical, commit, expected) in [
            (32 * GIB, 24 * GIB, 14 * GIB),
            (20 * GIB, 24 * GIB, 14 * GIB),
            (20 * GIB - MIB, 24 * GIB, 14 * GIB - MIB),
            (20 * GIB + MIB, 24 * GIB, 14 * GIB),
            (12 * GIB, 24 * GIB, 6 * GIB),
            (12 * GIB - MIB, 24 * GIB, 6143 * MIB),
            (12 * GIB + 2 * MIB, 24 * GIB, 6 * GIB + 2 * MIB),
            (16 * GIB, 13 * GIB, 10 * GIB),
            (5 * GIB + MIB, 24 * GIB, 2560 * MIB),
            (24 * GIB, 3 * GIB, 1536 * MIB),
            (24 * GIB, 6 * GIB, 4 * GIB),
            (24 * GIB, 4 * GIB, 2 * GIB),
            (24 * GIB, 4 * GIB - 1, 2047 * MIB),
            (2 * GIB, 2 * GIB, GIB),
            (u64::MAX, u64::MAX, 14 * GIB),
        ] {
            let compact = NativeImageLimits::default();
            let runtime = compact
                .clone()
                .with_runtime_memory(HostMemoryAvailability {
                    physical_bytes: physical,
                    commit_bytes: commit,
                })
                .unwrap();
            assert_eq!(runtime.native.session.resident_limit, expected);
            assert_eq!(runtime.native.session.mapped_limit, expected);
            assert!(physical - expected >= (physical / 2).min(6 * GIB));
            assert!(commit - expected >= (commit / 2).min(2 * GIB));
            assert!(runtime.defer_physical_commit);
            assert!(!compact.defer_physical_commit);
            assert_eq!(runtime.startup.kernel.direct_memory_bytes, 12_800 * MIB);
            assert_eq!(
                runtime.startup.kernel.max_mapped_bytes + 512 * MIB,
                expected
            );
            assert!(runtime.kernel_window_bytes >= 2 * runtime.startup.kernel.direct_memory_bytes);
            assert!(runtime.native.reserved_limit > runtime.kernel_window_bytes);
            assert_eq!(
                runtime.startup.kernel.nonfixed_virtual_end,
                Some(NATIVE_ADDRESS_LIMIT)
            );
            assert_eq!(compact.startup.kernel.nonfixed_virtual_end, None);
            assert_eq!(
                runtime.native.reserved_limit,
                runtime.kernel_window_bytes + runtime.thread_window_bytes + GIB + expected
            );
            assert_eq!(runtime.thread_window_bytes, 2 * GIB);
            assert_eq!(
                runtime.startup.startup.static_tls.max_storage_bytes,
                expected
            );
            assert!(runtime.startup.kernel.max_work > 4 * expected);
            assert_eq!(
                runtime.startup.kernel.max_bytes,
                compact.startup.kernel.max_bytes
            );
            assert_eq!(
                runtime.startup.kernel.max_attempts,
                compact.startup.kernel.max_attempts
            );
            assert_eq!(runtime.max_input_bytes, compact.max_input_bytes);
        }
        for host in [
            HostMemoryAvailability {
                physical_bytes: 2 * GIB - 1,
                commit_bytes: u64::MAX,
            },
            HostMemoryAvailability {
                physical_bytes: u64::MAX,
                commit_bytes: 2 * GIB - 1,
            },
            HostMemoryAvailability {
                physical_bytes: 0,
                commit_bytes: 0,
            },
        ] {
            assert_eq!(
                NativeImageLimits::default()
                    .with_runtime_memory(host)
                    .unwrap_err(),
                NativeResourceError::InsufficientHostMemory
            );
        }
        assert_eq!(
            NativeImageLimits::default()
                .startup
                .kernel
                .direct_memory_bytes,
            64 * MIB
        );
    }
}
