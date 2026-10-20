// SPDX-License-Identifier: GPL-2.0-only
// Adapted from authorized KytyPS5-Performance@209dffe9032d72749257ce18680d506b891284de,
// src/libs/libNet.cpp:1596-1740, blobd540a4d0f62a9c0fa8393e83164717ae5e6dcbcb.
// Explicit local virtual full-package mode; default addon catalogue is empty.
// Supplied local metadata is not PSN authentication or proof of ownership.

use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestError, GuestWrite, Kernel, Service};

const PARAMETER: u64 = 0x817d_0002;
const NO_ENTITLEMENT: u64 = 0x817d_0007;
const FULL_PACKAGE: u32 = 3;
const RECORD_BYTES: usize = 28;
const MAX_RECORDS: usize = 4096;

/// One caller-supplied local addon record. Label bytes are opaque and compared
/// by the source's early-NUL bounded17-byte rule, without UTF8 conversion.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LocalAddcont {
    label: [u8; 17],
    package_type: u32,
    download_status: u32,
}
impl LocalAddcont {
    #[must_use]
    pub const fn new(label: [u8; 17], package_type: u32, download_status: u32) -> Self {
        Self {
            label,
            package_type,
            download_status,
        }
    }
    fn bytes(self) -> [u8; RECORD_BYTES] {
        let mut bytes = [0; RECORD_BYTES];
        bytes[..17].copy_from_slice(&self.label);
        bytes[20..24].copy_from_slice(&self.package_type.to_le_bytes());
        bytes[24..28].copy_from_slice(&self.download_status.to_le_bytes());
        bytes
    }
}
impl std::fmt::Debug for LocalAddcont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalAddcont").finish_non_exhaustive()
    }
}

/// Owned configuration for local emulation, preserving supplied record order.
/// Construction bounds actual retained capacity; Kernel configuration validates
/// duplicate equivalent labels and process limits before replacing live data.
/// Default is an empty catalogue with the explicit fixed local FULL SKU mode.
#[derive(Default)]
pub struct LocalEntitlements {
    records: Vec<LocalAddcont>,
}
impl LocalEntitlements {
    pub fn new(records: Vec<LocalAddcont>) -> Result<Self, Failure> {
        if records.capacity() > MAX_RECORDS {
            return Err(Failure::Limit("local entitlement metadata"));
        }
        Ok(Self { records })
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    fn metadata_bytes(&self) -> usize {
        self.records.capacity() * core::mem::size_of::<LocalAddcont>()
    }

    pub(crate) fn invoke(
        &self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        use Service::*;
        let a = args.0;
        match service {
            EntitlementAccessInitialize => {
                if a[0] == 0 || a[1] == 0 {
                    return Ok(PARAMETER);
                }
                // Nonnull InitParam is opaque source data and is never read.
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[1],
                        bytes: &[0; 32],
                    }],
                )?;
                Ok(0)
            }
            EntitlementAccessGetSkuFlag => {
                if a[0] == 0 {
                    return Ok(PARAMETER);
                }
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[0],
                        bytes: &FULL_PACKAGE.to_le_bytes(),
                    }],
                )?;
                Ok(0)
            }
            EntitlementAccessGetAddcontEntitlementInfoList => self.list(a, guest, budget),
            EntitlementAccessGetAddcontEntitlementInfo => self.single(a, guest, budget),
            EntitlementAccessGetEntitlementKey => {
                if a[1] == 0 || a[2] == 0 {
                    return Ok(PARAMETER);
                }
                // Exact no-key source fallback: no label/key read or write.
                Ok(NO_ENTITLEMENT)
            }
            _ => Err(Failure::Unsupported("entitlement service")),
        }
    }

    fn list(
        &self,
        a: [u64; 6],
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let count = a[2] as u32;
        if a[3] == 0 || (a[1] == 0 && count != 0) {
            return Ok(PARAMETER);
        }
        let hit = (self.records.len() as u32).to_le_bytes();
        if count == 0 {
            budget.write(
                guest,
                &[GuestWrite {
                    address: a[3],
                    bytes: &hit,
                }],
            )?;
            return Ok(0);
        }
        let length = usize::try_from(u64::from(count) * RECORD_BYTES as u64)
            .map_err(|_| Failure::Limit("entitlement output bytes"))?;
        let list_end = a[1]
            .checked_add(length as u64)
            .ok_or(GuestError::AddressOverflow)?;
        let hit_end = a[3].checked_add(4).ok_or(GuestError::AddressOverflow)?;
        let overlap = a[1] < hit_end && a[3] < list_end;
        let start = if overlap { a[1].min(a[3]) } else { a[1] };
        let end = if overlap {
            list_end.max(hit_end)
        } else {
            list_end
        };
        let length_staged =
            usize::try_from(end - start).map_err(|_| Failure::Limit("entitlement output bytes"))?;
        let output_bytes = length_staged
            .checked_add(if overlap { 0 } else { 4 })
            .ok_or(Failure::Limit("entitlement output bytes"))?;
        if output_bytes > budget.limits.max_io_bytes {
            return Err(Failure::Limit("entitlement output bytes"));
        }
        let mut bytes = budget.buffer_held(length_staged, self.metadata_bytes())?;
        let list_at = (a[1] - start) as usize;
        let hit_after_clear = if overlap {
            let hit_at = (a[3] - start) as usize;
            bytes[hit_at..hit_at + 4].copy_from_slice(&hit);
            // Source stores total, clears the full list, THEN rereads total.
            budget.work(length as u64)?;
            bytes[list_at..list_at + length].fill(0);
            u32::from_le_bytes(bytes[hit_at..hit_at + 4].try_into().expect("four bytes"))
        } else {
            self.records.len() as u32
        };
        let copied = count.min(hit_after_clear) as usize;
        if copied > self.records.len() {
            return Err(Failure::Unsupported("entitlement staged hit count"));
        }
        budget.work(copied as u64 * RECORD_BYTES as u64 + 1)?;
        for (index, record) in self.records[..copied].iter().enumerate() {
            let at = list_at + index * RECORD_BYTES;
            bytes[at..at + RECORD_BYTES].copy_from_slice(&record.bytes());
        }
        // Normalize virtual overlap into one final span. The actual native
        // transaction rejects distinct physical aliases, including self-aliases.
        let mut writes = [
            GuestWrite {
                address: start,
                bytes: &bytes,
            },
            GuestWrite {
                address: a[3],
                bytes: &hit,
            },
        ];
        if overlap {
            writes[1].bytes = &[];
        }
        budget.write(guest, &writes[..if overlap { 1 } else { 2 }])?;
        Ok(0)
    }

    fn single(
        &self,
        a: [u64; 6],
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if a[1] == 0 || a[2] == 0 {
            return Ok(PARAMETER);
        }
        a[2].checked_add(RECORD_BYTES as u64)
            .ok_or(GuestError::AddressOverflow)?;
        let scratch = self
            .metadata_bytes()
            .checked_add(62)
            .ok_or(Failure::Limit("scratch"))?;
        budget.scratch(scratch)?;
        budget.record_scratch(scratch);
        let mut bytes = [0; RECORD_BYTES];
        let mut cache = [None; 17];
        let mut result = NO_ENTITLEMENT;
        for record in &self.records {
            budget.work(17)?;
            let mut matched = true;
            for (index, expected) in record.label.iter().copied().enumerate() {
                let value = if let Some(value) = cache[index] {
                    value
                } else {
                    let address = a[1]
                        .checked_add(index as u64)
                        .ok_or(GuestError::AddressOverflow)?;
                    // Four zero-u64 stores cover exactly28 output bytes. Each
                    // query yields the original byte or0; AND composes those
                    // zero overlays exactly, including ordinary virtual overlap.
                    // Existing adapters explicitly refuse distinct physical aliases.
                    let mut value = 0xff;
                    for offset in [0, 8, 16, 20] {
                        budget.work(8 * 16 * 16)?;
                        budget.precharge_read(1)?;
                        value &= guest.read_after_u64_write_byte(address, a[2] + offset, 0)?;
                    }
                    cache[index] = Some(value);
                    value
                };
                if value != expected {
                    matched = false;
                    break;
                }
                if value == 0 {
                    break;
                }
            }
            if matched {
                bytes = record.bytes();
                result = 0;
                break;
            }
        }
        budget.write(
            guest,
            &[GuestWrite {
                address: a[2],
                bytes: &bytes,
            }],
        )?;
        Ok(result)
    }
}

fn same_label(left: &[u8; 17], right: &[u8; 17]) -> bool {
    for (left, right) in left.iter().zip(right) {
        if left != right {
            return false;
        }
        if *left == 0 {
            return true;
        }
    }
    true
}

impl Kernel {
    /// Install explicit local addon metadata before service admission. No PSN
    /// or purchase check occurs. Refusal preserves the existing owned catalogue;
    /// work charges remain consumed. The caller transfers no guest buffer.
    pub fn configure_local_entitlements(&mut self, next: LocalEntitlements) -> Result<(), Failure> {
        if self.budget.usage().attempts != 0 {
            return Err(Failure::Unsupported(
                "local entitlement configuration after service admission",
            ));
        }
        if next.records.capacity() > MAX_RECORDS {
            return Err(Failure::Limit("local entitlement metadata"));
        }
        let held = self
            .entitlements
            .metadata_bytes()
            .checked_add(next.metadata_bytes())
            .ok_or(Failure::Limit("scratch"))?;
        self.budget.scratch(held)?;
        self.budget.record_scratch(held);
        let count = next.records.len() as u64;
        self.budget
            .work(count * count.saturating_sub(1) / 2 * 17 + held as u64 + 1)?;
        for (index, record) in next.records.iter().enumerate() {
            if next.records[..index]
                .iter()
                .any(|prior| same_label(&prior.label, &record.label))
            {
                return Err(Failure::Unsupported("duplicate local entitlement label"));
            }
        }
        self.entitlements = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
