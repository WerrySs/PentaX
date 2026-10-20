// SPDX-License-Identifier: GPL-2.0-only

//! Exclusive kernel access to the actual eagerly committed native pages.
//! No guest address becomes a host pointer in core and no detached snapshot
//! or caller-constructed backing can implement this private adapter.

use pentax_kernel::{Failure, GuestAccess, GuestError, GuestWrite, MemoryEffect};
use pentax_linker::SymbolicLinkPlan;
use pentax_loader::ElfType;
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryError, NativeMemoryTransaction, NativeMemoryWrite,
    NativePermissions, NativeTransactionLimits, PausedNativeMemory,
};

pub(super) struct KernelMemory<'borrow, 'owner, 'image> {
    pub(super) paused: &'borrow mut PausedNativeMemory<'owner>,
    pub(super) source: Option<&'borrow SymbolicLinkPlan<'image>>,
    pub(super) supplied_modules: usize,
    pub(super) lookup_limits: Option<pentax_linker::ModuleLookupLimits>,
    pub(super) lookup_work: Option<&'borrow mut u64>,
    pub(super) loaded_modules:
        Option<&'borrow mut super::kernel_startup::module_lifecycle::LoadedModules>,
    pub(super) thread_tls: Option<&'borrow crate::process::tls::StaticTlsInstance>,
    pub(super) initial_entry: Option<super::kernel_startup::NativeEntryLayout>,
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod game_intent_tests;
mod image_memory;
mod module_symbols;
#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod mspace_tests;
#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod np_webapi2_tests;
mod tls;

/// Conservative fixed metadata/page visits, admitted before a live query.
/// This is not a byte-read charge: the adapter never copies the payload.
pub(super) fn process_parameter_work(source: &SymbolicLinkPlan<'_>) -> u64 {
    source
        .modules()
        .first()
        .and_then(|module| module.runtime_plan().process_parameters)
        .map_or(4, |range| range.size.div_ceil(crate::memory::PAGE_SIZE) + 8)
}

pub(super) fn error(value: NativeMemoryError, write: bool) -> GuestError {
    match value {
        NativeMemoryError::Unmapped(_) => GuestError::Unmapped,
        NativeMemoryError::PermissionDenied(_) if write => GuestError::WriteDenied,
        NativeMemoryError::PermissionDenied(_) => GuestError::ReadDenied,
        NativeMemoryError::InvalidRange => GuestError::AddressOverflow,
        NativeMemoryError::Allocation => GuestError::Allocation,
        NativeMemoryError::BudgetExceeded => GuestError::BudgetExceeded,
        NativeMemoryError::Overlap => GuestError::WriteDenied,
        // Poisoned is fatal: the owner rejects every subsequent pause/read/
        // write/fetch. Never translate this into a guest errno or retry it.
        _ => GuestError::HostFailure,
    }
}

impl GuestAccess for KernelMemory<'_, '_, '_> {
    fn initialize_network_backend(&mut self) -> Result<bool, Failure> {
        Ok(pentax_platform::network::initialize())
    }
    fn image_memory_work(&self, length: u64) -> Result<u64, Failure> {
        image_memory::work(self, length)
    }
    fn preflight_image_memory(&self, address: u64, length: u64) -> Result<bool, Failure> {
        image_memory::preflight(self, address, length)
    }
    fn set_virtual_range_name(
        &mut self,
        address: u64,
        length: u64,
        name: &[u8],
    ) -> Result<(), Failure> {
        self.paused
            .set_virtual_range_name(address, length, name, &Default::default())
            .map_err(name_error)
    }
    fn virtual_name_bounds(
        &mut self,
        address: u64,
        start: u64,
        end: u64,
        default: &[u8],
    ) -> Result<(u64, u64), Failure> {
        let result = self
            .paused
            .virtual_range_name(
                address,
                pentax_platform::native_memory::NativeRange {
                    address: start,
                    size: end - start,
                },
                default,
                &Default::default(),
            )
            .map_err(name_error)?;
        Ok((
            result.range.address,
            result.range.address + result.range.size,
        ))
    }
    fn time_counter(&self) -> Result<pentax_kernel::TimeCounterSample, Failure> {
        let sample = pentax_platform::time_counter::sample()
            .map_err(|_| Failure::Unsupported("native TSC clock domain"))?;
        Ok(pentax_kernel::TimeCounterSample {
            ticks: sample.ticks,
            frequency: sample.frequency,
        })
    }

    fn initial_thread_stack(&self) -> Result<pentax_kernel::InitialThreadStack, Failure> {
        self.preflight_pthread()?;
        let entry = self
            .initial_entry
            .ok_or(Failure::Unsupported("bound initial-thread stack"))?;
        if entry.stack.size > 8 * 1024 * 1024
            || entry.guards.iter().any(|guard| guard.size > 0x4000)
        {
            return Err(Failure::Limit("initial-thread stack inspection"));
        }
        self.paused
            .preflight_read(entry.stack.address, entry.stack.size)
            .map_err(|value| Failure::Guest(error(value, false)))?;
        self.paused
            .preflight_write(entry.stack.address, entry.stack.size)
            .map_err(|value| Failure::Guest(error(value, true)))?;
        // Guest logical RWX permissions are separate from physical NX backing.
        // Check every stack page, rather than assuming write permission means NX.
        if self
            .paused
            .preflight_non_executable(entry.stack.address, entry.stack.size)
            .is_err()
        {
            return Err(Failure::Unsupported("initial-thread stack must remain NX"));
        }
        // Guards must still belong to this owner and be unmapped.
        for guard in entry.guards {
            self.paused
                .preflight_reserved_unmapped(guard.address, guard.size)
                .map_err(|value| Failure::Guest(error(value, false)))?;
        }
        Ok(pentax_kernel::InitialThreadStack {
            address: entry.stack.address,
            size: entry.stack.size,
            guard_size: entry.guards[0].size,
        })
    }

    fn preflight_pthread(&self) -> Result<(), Failure> {
        if self.source.is_some()
            && self
                .thread_tls
                .and_then(|thread| thread.thread_control_block)
                .is_some()
        {
            Ok(())
        } else {
            Err(Failure::Unsupported("bound initial-thread pthread context"))
        }
    }
    fn preflight_posix_errno(&self, address: Option<u64>) -> Result<(), Failure> {
        if self.source.is_none()
            || self
                .thread_tls
                .and_then(|thread| thread.thread_control_block)
                .is_none()
        {
            return Err(Failure::Unsupported("bound initial-thread POSIX errno"));
        }
        if let Some(address) = address {
            self.paused
                .preflight_read(address, 4)
                .map_err(|error_value| Failure::Guest(error(error_value, false)))?;
            self.paused
                .preflight_write(address, 4)
                .map_err(|error_value| Failure::Guest(error(error_value, true)))?;
            if !matches!(
                self.paused.preflight_fetch_source(address, 4),
                Err(NativeMemoryError::PermissionDenied(_))
            ) {
                return Err(Failure::Unsupported("POSIX errno must remain NX"));
            }
        }
        Ok(())
    }
    fn preflight_tls(&self) -> Result<(), Failure> {
        if self.source.is_some()
            && self.thread_tls.is_some()
            && self.lookup_limits.is_some()
            && self.lookup_work.is_some()
        {
            Ok(())
        } else {
            Err(Failure::Unsupported("image-bound loader/TLS integration"))
        }
    }
    fn tls_address(&mut self, descriptor: u64) -> Result<u64, Failure> {
        tls::address(self, descriptor)
    }
    fn retain_loaded_module(&mut self, path: &str, result: Option<u64>) -> Result<i32, Failure> {
        super::kernel_startup::module_lifecycle::retain(self, path, result)
    }
    fn module_symbol(&mut self, handle: i32, name: &str) -> Result<Option<u64>, Failure> {
        module_symbols::lookup(self, handle, name)
    }
    fn process_parameters(&self) -> Result<u64, Failure> {
        let source = self
            .source
            .ok_or(Failure::Unsupported("image-bound process parameters"))?;
        let module = source
            .modules()
            .first()
            .ok_or(Failure::Unsupported("missing primary executable"))?;
        let runtime = module.runtime_plan();
        let Some(range) = runtime.process_parameters else {
            // Known absence in the actual retained primary is the source's
            // null result. No provider's metadata substitutes for the primary.
            return Ok(0);
        };
        if !matches!(
            runtime.metadata.elf_type,
            ElfType::Executable | ElfType::SceExecutable | ElfType::SceDynamicExecutable
        ) {
            return Err(Failure::Unsupported(
                "process parameters require executable primary",
            ));
        }
        if range.size == 0 {
            return Err(Failure::Unsupported("empty process-parameter extent"));
        }
        let address = module
            .load_bias()
            .checked_add(range.address)
            .ok_or(Failure::Guest(GuestError::AddressOverflow))?;
        self.paused
            .preflight_read(address, range.size)
            .map_err(|value| Failure::Guest(error(value, false)))?;
        Ok(address)
    }

    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), GuestError> {
        self.paused
            .read(address, output)
            .map_err(|e| error(e, false))
    }

    fn preflight_write(&self, address: u64, length: u64) -> Result<(), GuestError> {
        self.paused
            .preflight_write(address, length)
            .map_err(|e| error(e, true))
    }

    fn preflight_heap_arena(&self, address: u64, size: u64) -> Result<(), GuestError> {
        // The engine precharges all three metadata walks. Prove actual live
        // same-owner RW/NX backing without reading, preparing or mapping it.
        self.paused
            .preflight_read(address, size)
            .map_err(|e| error(e, false))?;
        self.paused
            .preflight_write(address, size)
            .map_err(|e| error(e, true))?;
        self.paused
            .preflight_non_executable(address, size)
            .map_err(|e| error(e, false))
    }

    fn preflight_distinct_backing(&self, address: u64, length: u64) -> Result<(), GuestError> {
        if length > 32 {
            return Err(GuestError::UnsupportedMapping);
        }
        // Metadata only: mapped ownership and physical-byte uniqueness do not
        // grant read/write permission and do not read the response descriptor.
        self.paused
            .preflight_distinct_backing(address, length, 32)
            .map(|_| ())
            .map_err(|e| error(e, false))
    }

    fn read_after_u64_write_byte(
        &self,
        address: u64,
        destination: u64,
        value: u64,
    ) -> Result<u8, GuestError> {
        use pentax_platform::native_memory::NativeRange;
        self.paused
            .preflight_write(destination, 8)
            .map_err(|e| error(e, true))?;
        let mut byte = [0];
        self.paused
            .read(address, &mut byte)
            .map_err(|e| error(e, false))?;
        for (index, value) in value.to_le_bytes().into_iter().enumerate() {
            let at = destination
                .checked_add(index as u64)
                .ok_or(GuestError::AddressOverflow)?;
            if at == address {
                byte[0] = value;
            } else {
                // Exact virtual overlaps are staged. A different virtual view
                // of an edited physical byte is an explicit refusal.
                self.paused
                    .preflight_disjoint_backing(
                        NativeRange { address, size: 1 },
                        NativeRange {
                            address: at,
                            size: 1,
                        },
                        16,
                    )
                    .map_err(|e| error(e, false))?;
            }
        }
        Ok(byte[0])
    }

    fn read_after_writes(
        &self,
        address: u64,
        output: &mut [u8],
        writes: &[GuestWrite<'_>],
    ) -> Result<(), GuestError> {
        use pentax_platform::native_memory::NativeRange;
        let total = writes
            .iter()
            .try_fold(0usize, |total, write| total.checked_add(write.bytes.len()));
        if output.len() > 128 || writes.len() > 16 || total.is_none_or(|n| n > 1024 * 1024) {
            return Err(GuestError::UnsupportedMapping);
        }
        if output.is_empty() {
            return Ok(());
        }
        let end = address
            .checked_add(output.len() as u64)
            .ok_or(GuestError::AddressOverflow)?;
        // A source field read still needs readable backing, even when a prior
        // staged store supplies its value. Do not read the overwritten OLD
        // payload: only uncovered fragments need their original bytes.
        self.paused
            .preflight_read(address, output.len() as u64)
            .map_err(|e| error(e, false))?;
        let mut covered = [false; 128];
        output.fill(0);
        for write in writes {
            if write.bytes.is_empty() {
                continue;
            }
            let write_end = write
                .address
                .checked_add(write.bytes.len() as u64)
                .ok_or(GuestError::AddressOverflow)?;
            self.paused
                .preflight_write(write.address, write.bytes.len() as u64)
                .map_err(|e| error(e, true))?;
            self.paused
                .preflight_distinct_backing(write.address, write.bytes.len() as u64, 16_384)
                .map_err(|e| error(e, false))?;
            let first = address.max(write.address);
            let last = end.min(write_end);
            let check = |start: u64, stop: u64| -> Result<(), GuestError> {
                if start < stop {
                    self.paused
                        .preflight_disjoint_backing(
                            NativeRange {
                                address: start,
                                size: stop - start,
                            },
                            NativeRange {
                                address: write.address,
                                size: write.bytes.len() as u64,
                            },
                            16_384,
                        )
                        .map_err(|e| error(e, false))?;
                }
                Ok(())
            };
            if first < last {
                check(address, first)?;
                check(last, end)?;
                output[(first - address) as usize..(last - address) as usize].copy_from_slice(
                    &write.bytes[(first - write.address) as usize..(last - write.address) as usize],
                );
                covered[(first - address) as usize..(last - address) as usize].fill(true);
            } else {
                check(address, end)?;
            }
        }
        let mut offset = 0;
        while offset < output.len() {
            if covered[offset] {
                offset += 1;
                continue;
            }
            let start = offset;
            while offset < output.len() && !covered[offset] {
                offset += 1;
            }
            self.paused
                .read(address + start as u64, &mut output[start..offset])
                .map_err(|e| error(e, false))?;
        }
        Ok(())
    }

    fn apply(&mut self, effect: MemoryEffect, writes: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        let permissions = |protection: u32| {
            // CPU and GPU permissions remain separate on the same backing.
            // Only the kernel's decoded access bits belong at this boundary.
            if protection & !0x37 != 0 {
                return Err(GuestError::UnsupportedMapping);
            }
            Ok(NativePermissions::new(
                protection & 1 != 0,
                protection & 2 != 0,
                protection & 4 != 0,
            )
            .with_gpu(protection & 0x10 != 0, protection & 0x20 != 0))
        };
        let effect = match effect {
            MemoryEffect::None => NativeMemoryEffect::None,
            MemoryEffect::AllocateDirect {
                physical, length, ..
            } => NativeMemoryEffect::AllocatePhysical {
                physical,
                size: length,
            },
            MemoryEffect::ReleaseDirect { physical, length } => {
                NativeMemoryEffect::ReleasePhysical {
                    physical,
                    size: length,
                }
            }
            MemoryEffect::MapDirect {
                address,
                length,
                protection,
                physical,
            } => NativeMemoryEffect::MapPhysical {
                address,
                size: length,
                physical,
                permissions: permissions(protection)?,
            },
            MemoryEffect::MapZeroed {
                address,
                length,
                protection,
            } => NativeMemoryEffect::MapZeroed {
                address,
                size: length,
                permissions: permissions(protection)?,
            },
            MemoryEffect::Protect {
                address,
                length,
                protection,
            } => NativeMemoryEffect::Protect {
                address,
                size: length,
                permissions: permissions(protection)?,
            },
            MemoryEffect::Unmap { address, length } => NativeMemoryEffect::Unmap {
                address,
                size: length,
            },
        };
        // Small service transactions retain four stack descriptors. A Font
        // image can also contain many disjoint nonzero coverage runs; preserve
        // the untouched holes in one atomic transaction. Its kernel plan has
        // admitted both descriptor tables before this fallible conversion.
        const MAX_OUTPUTS: usize = 16_512;
        if writes.len() > MAX_OUTPUTS {
            return Err(GuestError::HostFailure);
        }
        if writes.len() > 4
            && writes
                .iter()
                .try_fold(0usize, |total, write| total.checked_add(write.bytes.len()))
                .is_none_or(|total| total > 4 * 1024 * 1024)
        {
            return Err(GuestError::UnsupportedMapping);
        }
        let limits = NativeTransactionLimits {
            max_outputs: MAX_OUTPUTS,
            max_output_bytes: 32 * 1024 * 1024 + 1328 + 8,
            max_preflight_steps: 8 * 1024 * 1024,
            ..Default::default()
        };
        let mut outputs = [NativeMemoryWrite {
            address: 0,
            bytes: &[],
        }; 4];
        let mut dynamic = Vec::new();
        let outputs = if writes.len() > outputs.len() {
            // Equal field layouts let the kernel charge this second table as
            // GuestWrite descriptors without depending on the native backend.
            const {
                assert!(
                    std::mem::size_of::<NativeMemoryWrite<'_>>()
                        == std::mem::size_of::<GuestWrite<'_>>()
                );
            }
            dynamic
                .try_reserve_exact(writes.len())
                .map_err(|_| GuestError::HostFailure)?;
            if dynamic.capacity() != writes.len() {
                return Err(GuestError::HostFailure);
            }
            dynamic.resize(
                writes.len(),
                NativeMemoryWrite {
                    address: 0,
                    bytes: &[],
                },
            );
            dynamic.as_mut_slice()
        } else {
            &mut outputs[..writes.len()]
        };
        for (output, write) in outputs.iter_mut().zip(writes) {
            *output = NativeMemoryWrite {
                address: write.address,
                bytes: write.bytes,
            };
        }
        self.paused
            .apply_transaction(
                NativeMemoryTransaction {
                    effect,
                    writes: outputs,
                },
                &limits,
            )
            .map_err(|e| error(e, true))
    }
}

fn name_error(value: NativeMemoryError) -> Failure {
    if value == NativeMemoryError::BudgetExceeded {
        Failure::Limit("virtual range name metadata")
    } else {
        Failure::Guest(error(value, false))
    }
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod save_output_tests {
    use super::*;
    use pentax_platform::native_memory::{NativeAddressSpace, NativeMemoryLimits};

    #[test]
    fn font_staged_native_reads_observe_last_store_without_publishing_and_refuse_bad_permissions() {
        const PAGE: u64 = 16384;
        let mut space =
            NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 4 * PAGE).unwrap();
        let base = space.reserved_ranges().next().unwrap().address;
        space
            .map_at(base, PAGE, NativePermissions::READ_WRITE)
            .unwrap();
        let mut paused = space.pause().unwrap();
        let mut guest = KernelMemory {
            paused: &mut paused,
            source: None,
            supplied_modules: 0,
            lookup_limits: None,
            lookup_work: None,
            loaded_modules: None,
            thread_tls: None,
            initial_entry: None,
        };
        guest
            .apply(
                MemoryEffect::None,
                &[GuestWrite {
                    address: base,
                    bytes: &[0x5a; 32],
                }],
            )
            .unwrap();
        let stores = [
            GuestWrite {
                address: base + 4,
                bytes: &[0x11; 8],
            },
            GuestWrite {
                address: base + 8,
                bytes: &[0x22; 8],
            },
        ];
        let mut actual = [0; 32];
        guest.read_after_writes(base, &mut actual, &stores).unwrap();
        let mut expected = [0x5a; 32];
        expected[4..8].fill(0x11);
        expected[8..16].fill(0x22);
        assert_eq!(actual, expected);
        guest.read(base, &mut actual).unwrap();
        assert_eq!(actual, [0x5a; 32]);
        guest
            .read_after_writes(base + 4, &mut actual[..12], &stores)
            .unwrap();
        assert_eq!(&actual[..12], &expected[4..16]);
        let bad_store = [GuestWrite {
            address: base + PAGE,
            bytes: &[0x33; 4],
        }];
        assert!(
            guest
                .read_after_writes(base, &mut actual, &bad_store)
                .is_err()
        );
        guest
            .apply(
                MemoryEffect::Protect {
                    address: base,
                    length: PAGE,
                    protection: 2,
                },
                &[],
            )
            .unwrap();
        let fully_covered = [GuestWrite {
            address: base,
            bytes: &[0x44; 32],
        }];
        assert!(
            guest
                .read_after_writes(base, &mut actual, &fully_covered)
                .is_err()
        );
        guest
            .apply(
                MemoryEffect::Protect {
                    address: base,
                    length: PAGE,
                    protection: 3,
                },
                &[],
            )
            .unwrap();
        guest.read(base, &mut actual).unwrap();
        assert_eq!(actual, [0x5a; 32]);
    }

    #[test]
    fn full_save_memory_outputs_preflight_all_native_pages_before_any_copy() {
        const SIZE: u64 = 32 * 1024 * 1024;
        const PAGE: u64 = 16384;
        let mut space =
            NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), SIZE + 4 * PAGE)
                .unwrap();
        let base = space.reserved_ranges().next().unwrap().address;
        space
            .map_at(base, SIZE + 2 * PAGE, NativePermissions::READ_WRITE)
            .unwrap();
        let mut paused = space.pause().unwrap();
        let mut guest = KernelMemory {
            paused: &mut paused,
            source: None,
            supplied_modules: 0,
            lookup_limits: None,
            lookup_work: None,
            loaded_modules: None,
            thread_tls: None,
            initial_entry: None,
        };
        let data = vec![0x5a; SIZE as usize];
        let param = [0x37; 1328];
        let zero = [0; 8];
        let mut writes = [
            GuestWrite {
                address: base,
                bytes: &data,
            },
            GuestWrite {
                address: base + SIZE,
                bytes: &param,
            },
            GuestWrite {
                address: base + SIZE + 3 * PAGE,
                bytes: &zero,
            },
        ];
        assert_eq!(
            guest.apply(MemoryEffect::None, &writes),
            Err(GuestError::Unmapped)
        );
        let mut actual = [0; 8];
        guest.read(base, &mut actual).unwrap();
        assert_eq!(actual, [0; 8]);
        writes[2].address = base + SIZE + PAGE;
        guest.apply(MemoryEffect::None, &writes).unwrap();
        for offset in [0, SIZE - 8] {
            guest.read(base + offset, &mut actual).unwrap();
            assert_eq!(actual, [0x5a; 8]);
        }
        guest.read(base + SIZE, &mut actual).unwrap();
        assert_eq!(actual, [0x37; 8]);
    }
}
