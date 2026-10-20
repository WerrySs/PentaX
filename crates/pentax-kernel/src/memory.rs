// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: checked whole-range atomic backing effects,
// owned allocation records and bounded resources; see NOTICE.

use crate::budget::{Budget, Limits};
use crate::{
    CallArgs, Errno, Failure, GuestAccess, GuestWrite, MemoryEffect, RuntimeDataOutputs, Service,
};

const PAGE: u64 = 0x4000;
/// Closed provenance for an actual ENOMEM mapping refusal. Contains no input
/// addresses or sizes and does not change the guest error contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryMapRefusal {
    PhysicalCoverage,
    VirtualWindow,
    FixedCollision,
}
impl MemoryMapRefusal {
    pub const fn label(self) -> &'static str {
        match self {
            Self::PhysicalCoverage => "physical-coverage",
            Self::VirtualWindow => "virtual-window",
            Self::FixedCollision => "fixed-collision",
        }
    }
}
mod pthread;
mod ranges;
mod release;
mod runtime_object;
pub(crate) use pthread::PthreadObjectKind;
pub(crate) use runtime_object::RuntimeObject;

struct Allocation {
    start: u64,
    length: u64,
}
struct Mapping {
    identity: u64,
    start: u64,
    length: u64,
    protection: u32,
    physical: Option<u64>,
    internal: bool,
    pthread: Option<PthreadObjectKind>,
    _name: Vec<u8>,
}

pub(crate) struct MemoryState {
    allocations: Vec<Allocation>,
    mappings: Vec<Mapping>,
    limits: Limits,
    next_mapping: u64,
    next_pthread_address: u64,
    pub(crate) last_map_refusal: Option<MemoryMapRefusal>,
}

fn aligned(value: u64, alignment: u64) -> Result<u64, Failure> {
    // Source GetAlignedPos leaves the physical search position unchanged when
    // alignment is zero. Virtual mapping callers pass at least PAGE.
    if alignment == 0 {
        return Ok(value);
    }
    let alignment = alignment.max(PAGE);
    value
        .checked_add(alignment - 1)
        .and_then(|v| (v / alignment).checked_mul(alignment))
        .ok_or(Errno::Invalid.into())
}

impl MemoryState {
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.allocations.len() + self.mappings.len()
    }

    pub(crate) fn retained_pthread_object_count(&self) -> usize {
        self.mappings
            .iter()
            .filter(|mapping| mapping.pthread.is_some())
            .count()
    }

    pub(crate) fn new(limits: Limits) -> Result<Self, Failure> {
        let mut allocations = Vec::new();
        let mut mappings = Vec::new();
        allocations
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        mappings
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        Ok(Self {
            allocations,
            mappings,
            limits,
            next_mapping: 1,
            next_pthread_address: limits.virtual_base,
            last_map_refusal: None,
        })
    }

    pub(crate) fn initial_thread(
        &mut self,
        id: u32,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        // Pinned pthread.cpp PthreadGuestData: LE i32 ID then 4092 reserved
        // zero bytes. Materialize its real guest prefix in an owned 16KiB
        // guest mapping; no host pthread tail, TCB or new execution is claimed.
        self.runtime_word(id, guest, budget)
    }

    /// One protected RW/NX runtime-owned page with a checked initial word.
    /// Thread prefixes and errno use independent pages and resource identities.
    pub(crate) fn runtime_word(
        &mut self,
        value: u32,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        budget.can_add_handle()?;
        budget.can_map(PAGE)?;
        let address = self.choose_address(0, PAGE, PAGE, 0, budget)?;
        budget.work(PAGE)?;
        let bytes = value.to_le_bytes();
        let next_mapping = self
            .next_mapping
            .checked_add(1)
            .ok_or(Failure::Limit("mapping identities"))?;
        budget.apply(
            guest,
            MemoryEffect::MapZeroed {
                address,
                length: PAGE,
                protection: 3,
            },
            &[GuestWrite {
                address,
                bytes: &bytes,
            }],
        )?;
        self.mappings.push(Mapping {
            identity: self.next_mapping,
            start: address,
            length: PAGE,
            protection: 3,
            physical: None,
            internal: true,
            pthread: None,
            _name: Vec::new(),
        });
        self.next_mapping = next_mapping;
        budget.added_handle();
        budget.mapped(PAGE);
        Ok(address)
    }

    pub(crate) fn runtime_data(
        &mut self,
        size: usize,
        held_scratch: usize,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        prepare: impl FnOnce(u64, &mut [u8]) -> Result<RuntimeDataOutputs, Failure>,
    ) -> Result<u64, Failure> {
        if size == 0 {
            return Err(Failure::Limit("empty runtime data"));
        }
        let length = (size as u64)
            .checked_add(PAGE - 1)
            .map(|n| n & !(PAGE - 1))
            .ok_or(Failure::Limit("runtime data size"))?;
        budget.can_add_handle()?;
        budget.can_map(length)?;
        // Existing range revisions retain max_handles+2 slots; physical
        // releases can retain twice the admitted mapping count. Bound actual
        // capacity and prove the post-publication push cannot allocate.
        let maximum_capacity = self
            .limits
            .max_handles
            .checked_mul(2)
            .zip(self.limits.max_handles.checked_add(2))
            .map(|(release, ranges)| release.max(ranges))
            .ok_or(Failure::Limit("mapping metadata"))?;
        if self.mappings.capacity() > maximum_capacity
            || self.mappings.len() >= self.mappings.capacity()
        {
            return Err(Failure::Limit("mapping metadata"));
        }
        self.mappings
            .capacity()
            .checked_mul(std::mem::size_of::<Mapping>())
            .ok_or(Failure::Limit("mapping metadata"))?;
        let address = self.choose_address(0, length, PAGE, 0, budget)?;
        let next_mapping = self
            .next_mapping
            .checked_add(1)
            .ok_or(Failure::Limit("mapping identities"))?;
        let minimum_scratch = held_scratch
            .checked_add(size)
            .ok_or(Failure::Limit("scratch"))?;
        budget.scratch(minimum_scratch)?;
        let mut bytes = budget.buffer(size)?;
        let combined_scratch = held_scratch
            .checked_add(bytes.capacity())
            .ok_or(Failure::Limit("scratch"))?;
        budget.scratch(combined_scratch)?;
        budget.record_scratch(combined_scratch);
        let outputs = prepare(address, &mut bytes)?;
        let mut writes = [GuestWrite {
            address: 0,
            bytes: &[],
        }; 4];
        let mut output_work = 0;
        for (destination, output) in writes.iter_mut().zip(outputs.writes()) {
            output_work += output.bytes.len() as u64 + 1; // At most three fixed 256-byte slots.
            *destination = output;
        }
        budget.work(output_work)?;
        writes[outputs.len()] = GuestWrite {
            address,
            bytes: &bytes,
        };
        budget.work(length)?;
        let mapping = Mapping {
            identity: self.next_mapping,
            start: address,
            length,
            protection: 3,
            physical: None,
            internal: true,
            pthread: None,
            _name: Vec::new(),
        };
        budget.apply(
            guest,
            MemoryEffect::MapZeroed {
                address,
                length,
                protection: 3,
            },
            &writes[..outputs.len() + 1],
        )?;
        // All capacity, identities and resource arithmetic passed above. No
        // allocation, callback or fallible lookup follows the actual effect.
        self.mappings.push(mapping);
        self.next_mapping = next_mapping;
        budget.added_handle();
        budget.mapped(length);
        Ok(address)
    }

    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        self.last_map_refusal = None;
        let a = args.0;
        match service {
            Service::DirectMemorySize => Ok(self.limits.direct_memory_bytes),
            Service::AllocateDirect => self.allocate(
                a[0] as i64,
                a[1] as i64,
                a[2],
                a[3],
                a[4] as u32 as i32,
                a[5],
                guest,
                budget,
            ),
            Service::AllocateMainDirect => self.allocate(
                0,
                self.limits.direct_memory_bytes as i64,
                a[0],
                a[1],
                a[2] as u32 as i32,
                a[3],
                guest,
                budget,
            ),
            Service::ReleaseDirect => self.release(a[0] as i64, a[1], guest, budget),
            Service::MapFlexible | Service::MapNamedFlexible | Service::MapDirect => {
                self.map(service, a, None, guest, budget)
            }
            Service::Munmap => self.unmap(a[0], a[1], guest, budget),
            Service::Mprotect => self.protect(a[0], a[1], a[2] as u32, guest, budget),
            Service::QueryProtection => self.query(a, guest, budget),
            Service::SetVirtualRangeName => {
                let name = mapping_name(guest, budget, a[2])?;
                guest.set_virtual_range_name(a[0], a[1], &name)?;
                Ok(0)
            }
            Service::ClearVirtualRangeName => {
                guest.set_virtual_range_name(a[0], a[1], &[])?;
                Ok(0)
            }
            _ => Err(Failure::Unsupported("memory service")),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn allocate(
        &mut self,
        start: i64,
        end: i64,
        length: u64,
        alignment: u64,
        memory_type: i32,
        output: u64,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if start < 0
            || end <= start
            || length == 0
            || length & (PAGE - 1) != 0
            || alignment & (PAGE - 1) != 0
            || output == 0
        {
            return Err(Errno::Invalid.into());
        }
        budget.can_add_handle()?;
        let end = (end as u64).min(self.limits.direct_memory_bytes);
        let mut candidate = aligned(start as u64, alignment)?;
        // Restarting after each collision avoids dependence on insertion order.
        // Precharge the full conservative quadratic bound before scanning.
        let count = self.allocations.len() as u64;
        budget.work(
            count
                .checked_add(1)
                .and_then(|v| v.checked_mul(v))
                .ok_or(Failure::Limit("allocation search"))?,
        )?;
        loop {
            let candidate_end = candidate.checked_add(length).ok_or(Errno::Again)?;
            if candidate_end > end {
                return Err(Errno::Again.into());
            }
            if let Some(block) = self
                .allocations
                .iter()
                .find(|block| candidate < block.start + block.length && block.start < candidate_end)
            {
                candidate = aligned(block.start + block.length, alignment)?;
            } else {
                break;
            }
        }
        budget.work(length)?;
        let bytes = candidate.to_le_bytes();
        budget.apply(
            guest,
            MemoryEffect::AllocateDirect {
                physical: candidate,
                length,
                memory_type,
            },
            &[GuestWrite {
                address: output,
                bytes: &bytes,
            }],
        )?;
        self.allocations.push(Allocation {
            start: candidate,
            length,
        });
        budget.added_handle();
        Ok(0)
    }

    fn release(
        &mut self,
        start: i64,
        length: u64,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if start < 0 || (start as u64) & (PAGE - 1) != 0 || length & (PAGE - 1) != 0 {
            return Err(Errno::Invalid.into());
        }
        if length == 0 {
            return Ok(0);
        }
        self.release_range(start as u64, length, guest, budget)
    }

    fn choose_address(
        &mut self,
        hint: u64,
        length: u64,
        alignment: u64,
        flags: u32,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let fixed = flags & 0x10 != 0;
        let initial_end = self.limits.virtual_base + self.limits.virtual_bytes;
        let window_end = if fixed {
            initial_end
        } else {
            self.limits.nonfixed_virtual_end.unwrap_or(initial_end)
        };
        let mut candidate = if fixed {
            hint
        } else {
            aligned(hint.max(self.limits.virtual_base), alignment)?
        };
        if candidate == 0 || candidate & (PAGE - 1) != 0 || candidate % alignment.max(PAGE) != 0 {
            return Err(Errno::Invalid.into());
        }
        let count = self.mappings.len() as u64;
        budget.work(
            count
                .checked_add(1)
                .and_then(|v| v.checked_mul(v))
                .ok_or(Failure::Limit("mapping search"))?,
        )?;
        loop {
            let end = candidate.checked_add(length).ok_or(Errno::Invalid)?;
            if candidate < self.limits.virtual_base || end > window_end {
                self.last_map_refusal = Some(MemoryMapRefusal::VirtualWindow);
                return Err(Errno::NoMemory.into());
            }
            if let Some(mapping) = self
                .mappings
                .iter()
                .find(|m| candidate < m.start + m.length && m.start < end)
            {
                if fixed {
                    return if flags & 0x80 != 0 {
                        self.last_map_refusal = Some(MemoryMapRefusal::FixedCollision);
                        Err(Errno::NoMemory.into())
                    } else {
                        Err(Failure::Unsupported(
                            "fixed replacement of an existing mapping",
                        ))
                    };
                }
                candidate = aligned(mapping.start + mapping.length, alignment)?;
            } else {
                return Ok(candidate);
            }
        }
    }

    pub(crate) fn map_named_direct(
        &mut self,
        args: CallArgs,
        name: u64,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        // Upstream validates the name before entering MapDirectMemory.
        self.last_map_refusal = None;
        let name = mapping_name(guest, budget, name)?;
        self.map(Service::MapNamedDirect, args.0, Some(name), guest, budget)
    }

    fn map(
        &mut self,
        service: Service,
        a: [u64; 6],
        supplied_name: Option<Vec<u8>>,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        self.last_map_refusal = None;
        let [output, length, protection, flags, extra, alignment] = a;
        if output == 0 {
            return Err(Errno::Fault.into());
        }
        if length == 0 || length & (PAGE - 1) != 0 {
            return Err(Errno::Invalid.into());
        }
        let protection = protection as u32;
        let flags = flags as u32;
        let direct = matches!(service, Service::MapDirect | Service::MapNamedDirect);
        if direct && protection & 4 != 0 {
            return Err(Errno::Access.into());
        }
        let access = mapping_access(protection)?;
        // Cache/GPU compatibility encodings have distinct upstream paths.
        // Preserve the known fixed/no-overwrite and flexible alignment profile;
        // do not silently ignore unported mapping behavior.
        if !direct && flags & !0xff40_8490 != 0 {
            return Err(Errno::Invalid.into());
        }
        let supported = if direct { 0x90 } else { 0xff40_0090 };
        if flags & !supported != 0 {
            return Err(Failure::Unsupported("mapping flag profile"));
        }
        let align = if direct {
            if (extra as i64) < 0
                || extra & (PAGE - 1) != 0
                || alignment != 0 && !alignment.is_power_of_two() && alignment & (PAGE - 1) != 0
            {
                return Err(Errno::Invalid.into());
            }
            alignment.max(PAGE)
        } else {
            let shift = flags >> 24;
            if shift != 0 && !(14..=31).contains(&shift) {
                return Err(Errno::Invalid.into());
            }
            if shift == 0 { PAGE } else { 1_u64 << shift }
        };
        let name = if let Some(name) = supplied_name {
            name
        } else if service == Service::MapNamedFlexible {
            mapping_name(guest, budget, extra)?
        } else {
            Vec::new()
        };
        if direct {
            budget.work(self.allocations.len() as u64)?;
            let end = extra.checked_add(length).ok_or(Errno::Invalid)?;
            let count = self.allocations.len() as u64 + 1;
            budget.work(
                count
                    .checked_mul(count)
                    .ok_or(Failure::Limit("physical span work"))?,
            )?;
            if !self.physical_coverage(extra, end) {
                self.last_map_refusal = Some(MemoryMapRefusal::PhysicalCoverage);
                return Err(Errno::NoMemory.into());
            }
        }
        budget.can_add_handle()?;
        budget.can_map(length)?;
        let hint = budget.u64(guest, output)?;
        let address = self.choose_address(hint, length, align, flags, budget)?;
        budget.work(length)?;
        let effect = if direct {
            MemoryEffect::MapDirect {
                address,
                length,
                protection: access,
                physical: extra,
            }
        } else {
            MemoryEffect::MapZeroed {
                address,
                length,
                protection: access,
            }
        };
        let bytes = address.to_le_bytes();
        let next_mapping = self
            .next_mapping
            .checked_add(1)
            .ok_or(Failure::Limit("mapping identities"))?;
        budget.apply(
            guest,
            effect,
            &[GuestWrite {
                address: output,
                bytes: &bytes,
            }],
        )?;
        self.mappings.push(Mapping {
            identity: self.next_mapping,
            start: address,
            length,
            protection,
            physical: direct.then_some(extra),
            internal: false,
            pthread: None,
            _name: name,
        });
        self.next_mapping = next_mapping;
        budget.added_handle();
        budget.mapped(length);
        Ok(0)
    }

    fn unmap(
        &mut self,
        address: u64,
        length: u64,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if length == 0
            || address & (PAGE - 1) != 0
            || length & (PAGE - 1) != 0
            || address.checked_add(length).is_none()
        {
            return Err(Errno::Invalid.into());
        }
        let (mappings, handles) = self.revised_mappings(address, length, None, budget)?;
        budget.work(length / PAGE)?;
        budget.apply(guest, MemoryEffect::Unmap { address, length }, &[])?;
        self.mappings = mappings;
        budget.replaced_handles(handles);
        budget.unmapped(length);
        Ok(0)
    }

    fn protect(
        &mut self,
        address: u64,
        length: u64,
        protection: u32,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        // The pinned ProtectMemory rounds the byte range outward to guest
        // pages; its zero rounded length succeeds before inspecting protection.
        let offset = address & (PAGE - 1);
        let length = length
            .checked_add(offset)
            .and_then(|value| value.checked_add(PAGE - 1))
            .ok_or(Failure::Unsupported(
                "source-aborting protection range overflow",
            ))?
            & !(PAGE - 1);
        let address = address - offset;
        if length == 0 {
            return Ok(0);
        }
        if address.checked_add(length).is_none() {
            return Err(Failure::Unsupported(
                "source-aborting protection range overflow",
            ));
        }
        let access = mapping_access(protection)?;
        let (imported, imported_bytes, next_mapping) =
            self.protection_imports(address, length, protection, guest, budget)?;
        let (mappings, handles) = self.revised_with_imports(
            address,
            length,
            Some(protection),
            &imported,
            imported.capacity() * std::mem::size_of::<Mapping>(),
            budget,
        )?;
        budget.work(length / PAGE)?;
        budget.apply(
            guest,
            MemoryEffect::Protect {
                address,
                length,
                protection: access,
            },
            &[],
        )?;
        self.mappings = mappings;
        self.next_mapping = next_mapping;
        budget.replaced_handles(handles);
        budget.mapped(imported_bytes);
        Ok(0)
    }

    fn query(
        &self,
        a: [u64; 6],
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if a[0] == 0 {
            return Err(Failure::Unsupported("source-aborting null memory query"));
        }
        budget.work(self.mappings.len() as u64)?;
        let mapping = self
            .mappings
            .iter()
            .find(|m| m.start <= a[0] && a[0] < m.start + m.length)
            .ok_or(Errno::Access)?;
        let (start, end) = guest.virtual_name_bounds(
            a[0],
            mapping.start,
            mapping.start + mapping.length,
            &mapping._name,
        )?;
        if start < mapping.start
            || start > a[0]
            || end <= a[0]
            || end > mapping.start + mapping.length
        {
            return Err(Failure::Unsupported("invalid virtual-name boundaries"));
        }
        let start = start.to_le_bytes();
        let end = end.to_le_bytes();
        let protection = mapping.protection.to_le_bytes();
        let possible = [
            (a[1], start.as_slice()),
            (a[2], end.as_slice()),
            (a[3], protection.as_slice()),
        ];
        let mut writes = [GuestWrite {
            address: 0,
            bytes: &[],
        }; 3];
        let mut used = 0;
        for (address, bytes) in possible {
            if address != 0 {
                writes[used] = GuestWrite { address, bytes };
                used += 1;
            }
        }
        budget.write(guest, &writes[..used])?;
        Ok(0)
    }
}

#[cfg(test)]
#[path = "memory/runtime_data_tests.rs"]
mod runtime_data_tests;

// Pinned DecodeMemoryProtection derives access only from these five bits, but
// retains the complete protection word for range queries. CPU write access is
// read/write on x86, matching the pinned host_write=>host_read rule. Explicit
// GPU-only access stays on PentaX's independent GPU path. Non-access metadata
// never grants rights; a nonzero word without any access bit is EINVAL.
fn mapping_access(protection: u32) -> Result<u32, Failure> {
    let access = protection & 0x37;
    if protection != 0 && access == 0 {
        Err(Errno::Invalid.into())
    } else {
        Ok(if access & 2 != 0 { access | 1 } else { access })
    }
}

// Mapping labels are bounded byte strings, not filesystem paths or UTF-8 text.
fn mapping_name(
    guest: &impl GuestAccess,
    budget: &mut Budget,
    address: u64,
) -> Result<Vec<u8>, Failure> {
    if address == 0 {
        return Err(Errno::Fault.into());
    }
    let mut bytes = budget.buffer(budget.limits.max_name_bytes.min(32))?;
    for index in 0..bytes.len() {
        let at = address
            .checked_add(index as u64)
            .ok_or(crate::GuestError::AddressOverflow)?;
        budget.read(guest, at, &mut bytes[index..index + 1])?;
        if bytes[index] == 0 {
            bytes.truncate(index);
            return Ok(bytes);
        }
    }
    if bytes.len() == 32 {
        Err(Errno::NameTooLong.into())
    } else {
        Err(Failure::Limit("name bytes"))
    }
}
