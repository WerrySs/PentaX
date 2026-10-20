// SPDX-License-Identifier: GPL-2.0-only
// Original PentaX tests using original bytes and private temporary file data.

use super::*;
#[path = "tests/ampr.rs"]
mod ampr;
#[path = "tests/apr.rs"]
mod apr;
#[path = "tests/apr_submit.rs"]
mod apr_submit;
#[path = "tests/dialog.rs"]
mod dialog;
#[path = "tests/fiber.rs"]
mod fiber;
#[path = "tests/kernel_semaphore_wait.rs"]
mod kernel_semaphore_wait;
#[path = "tests/net_ctl.rs"]
mod net_ctl;

#[path = "tests/application_heap.rs"]
mod application_heap;
#[path = "tests/named_direct.rs"]
mod named_direct;
#[path = "tests/virtual_names.rs"]
mod virtual_names;

#[path = "tests/library_results.rs"]
mod library_results;

#[path = "tests/posix.rs"]
mod posix;

#[path = "tests/calendar.rs"]
mod calendar;
#[path = "tests/hardware_mode.rs"]
mod hardware_mode;
#[path = "tests/np.rs"]
mod np;
#[path = "tests/output.rs"]
mod output;
#[path = "tests/pad.rs"]
mod pad;
#[path = "tests/save.rs"]
mod save;
#[path = "tests/save_memory.rs"]
mod save_memory;
#[path = "tests/sysmodule.rs"]
mod sysmodule;

#[path = "tests/share.rs"]
mod share;
#[path = "tests/system.rs"]
mod system;
#[path = "tests/system_display.rs"]
mod system_display;
#[path = "tests/system_events.rs"]
mod system_events;
#[path = "tests/user.rs"]
mod user;

#[path = "tests/pthread_affinity.rs"]
mod pthread_affinity;
#[path = "tests/pthread_attr.rs"]
mod pthread_attr;
#[path = "tests/pthread_condition.rs"]
mod pthread_condition;
#[path = "tests/pthread_condition_wait.rs"]
mod pthread_condition_wait;
#[path = "tests/pthread_mutex.rs"]
mod pthread_mutex;
#[path = "tests/pthread_rwlock.rs"]
mod pthread_rwlock;
#[path = "tests/pthread_semaphore.rs"]
mod pthread_semaphore;

#[path = "tests/pthread_keys.rs"]
mod pthread_keys;

#[path = "tests/pthread_create.rs"]
mod pthread_create;
#[path = "tests/pthread_once.rs"]
mod pthread_once;

#[cfg(windows)]
#[path = "tests/directories.rs"]
mod directories;
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) const ARG: u64 = 0x10000;
pub(crate) const PAGE: u64 = 0x4000;
struct Region {
    start: u64,
    length: u64,
    protection: u32,
    physical: Option<u64>,
    bytes: Vec<u8>,
}
pub(crate) struct TestGuest {
    pub(crate) network_calls: usize,
    pub(crate) network_backend_result: bool,
    regions: Vec<Region>,
    physical: Vec<(u64, Vec<u8>)>,
    pub(crate) refuse: bool,
    effects: usize,
}

impl TestGuest {
    pub(crate) fn new() -> Self {
        Self {
            network_calls: 0,
            network_backend_result: false,
            regions: vec![Region {
                start: ARG,
                length: PAGE,
                protection: 3,
                physical: None,
                bytes: vec![0; PAGE as usize],
            }],
            physical: vec![],
            refuse: false,
            effects: 0,
        }
    }
    fn region(&self, address: u64, len: usize, writing: bool) -> Result<&Region, GuestError> {
        let end = address
            .checked_add(len as u64)
            .ok_or(GuestError::AddressOverflow)?;
        let region = self
            .regions
            .iter()
            .find(|r| r.start <= address && end <= r.start + r.length)
            .ok_or(GuestError::Unmapped)?;
        if region.protection & if writing { 2 } else { 1 } == 0 {
            return Err(if writing {
                GuestError::WriteDenied
            } else {
                GuestError::ReadDenied
            });
        }
        Ok(region)
    }
    pub(crate) fn put(&mut self, offset: u64, bytes: &[u8]) {
        self.apply(
            MemoryEffect::None,
            &[GuestWrite {
                address: ARG + offset,
                bytes,
            }],
        )
        .unwrap();
    }
    pub(crate) fn get(&self, address: u64, length: usize) -> Vec<u8> {
        let mut bytes = vec![0; length];
        self.read(address, &mut bytes).unwrap();
        bytes
    }
    pub(crate) fn u64(&self, offset: u64) -> u64 {
        u64::from_le_bytes(self.get(ARG + offset, 8).try_into().unwrap())
    }
    fn copy(&mut self, address: u64, bytes: &[u8]) {
        let index = self
            .regions
            .iter()
            .position(|r| r.start <= address && address + bytes.len() as u64 <= r.start + r.length)
            .unwrap();
        let offset = (address - self.regions[index].start) as usize;
        if let Some(physical) = self.regions[index].physical {
            let target = physical + offset as u64;
            let block = self
                .physical
                .iter_mut()
                .find(|(base, data)| {
                    *base <= target && target + bytes.len() as u64 <= *base + data.len() as u64
                })
                .unwrap();
            let offset = (target - block.0) as usize;
            block.1[offset..offset + bytes.len()].copy_from_slice(bytes);
        } else {
            self.regions[index].bytes[offset..offset + bytes.len()].copy_from_slice(bytes);
        }
    }
}

impl GuestAccess for TestGuest {
    fn preflight_heap_arena(&self, address: u64, length: u64) -> Result<(), GuestError> {
        let end = address
            .checked_add(length)
            .ok_or(GuestError::AddressOverflow)?;
        let mut cursor = address;
        while cursor < end {
            let region = self
                .regions
                .iter()
                .find(|region| region.start <= cursor && cursor < region.start + region.length)
                .ok_or(GuestError::Unmapped)?;
            if region.protection & 1 == 0 || region.protection & 4 != 0 {
                return Err(GuestError::ReadDenied);
            }
            if region.protection & 2 == 0 {
                return Err(GuestError::WriteDenied);
            }
            cursor = end.min(region.start + region.length);
        }
        Ok(())
    }
    fn preflight_distinct_backing(&self, address: u64, length: u64) -> Result<(), GuestError> {
        if length > 32 {
            return Err(GuestError::UnsupportedMapping);
        }
        let end = address
            .checked_add(length)
            .ok_or(GuestError::AddressOverflow)?;
        // Each retained region is one linear private or physical span. No
        // payload read or permission inference is needed for this proof.
        if self
            .regions
            .iter()
            .any(|region| region.start <= address && end <= region.start + region.length)
        {
            Ok(())
        } else {
            Err(GuestError::UnsupportedMapping)
        }
    }
    fn initialize_network_backend(&mut self) -> Result<bool, Failure> {
        self.network_calls += 1;
        Ok(self.network_backend_result)
    }
    fn read_after_u64_write_byte(
        &self,
        address: u64,
        destination: u64,
        value: u64,
    ) -> Result<u8, GuestError> {
        let region = self.region(address, 1, false)?;
        let target = self.region(destination, 8, true)?;
        let mut byte = self.get(address, 1)[0];
        for (index, value) in value.to_le_bytes().into_iter().enumerate() {
            let at = destination + index as u64;
            if address == at
                || match (region.physical, target.physical) {
                    (Some(a), Some(b)) => a + address - region.start == b + at - target.start,
                    _ => false,
                }
            {
                byte = value;
            }
        }
        Ok(byte)
    }
    fn read_after_writes(
        &self,
        address: u64,
        output: &mut [u8],
        writes: &[GuestWrite<'_>],
    ) -> Result<(), GuestError> {
        if output.is_empty() {
            return Ok(());
        }
        if output.len() > 128 {
            return Err(GuestError::UnsupportedMapping);
        }
        let source = self.region(address, output.len(), false)?;
        let mut covered = [false; 128];
        output.fill(0);
        for write in writes {
            if write.bytes.is_empty() {
                continue;
            }
            let target = self.region(write.address, write.bytes.len(), true)?;
            let end = write
                .address
                .checked_add(write.bytes.len() as u64)
                .ok_or(GuestError::AddressOverflow)?;
            for (index, byte) in output.iter_mut().enumerate() {
                let at = address
                    .checked_add(index as u64)
                    .ok_or(GuestError::AddressOverflow)?;
                if write.address <= at && at < end {
                    *byte = write.bytes[(at - write.address) as usize];
                    covered[index] = true;
                } else if let (Some(left), Some(right)) = (source.physical, target.physical) {
                    let coordinate = left + at - source.start;
                    let first = right + write.address - target.start;
                    if first <= coordinate && coordinate - first < write.bytes.len() as u64 {
                        return Err(GuestError::UnsupportedMapping);
                    }
                }
            }
        }
        for (index, byte) in output.iter_mut().enumerate() {
            if !covered[index] {
                *byte = self.get(address + index as u64, 1)[0];
            }
        }
        Ok(())
    }

    fn preflight_write(&self, address: u64, length: u64) -> Result<(), GuestError> {
        self.region(
            address,
            usize::try_from(length).map_err(|_| GuestError::AddressOverflow)?,
            true,
        )
        .map(|_| ())
    }

    fn preflight_pthread(&self) -> Result<(), Failure> {
        Ok(())
    }
    fn preflight_posix_errno(&self, address: Option<u64>) -> Result<(), Failure> {
        if let Some(address) = address {
            let region = self.region(address, 4, true)?;
            self.region(address, 4, false)?;
            if region.protection & 4 != 0 {
                return Err(Failure::Unsupported("POSIX errno must remain NX"));
            }
        }
        Ok(())
    }
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), GuestError> {
        if output.is_empty() {
            return Ok(());
        }
        let region = self.region(address, output.len(), false)?;
        let offset = (address - region.start) as usize;
        if let Some(physical) = region.physical {
            let target = physical + offset as u64;
            let block = self
                .physical
                .iter()
                .find(|(base, data)| {
                    *base <= target && target + output.len() as u64 <= *base + data.len() as u64
                })
                .ok_or(GuestError::Unmapped)?;
            let offset = (target - block.0) as usize;
            output.copy_from_slice(&block.1[offset..offset + output.len()]);
        } else {
            output.copy_from_slice(&region.bytes[offset..offset + output.len()]);
        }
        Ok(())
    }
    fn apply(&mut self, effect: MemoryEffect, writes: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        if self.refuse {
            return Err(GuestError::WriteDenied);
        }
        let prospective = match effect {
            MemoryEffect::MapZeroed {
                address,
                length,
                protection,
            }
            | MemoryEffect::MapDirect {
                address,
                length,
                protection,
                ..
            } => Some((address, length, protection)),
            _ => None,
        };
        if let Some((address, length, protection)) = prospective {
            let end = address
                .checked_add(length)
                .ok_or(GuestError::AddressOverflow)?;
            if self
                .regions
                .iter()
                .any(|r| r.start < end && address < r.start + r.length)
            {
                return Err(GuestError::HostFailure);
            }
            if protection & 0x30 != 0 {
                return Err(GuestError::UnsupportedMapping);
            }
        }
        match effect {
            MemoryEffect::AllocateDirect {
                physical, length, ..
            } => {
                if self.physical.iter().any(|(base, data)| {
                    *base < physical + length && physical < *base + data.len() as u64
                }) {
                    return Err(GuestError::HostFailure);
                }
            }
            MemoryEffect::ReleaseDirect { physical, length } => {
                // This finite raw fixture only models whole, unmapped physical
                // blocks. Partial/mapped release is checked on the real native
                // owner rather than duplicating its split/rollback algorithm.
                if self.regions.iter().any(|r| {
                    r.physical
                        .is_some_and(|p| p < physical + length && physical < p + r.length)
                }) {
                    return Err(GuestError::UnsupportedMapping);
                }
                if !self
                    .physical
                    .iter()
                    .any(|(base, data)| *base == physical && data.len() as u64 == length)
                {
                    return Err(GuestError::Unmapped);
                }
            }
            MemoryEffect::MapDirect {
                physical, length, ..
            } => {
                if !self.physical.iter().any(|(base, data)| {
                    *base <= physical && physical + length <= *base + data.len() as u64
                }) {
                    return Err(GuestError::Unmapped);
                }
            }
            MemoryEffect::Unmap { address, length }
            | MemoryEffect::Protect {
                address, length, ..
            } if !self
                .regions
                .iter()
                .any(|r| r.start == address && r.length == length) =>
            {
                return Err(GuestError::Unmapped);
            }
            _ => {}
        }
        for write in writes {
            if write.bytes.is_empty() {
                continue;
            }
            if self.region(write.address, write.bytes.len(), true).is_err()
                && !prospective.is_some_and(|(start, length, protection)| {
                    protection & 2 != 0
                        && start <= write.address
                        && write
                            .address
                            .checked_add(write.bytes.len() as u64)
                            .is_some_and(|end| end <= start + length)
                })
            {
                return Err(GuestError::WriteDenied);
            }
        }
        match effect {
            MemoryEffect::None => {}
            MemoryEffect::AllocateDirect {
                physical, length, ..
            } => self.physical.push((physical, vec![0; length as usize])),
            MemoryEffect::ReleaseDirect { physical, .. } => {
                self.physical.retain(|(base, _)| *base != physical);
            }
            MemoryEffect::MapZeroed {
                address,
                length,
                protection,
            } => self.regions.push(Region {
                start: address,
                length,
                protection,
                physical: None,
                bytes: vec![0; length as usize],
            }),
            MemoryEffect::MapDirect {
                address,
                length,
                protection,
                physical,
            } => self.regions.push(Region {
                start: address,
                length,
                protection,
                physical: Some(physical),
                bytes: vec![],
            }),
            MemoryEffect::Unmap { address, .. } => self.regions.retain(|r| r.start != address),
            MemoryEffect::Protect {
                address,
                protection,
                ..
            } => {
                self.regions
                    .iter_mut()
                    .find(|r| r.start == address)
                    .unwrap()
                    .protection = protection
            }
        }
        for write in writes {
            if !write.bytes.is_empty() {
                self.copy(write.address, write.bytes);
            }
        }
        self.effects += 1;
        Ok(())
    }
}

pub(crate) fn kernel(limits: Limits) -> Kernel {
    Kernel::new(
        limits,
        ProcessIdentity {
            process_id: 100,
            initial_thread_id: 7,
        },
        vec![],
    )
    .unwrap()
}
fn call(
    kernel: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    arguments: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..arguments.len()].copy_from_slice(arguments);
    kernel.invoke(service, CallArgs(args), guest)
}

#[test]
fn loader_queries_without_image_binding_remain_unsupported_and_keep_attempt_accounting() {
    let mut kernel = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(0, b"original-query\0");
    let initial_effects = guest.effects;
    for service in [
        Service::GetProcParam,
        Service::Dlsym,
        Service::LoadStartModule,
        Service::TlsGetAddr,
    ] {
        let args = if service == Service::Dlsym {
            &[1, ARG, ARG + 128][..]
        } else if service == Service::LoadStartModule {
            &[ARG][..]
        } else {
            &[]
        };
        assert!(matches!(
            call(&mut kernel, &mut guest, service, args),
            Err(Failure::Unsupported(_))
        ));
    }
    assert_eq!(kernel.usage().attempts, 4);
    assert_eq!(kernel.usage().bytes_written, 0);
    assert_eq!(
        kernel.usage().bytes_read,
        2 * b"original-query\0".len() as u64
    );
    assert_eq!(guest.effects, initial_effects);
}

#[test]
fn unsupported_module_flags_and_options_refuse_before_guest_path_or_output_access() {
    let mut kernel = kernel(Default::default());
    let mut guest = TestGuest::new();
    let initial = guest.effects;
    for arguments in [[0, 0, 0, 1, 0, ARG], [0, 0, 0, 0, ARG, ARG]] {
        assert!(matches!(
            call(
                &mut kernel,
                &mut guest,
                Service::LoadStartModule,
                &arguments
            ),
            Err(Failure::Unsupported(_))
        ));
    }
    assert_eq!(kernel.usage().attempts, 2);
    assert_eq!(kernel.usage().bytes_read, 0);
    assert_eq!(kernel.usage().bytes_written, 0);
    assert_eq!(guest.effects, initial);
}

#[test]
fn deferred_module_admission_retains_arguments_and_budget_without_publishing_output() {
    let mut kernel = kernel(Limits {
        max_attempts: 1,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    guest.put(0, b"/app0/original.sprx\0");
    let effects = guest.effects;
    let args = CallArgs([ARG, 17, ARG + 512, 1_u64 << 32, 0, ARG + 128]);
    let admitted = kernel.admit_module_load(args, &guest).unwrap();
    assert_eq!(admitted.path(), "/app0/original.sprx");
    assert_eq!(admitted.module_args(), 17);
    assert_eq!(admitted.argument_pointer(), ARG + 512);
    assert_eq!(admitted.result_pointer(), Some(ARG + 128));
    assert_eq!(admitted.raw_arguments(), args);
    assert_eq!(kernel.usage().attempts, 1);
    assert_eq!(kernel.usage().bytes_written, 4);
    assert_eq!(guest.effects, effects);
    assert!(matches!(
        kernel.admit_module_load(args, &guest),
        Err(Failure::Limit(_))
    ));
    assert_eq!(kernel.usage().attempts, 1);
    assert_eq!(guest.effects, effects);
}

#[test]
fn exact_scope_and_source_aliases_are_required() {
    assert_eq!(
        resolve("libkernel", (1, 1), "libkernel", 1, "rTXw65xmLIA"),
        Some(Service::AllocateDirect)
    );
    assert_eq!(
        resolve("libkernel", (1, 0), "libkernel", 1, "rTXw65xmLIA"),
        None
    );
    assert_eq!(
        resolve("other", (1, 1), "libkernel", 1, "rTXw65xmLIA"),
        None
    );
    assert_eq!(
        resolve("libkernel", (1, 1), "other", 1, "rTXw65xmLIA"),
        None
    );
    assert_eq!(Errno::Invalid.kernel_code(), -2147352554);
    assert_eq!(
        Service::registrations()
            .iter()
            .filter(|r| r.service == Service::Usleep)
            .count(),
        2
    );
}

#[test]
fn dlsym_null_arguments_use_the_observed_int32_domain_and_failed_output_stays_transactional() {
    let mut kernel = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(128, &u64::MAX.to_le_bytes());
    let fault = u64::from(Errno::Fault.kernel_code() as u32);
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Dlsym, &[1, 0, ARG + 128]),
        Ok(fault)
    );
    assert_eq!(guest.u64(128), 0);
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Dlsym, &[1, u64::MAX, 0]),
        Ok(fault)
    );
    guest.put(PAGE - 4, &[0x55; 4]);
    assert!(matches!(
        call(
            &mut kernel,
            &mut guest,
            Service::Dlsym,
            &[1, 0, ARG + PAGE - 4]
        ),
        Err(Failure::Guest(_))
    ));
    assert_eq!(guest.get(ARG + PAGE - 4, 4), [0x55; 4]);
    assert_eq!(kernel.usage().attempts, 3);
}

#[test]
fn direct_allocation_uses_sixth_argument_and_rolls_back_failed_output() {
    let mut kernel = kernel(Limits::default());
    let mut guest = TestGuest::new();
    guest.refuse = true;
    assert!(
        call(
            &mut kernel,
            &mut guest,
            Service::AllocateDirect,
            &[0, 0x10000, PAGE, PAGE, 3, ARG]
        )
        .is_err()
    );
    assert_eq!(kernel.usage().handles_live, 0);
    assert!(guest.physical.is_empty());
    guest.refuse = false;
    call(
        &mut kernel,
        &mut guest,
        Service::AllocateDirect,
        &[0, 0x10000, PAGE, PAGE, 3, ARG],
    )
    .unwrap();
    assert_eq!(guest.u64(0), 0);
    assert_eq!(kernel.usage().handles_live, 1);
}

#[test]
fn zero_alignment_preserves_the_source_physical_search_start() {
    for (start, alignment, expected) in [(1, 0, 1), (1, PAGE, PAGE), (PAGE + 1, 0, PAGE + 1)] {
        let mut kernel = kernel(Limits::default());
        let mut guest = TestGuest::new();
        call(
            &mut kernel,
            &mut guest,
            Service::AllocateDirect,
            &[start, 0x10000, PAGE, alignment, 3, ARG],
        )
        .unwrap();
        assert_eq!(guest.u64(0), expected);
        assert_eq!(guest.physical[0].0, expected);
        assert_eq!(guest.physical[0].1.len(), PAGE as usize);
    }
}

#[test]
fn unchecked_release_refuses_source_noop_without_inventing_an_errno() {
    let mut kernel = kernel(Limits::default());
    let mut guest = TestGuest::new();
    let effects = guest.effects;
    assert_eq!(
        call(&mut kernel, &mut guest, Service::ReleaseDirect, &[0, PAGE]),
        Err(Failure::Unsupported(
            "source unchecked-release no-op branch"
        ))
    );
    assert_eq!(guest.effects, effects);
    assert_eq!(kernel.usage().handles_live, 0);
    assert_eq!(
        call(&mut kernel, &mut guest, Service::ReleaseDirect, &[0, 0]),
        Ok(0)
    );
    assert_eq!(guest.effects, effects);
    assert_eq!(
        call(&mut kernel, &mut guest, Service::ReleaseDirect, &[1, 0]),
        Err(Failure::Errno(Errno::Invalid))
    );

    call(
        &mut kernel,
        &mut guest,
        Service::AllocateMainDirect,
        &[PAGE, PAGE, 3, ARG],
    )
    .unwrap();
    guest.refuse = true;
    assert_eq!(
        call(&mut kernel, &mut guest, Service::ReleaseDirect, &[0, PAGE]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(kernel.usage().handles_live, 1);
    assert_eq!(guest.physical.len(), 1);
    guest.refuse = false;
    assert_eq!(
        call(&mut kernel, &mut guest, Service::ReleaseDirect, &[0, PAGE]),
        Ok(0)
    );
    assert_eq!(kernel.usage().handles_live, 0);
    assert!(guest.physical.is_empty());
    assert_eq!(
        call(&mut kernel, &mut guest, Service::ReleaseDirect, &[0, PAGE]),
        Err(Failure::Unsupported(
            "source unchecked-release no-op branch"
        ))
    );
}

#[test]
fn direct_aliases_share_actual_backing_and_remapping_preserves_bytes() {
    let mut kernel = kernel(Limits::default());
    let mut guest = TestGuest::new();
    call(
        &mut kernel,
        &mut guest,
        Service::AllocateMainDirect,
        &[PAGE, PAGE, 3, ARG],
    )
    .unwrap();
    call(
        &mut kernel,
        &mut guest,
        Service::MapDirect,
        &[ARG + 8, PAGE, 3, 0, 0, PAGE],
    )
    .unwrap();
    call(
        &mut kernel,
        &mut guest,
        Service::MapDirect,
        &[ARG + 16, PAGE, 3, 0, 0, PAGE],
    )
    .unwrap();
    let first = guest.u64(8);
    let second = guest.u64(16);
    guest
        .apply(
            MemoryEffect::None,
            &[GuestWrite {
                address: first + 12,
                bytes: b"owned",
            }],
        )
        .unwrap();
    assert_eq!(guest.get(second + 12, 5), b"owned");
    guest.refuse = true;
    assert_eq!(
        call(&mut kernel, &mut guest, Service::ReleaseDirect, &[0, PAGE]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    guest.refuse = false;
    call(&mut kernel, &mut guest, Service::Munmap, &[first, PAGE]).unwrap();
    call(
        &mut kernel,
        &mut guest,
        Service::MapDirect,
        &[ARG + 24, PAGE, 3, 0, 0, PAGE],
    )
    .unwrap();
    assert_eq!(guest.get(guest.u64(24) + 12, 5), b"owned");
}

#[test]
fn mapping_protection_and_query_are_atomic_and_bound_to_owned_ranges() {
    let mut kernel = kernel(Limits::default());
    let mut guest = TestGuest::new();
    call(
        &mut kernel,
        &mut guest,
        Service::MapFlexible,
        &[ARG, PAGE, 3, 0],
    )
    .unwrap();
    let address = guest.u64(0);
    assert!(guest.get(address, PAGE as usize).iter().all(|v| *v == 0));
    guest.refuse = true;
    assert!(
        call(
            &mut kernel,
            &mut guest,
            Service::Mprotect,
            &[address, PAGE, 1]
        )
        .is_err()
    );
    guest.refuse = false;
    guest
        .apply(
            MemoryEffect::None,
            &[GuestWrite {
                address,
                bytes: b"rw",
            }],
        )
        .unwrap();
    call(
        &mut kernel,
        &mut guest,
        Service::Mprotect,
        &[address, PAGE, 1],
    )
    .unwrap();
    assert_eq!(
        guest.apply(
            MemoryEffect::None,
            &[GuestWrite {
                address,
                bytes: b"x"
            }]
        ),
        Err(GuestError::WriteDenied)
    );
    call(
        &mut kernel,
        &mut guest,
        Service::QueryProtection,
        &[address, ARG + 8, ARG + 16, ARG + 24],
    )
    .unwrap();
    assert_eq!(guest.u64(8), address);
    assert_eq!(guest.u64(16), address + PAGE);
    assert_eq!(guest.get(ARG + 24, 4), 1_u32.to_le_bytes());
    let before = guest.get(ARG + 8, 20);
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::QueryProtection,
            &[address, ARG + 8, ARG + 8, ARG + 24]
        ),
        Err(Failure::Unsupported("overlapping guest outputs"))
    );
    assert_eq!(guest.get(ARG + 8, 20), before);
}

#[test]
fn resource_byte_work_attempt_and_mapping_limits_precede_effects() {
    let mut guest = TestGuest::new();
    let baseline = guest.effects;
    let mut limited = kernel(Limits {
        max_handles: 0,
        ..Limits::default()
    });
    assert_eq!(
        call(
            &mut limited,
            &mut guest,
            Service::MapFlexible,
            &[ARG, PAGE, 3, 0]
        ),
        Err(Failure::Limit("handles"))
    );
    let mut limited = kernel(Limits {
        max_mapped_bytes: PAGE - 1,
        ..Limits::default()
    });
    assert_eq!(
        call(
            &mut limited,
            &mut guest,
            Service::MapFlexible,
            &[ARG, PAGE, 3, 0]
        ),
        Err(Failure::Limit("mapped bytes"))
    );
    let mut limited = kernel(Limits {
        max_work: 4,
        ..Limits::default()
    });
    assert!(matches!(
        call(
            &mut limited,
            &mut guest,
            Service::MapFlexible,
            &[ARG, PAGE, 3, 0]
        ),
        Err(Failure::Limit("work"))
    ));
    let mut limited = kernel(Limits {
        max_attempts: 0,
        ..Limits::default()
    });
    assert_eq!(
        call(&mut limited, &mut guest, Service::Getpid, &[]),
        Err(Failure::Limit("attempts"))
    );
    assert_eq!(guest.effects, baseline);
}

#[test]
fn initial_thread_has_real_source_prefix_without_starting_another_thread() {
    let mut kernel = kernel(Limits::default());
    let mut guest = TestGuest::new();
    let pointer = call(&mut kernel, &mut guest, Service::PthreadSelf, &[]).unwrap();
    let bytes = guest.get(pointer, 4096);
    assert_eq!(&bytes[..4], &7_i32.to_le_bytes());
    assert!(bytes[4..].iter().all(|b| *b == 0));
    assert_eq!(
        call(&mut kernel, &mut guest, Service::PthreadSelf, &[]),
        Ok(pointer)
    );
    assert_eq!(kernel.usage().handles_live, 1);
    assert!(matches!(
        call(&mut kernel, &mut guest, Service::Munmap, &[pointer, PAGE]),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(guest.get(pointer, 4), 7_i32.to_le_bytes());
    assert!(matches!(
        call(&mut kernel, &mut guest, Service::PthreadCreate, &[]),
        Err(Failure::Unsupported(_))
    ));
}

fn semaphore(kernel: &mut Kernel, guest: &mut TestGuest, initial: u64, max: u64) -> u64 {
    guest.put(64, b"original-sema\0");
    call(
        kernel,
        guest,
        Service::CreateSema,
        &[ARG, ARG + 64, 1, initial, max, 0],
    )
    .unwrap();
    guest.u64(0)
}
#[test]
fn semaphore_counts_timeout_distinction_cancel_and_stale_handles() {
    let mut kernel = kernel(Limits::default());
    let mut guest = TestGuest::new();
    let handle = semaphore(&mut kernel, &mut guest, 2, 3);
    call(&mut kernel, &mut guest, Service::PollSema, &[handle, 2]).unwrap();
    assert_eq!(
        call(&mut kernel, &mut guest, Service::PollSema, &[handle, 1]),
        Err(Failure::Errno(Errno::Busy))
    );
    guest.put(32, &0_u32.to_le_bytes());
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::WaitSema,
            &[handle, 1, ARG + 32]
        ),
        Err(Failure::Errno(Errno::TimedOut))
    );
    assert!(matches!(
        call(&mut kernel, &mut guest, Service::WaitSema, &[handle, 1, 0]),
        Err(Failure::Unsupported(_))
    ));
    call(&mut kernel, &mut guest, Service::SignalSema, &[handle, 3]).unwrap();
    assert_eq!(
        call(&mut kernel, &mut guest, Service::SignalSema, &[handle, 1]),
        Err(Failure::Errno(Errno::Invalid))
    );
    call(
        &mut kernel,
        &mut guest,
        Service::CancelSema,
        &[handle, u64::MAX, ARG + 40],
    )
    .unwrap();
    assert_eq!(guest.get(ARG + 40, 4), 0_i32.to_le_bytes());
    call(&mut kernel, &mut guest, Service::PollSema, &[handle, 2]).unwrap();
    call(&mut kernel, &mut guest, Service::DeleteSema, &[handle]).unwrap();
    assert_eq!(
        call(&mut kernel, &mut guest, Service::PollSema, &[handle, 1]),
        Err(Failure::Errno(Errno::NoSuchResource))
    );
}

#[test]
fn semaphore_failed_remaining_time_write_does_not_consume_count() {
    let mut kernel = kernel(Limits::default());
    let mut guest = TestGuest::new();
    let handle = semaphore(&mut kernel, &mut guest, 1, 1);
    guest.put(32, &12_u32.to_le_bytes());
    guest.refuse = true;
    assert!(
        call(
            &mut kernel,
            &mut guest,
            Service::WaitSema,
            &[handle, 1, ARG + 32]
        )
        .is_err()
    );
    guest.refuse = false;
    call(&mut kernel, &mut guest, Service::PollSema, &[handle, 1]).unwrap();
}

#[test]
fn event_flags_keep_mask_clear_modes_outputs_and_typed_handles() {
    let mut kernel = kernel(Limits::default());
    let mut guest = TestGuest::new();
    guest.put(64, b"original-flag\0");
    call(
        &mut kernel,
        &mut guest,
        Service::CreateEventFlag,
        &[ARG, ARG + 64, 0x21, 0b1111, 0],
    )
    .unwrap();
    let flag = guest.u64(0);
    assert_eq!(
        call(&mut kernel, &mut guest, Service::PollSema, &[flag, 1]),
        Err(Failure::Errno(Errno::NoSuchResource))
    );
    call(
        &mut kernel,
        &mut guest,
        Service::ClearEventFlag,
        &[flag, 0b1010],
    )
    .unwrap();
    guest.refuse = true;
    assert!(
        call(
            &mut kernel,
            &mut guest,
            Service::PollEventFlag,
            &[flag, 0b0010, 0x21, ARG + 16]
        )
        .is_err()
    );
    guest.refuse = false;
    call(
        &mut kernel,
        &mut guest,
        Service::PollEventFlag,
        &[flag, 0b0010, 0x21, ARG + 16],
    )
    .unwrap();
    assert_eq!(guest.u64(16), 0b1010);
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::PollEventFlag,
            &[flag, 0b0010, 1, ARG + 16]
        ),
        Err(Failure::Errno(Errno::Busy))
    );
    assert_eq!(guest.u64(16), 0b1000);
    call(
        &mut kernel,
        &mut guest,
        Service::PollEventFlag,
        &[flag, 0b1000, 0x12, ARG + 16],
    )
    .unwrap();
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::PollEventFlag,
            &[flag, 1, 1, ARG + 16]
        ),
        Err(Failure::Errno(Errno::Busy))
    );
    assert_eq!(guest.u64(16), 0);
}

#[test]
fn event_timeout_writes_current_pattern_and_zero_without_consumption() {
    let mut kernel = kernel(Limits::default());
    let mut guest = TestGuest::new();
    guest.put(64, b"flag\0");
    call(
        &mut kernel,
        &mut guest,
        Service::CreateEventFlag,
        &[ARG, ARG + 64, 0, 2, 0],
    )
    .unwrap();
    let flag = guest.u64(0);
    guest.put(32, &0_u32.to_le_bytes());
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::WaitEventFlag,
            &[flag, 1, 1, ARG + 16, ARG + 32]
        ),
        Err(Failure::Errno(Errno::TimedOut))
    );
    assert_eq!(guest.u64(16), 2);
    assert_eq!(guest.get(ARG + 32, 4), [0; 4]);
    call(&mut kernel, &mut guest, Service::SetEventFlag, &[flag, 1]).unwrap();
    call(
        &mut kernel,
        &mut guest,
        Service::WaitEventFlag,
        &[flag, 3, 1, ARG + 16, ARG + 32],
    )
    .unwrap();
    assert_eq!(guest.u64(16), 3);
}

#[test]
fn source_clocks_normalize_and_invalid_nanosleep_does_not_wait() {
    let mut kernel = kernel(Limits::default());
    let mut guest = TestGuest::new();
    call(&mut kernel, &mut guest, Service::ClockGettime, &[4, ARG]).unwrap();
    let before = guest.get(ARG, 16);
    call(&mut kernel, &mut guest, Service::ClockGettime, &[4, ARG]).unwrap();
    let after = guest.get(ARG, 16);
    let pair = |bytes: &[u8]| {
        (
            i64::from_le_bytes(bytes[..8].try_into().unwrap()),
            i64::from_le_bytes(bytes[8..].try_into().unwrap()),
        )
    };
    assert!(pair(&after) >= pair(&before));
    assert!((0..1_000_000_000).contains(&pair(&after).1));
    call(&mut kernel, &mut guest, Service::Gettimeofday, &[ARG]).unwrap();
    assert!((0..1_000_000).contains(&pair(&guest.get(ARG, 16)).1));
    assert!(matches!(
        call(&mut kernel, &mut guest, Service::ClockGettime, &[14, ARG]),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(
        call(&mut kernel, &mut guest, Service::ClockGettime, &[3, ARG]),
        Err(Failure::Errno(Errno::Invalid))
    );
    let mut request = [0; 16];
    request[8..].copy_from_slice(&1_000_000_000_i64.to_le_bytes());
    guest.put(64, &request);
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Nanosleep, &[ARG + 64, 0]),
        Err(Failure::Errno(Errno::Invalid))
    );
    guest.put(64, &[0; 16]);
    call(
        &mut kernel,
        &mut guest,
        Service::Nanosleep,
        &[ARG + 64, ARG + 96],
    )
    .unwrap();
    assert_eq!(guest.get(ARG + 96, 16), [0; 16]);
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Sleep, &[1]),
        Err(Failure::Limit("sleep duration"))
    );
}

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
struct OriginalFile {
    path: PathBuf,
    directory: PathBuf,
    base: PathBuf,
}
impl OriginalFile {
    fn new() -> Self {
        let base =
            std::env::temp_dir().join(format!("pentax-kernel-original-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let directory = base.join(format!(
            "kernel-fixture-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("original.txt");
        std::fs::write(&path, b"original-data-0123456789").unwrap();
        Self {
            path,
            directory,
            base,
        }
    }
    fn root(&self) -> ReadOnlyRoot {
        ReadOnlyRoot::new(
            "/original",
            vec![ReadOnlyFile::new("data.txt", File::open(&self.path).unwrap()).unwrap()],
        )
        .unwrap()
    }
}
impl Drop for OriginalFile {
    fn drop(&mut self) {
        // Verify the exact named private test parent before recursive cleanup.
        if self.directory.is_absolute()
            && self.base.is_absolute()
            && self.directory.parent() == Some(self.base.as_path())
            && self
                .directory
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("kernel-fixture-")
        {
            let _ = std::fs::remove_dir_all(&self.directory);
            let _ = std::fs::remove_dir(&self.base);
        }
    }
}

#[test]
fn original_read_only_capability_read_pread_seek_and_stat() {
    let original = OriginalFile::new();
    let mut kernel = Kernel::new(
        Limits::default(),
        ProcessIdentity {
            process_id: 100,
            initial_thread_id: 1,
        },
        vec![original.root()],
    )
    .unwrap();
    let mut guest = TestGuest::new();
    guest.put(64, b"/original/data.txt\0");
    let fd = call(&mut kernel, &mut guest, Service::Open, &[ARG + 64, 0, 0]).unwrap();
    assert_eq!(fd, 3);
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Read, &[fd, ARG + 128, 8]),
        Ok(8)
    );
    assert_eq!(guest.get(ARG + 128, 8), b"original");
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::Pread,
            &[fd, ARG + 128, 4, 14]
        ),
        Ok(4)
    );
    assert_eq!(guest.get(ARG + 128, 4), b"0123");
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Lseek, &[fd, 0, 1]),
        Ok(8)
    );
    call(&mut kernel, &mut guest, Service::Fstat, &[fd, ARG + 256]).unwrap();
    assert_eq!(guest.u64(256 + 72), 24);
    assert_eq!(guest.get(ARG + 256 + 88, 4), 512_u32.to_le_bytes());
    call(&mut kernel, &mut guest, Service::Close, &[fd]).unwrap();
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Read, &[fd, ARG, 1]),
        Err(Failure::Errno(Errno::BadFd))
    );
}

#[test]
fn failed_file_output_does_not_advance_logical_cursor_or_escape_roots() {
    let original = OriginalFile::new();
    let mut kernel = Kernel::new(
        Limits::default(),
        ProcessIdentity {
            process_id: 100,
            initial_thread_id: 1,
        },
        vec![original.root()],
    )
    .unwrap();
    let mut guest = TestGuest::new();
    guest.put(64, b"/original/data.txt\0");
    let fd = call(&mut kernel, &mut guest, Service::Open, &[ARG + 64, 0, 0]).unwrap();
    guest.refuse = true;
    assert!(call(&mut kernel, &mut guest, Service::Read, &[fd, ARG + 128, 8]).is_err());
    guest.refuse = false;
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Read, &[fd, ARG + 128, 8]),
        Ok(8)
    );
    assert_eq!(guest.get(ARG + 128, 8), b"original");
    for path in [
        b"/original/../data.txt\0".as_slice(),
        b"D:/arbitrary.txt\0",
        b"/other/data.txt\0",
    ] {
        guest.put(64, path);
        assert_eq!(
            call(&mut kernel, &mut guest, Service::Open, &[ARG + 64, 0, 0]),
            Err(Failure::Errno(Errno::NoEntry))
        );
    }
    guest.put(64, b"/original/data.txt\0");
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Open, &[ARG + 64, 1, 0]),
        Err(Failure::Errno(Errno::ReadOnly))
    );
}

#[test]
fn thread_destructor_registration_is_process_owned_and_never_executes_or_reads_the_pointer() {
    let mut first = kernel(Limits::default());
    let second = kernel(Limits::default());
    let mut guest = TestGuest::new();
    for target in [0, 0, 0x1234_5000] {
        first
            .invoke_library(
                Service::SetThreadDtors,
                CallArgs([target, 0, 0, 0, 0, 0]),
                &mut guest,
            )
            .unwrap();
        assert_eq!(
            first.thread_lifecycle_callbacks().destructor,
            (target != 0).then_some(target)
        );
    }
    for target in [0, 0x1234_5000, 0x5678_9000] {
        assert_eq!(
            first.invoke_library(
                Service::SetThreadDtors,
                CallArgs([target, 0, 0, 0, 0, 0]),
                &mut guest
            ),
            Err(Failure::Unsupported(
                "thread lifecycle callback replacement"
            ))
        );
        assert_eq!(
            first.thread_lifecycle_callbacks().destructor,
            Some(0x1234_5000)
        );
    }
    assert_eq!(second.thread_lifecycle_callbacks().destructor, None);
    assert_eq!(guest.effects, 0);
    assert_eq!(first.usage().bytes_read + first.usage().bytes_written, 0);
    assert!(matches!(
        first.invoke_library(Service::PthreadExit, CallArgs([0; 6]), &mut guest),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(
        first.thread_lifecycle_callbacks().destructor,
        Some(0x1234_5000)
    );
}

#[test]
fn thread_destructor_registration_retains_attempt_limits_before_state_changes() {
    let mut state = kernel(Limits {
        max_attempts: 0,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    assert_eq!(
        state.invoke(
            Service::SetThreadDtors,
            CallArgs([0x1234, 0, 0, 0, 0, 0]),
            &mut guest
        ),
        Err(Failure::Limit("attempts"))
    );
    assert_eq!(state.thread_lifecycle_callbacks().destructor, None);
    assert_eq!(guest.effects, 0);
}

#[test]
fn thread_atexit_callbacks_have_independent_process_owned_registration_slots() {
    let mut state = kernel(Limits::default());
    let mut guest = TestGuest::new();
    for service in [
        Service::SetThreadAtexitCount,
        Service::SetThreadAtexitReport,
    ] {
        state.invoke(service, CallArgs([0; 6]), &mut guest).unwrap();
    }
    for (service, pointer) in [
        (Service::SetThreadDtors, 0x1000),
        (Service::SetThreadAtexitCount, 0x2000),
        (Service::SetThreadAtexitReport, 0x3000),
    ] {
        state
            .invoke_library(service, CallArgs([pointer, 0, 0, 0, 0, 0]), &mut guest)
            .unwrap();
        assert!(matches!(
            state.invoke_library(service, CallArgs([0; 6]), &mut guest),
            Err(Failure::Unsupported(
                "thread lifecycle callback replacement"
            ))
        ));
    }
    let callbacks = state.thread_lifecycle_callbacks();
    assert_eq!(callbacks.destructor, Some(0x1000));
    assert_eq!(callbacks.atexit_count, Some(0x2000));
    assert_eq!(callbacks.atexit_report, Some(0x3000));
    let other = kernel(Limits::default()).thread_lifecycle_callbacks();
    assert!(
        other.destructor.is_none() && other.atexit_count.is_none() && other.atexit_report.is_none()
    );
    assert_eq!(guest.effects, 0);
}
