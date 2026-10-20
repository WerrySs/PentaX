// SPDX-License-Identifier: GPL-2.0-only
// Adapted from authorized KytyPS5-Performance@209dffe libPlayGo.cpp.
// Blob 94f0093bd0013d266ed2e32c877f11393ba84a9c; see NOTICE/provenance.
//! Local selected-package PlayGo profile, with source errors and checked I/O.

use crate::{CallArgs, Failure, GuestAccess, GuestError, GuestWrite, Service};
use crate::{budget::Budget, filesystem::FileState};

mod package;
#[cfg(test)]
mod tests;

const BAD_ARGUMENT: u64 = 0x80b2_0004;
const BAD_HANDLE: u64 = 0x80b2_0009;
const BAD_POINTER: u64 = 0x80b2_000a;
const BAD_SIZE: u64 = 0x80b2_000b;
const BAD_CHUNK: u64 = 0x80b2_000c;
const BAD_SPEED: u64 = 0x80b2_000d;
const NOT_SUPPORTED: u64 = 0x80b2_000e;
const BAD_LOCUS: u64 = 0x80b2_0010;
const BAD_OPTIONAL: u64 = 0x80b2_0024;

#[derive(Default)]
pub(crate) struct PlayGoState {
    // Fixed selected-package metadata, not a live handle or download resource.
    chunks: Option<u16>,
}

impl PlayGoState {
    fn ensure_chunks(&mut self, files: &FileState, budget: &mut Budget) -> Result<(), Failure> {
        if self.chunks.is_some() {
            return Ok(());
        }
        const MAXIMUM: usize = 16 * 1024 * 1024;
        if let Some(bytes) =
            files.application_metadata("/app0/sce_sys/playgo-chunk.dat", Some(12), 12, budget)?
            && let Some(count) = package::binary(&bytes)
        {
            self.chunks = Some(count);
            return Ok(());
        }
        if let Some(bytes) =
            files.application_metadata("/app0/playgo-chunkdefs.xml", None, MAXIMUM, budget)?
            && let Some(count) = package::xml(&bytes, budget)?
        {
            self.chunks = Some(count);
            return Ok(());
        }
        if let Some(bytes) =
            files.application_metadata("/app0/Data/chunkmanifest", None, MAXIMUM, budget)?
            && let Some(count) = package::manifest(&bytes, budget)?
        {
            self.chunks = Some(count);
        }
        Ok(())
    }

    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        files: &FileState,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        use Service::*;
        budget.work(1)?;
        let a = args.0;
        match service {
            PlayGoInitialize => {
                if a[0] == 0 || budget.u64(guest, a[0])? == 0 {
                    return Ok(BAD_POINTER);
                }
                // The work buffer itself and reserved logging field are not
                // accessed. The reference's optional fabricated-count hack is off.
                return Ok(if budget.u32(guest, at(a[0], 8)?)? < 2 * 1024 * 1024 {
                    BAD_SIZE
                } else {
                    0
                });
            }
            PlayGoTerminate => return Ok(0),
            PlayGoOpen => {
                if a[0] == 0 {
                    return Ok(BAD_POINTER);
                }
                if a[1] != 0 {
                    return Ok(BAD_ARGUMENT);
                }
                self.ensure_chunks(files, budget)?;
                if self.chunks.is_none() {
                    return Ok(NOT_SUPPORTED);
                }
                store(guest, budget, a[0], &1_i32.to_le_bytes())?;
                return Ok(0);
            }
            _ => {}
        }
        if a[0] as u32 as i32 != 1 {
            return Ok(BAD_HANDLE);
        }
        match service {
            PlayGoClose => Ok(0),
            PlayGoGetInstallSpeed => {
                if a[1] == 0 {
                    return Ok(BAD_POINTER);
                }
                store(guest, budget, a[1], &2_i32.to_le_bytes())?;
                Ok(0)
            }
            PlayGoSetInstallSpeed => Ok(if (0..=2).contains(&(a[1] as u32 as i32)) {
                0
            } else {
                BAD_SPEED
            }),
            PlayGoGetLocus => {
                if a[1] == 0 || a[3] == 0 {
                    return Ok(BAD_POINTER);
                }
                let count = a[2] as u32 as usize;
                if count == 0 {
                    return Ok(BAD_SIZE);
                }
                self.ensure_chunks(files, budget)?;
                admit(count, 2, budget)?;
                let mut loci = budget.buffer_held(count, 0)?;
                for index in 0..count {
                    let mut id = [0; 2];
                    let address = at(a[1], (index * 2) as u64)?;
                    if index == 0 {
                        budget.read(guest, address, &mut id)?;
                    } else {
                        // Previous output may alias later IDs through either
                        // virtual addresses or actual physical backing.
                        budget.read_staged(
                            guest,
                            address,
                            &mut id,
                            &[GuestWrite {
                                address: a[3],
                                bytes: &loci[..index],
                            }],
                        )?;
                    }
                    if !self.valid(u16::from_le_bytes(id)) {
                        if index != 0 {
                            store(guest, budget, a[3], &loci[..index])?;
                        }
                        return Ok(BAD_CHUNK);
                    }
                    loci[index] = 3;
                }
                store(guest, budget, a[3], &loci)?;
                Ok(0)
            }
            PlayGoGetChunkId | PlayGoGetInstallChunkId => {
                let count = a[2] as u32;
                if a[3] == 0 || (count != 0 && a[1] == 0) {
                    return Ok(BAD_POINTER);
                }
                self.ensure_chunks(files, budget)?;
                let entries = count.min(u32::from(self.chunks.unwrap_or(0)));
                admit(entries as usize, 2, budget)?;
                let mut list = budget.buffer_held(entries as usize * 2, 0)?;
                for (index, bytes) in list.chunks_exact_mut(2).enumerate() {
                    bytes.copy_from_slice(&(index as u16).to_le_bytes());
                }
                list_and_count(guest, budget, a[1], &list, a[3], entries)?;
                Ok(0)
            }
            PlayGoGetToDoList => {
                if a[3] == 0 || (a[2] as u32 != 0 && a[1] == 0) {
                    return Ok(BAD_POINTER);
                }
                // The source never accesses the supplied list payload.
                store(guest, budget, a[3], &0_u32.to_le_bytes())?;
                Ok(0)
            }
            PlayGoSetToDoList | PlayGoPrefetch | PlayGoGetEta | PlayGoGetProgress => {
                let has_output = matches!(service, PlayGoGetEta | PlayGoGetProgress);
                if a[1] == 0 || (has_output && a[3] == 0) {
                    return Ok(BAD_POINTER);
                }
                let count = a[2] as u32 as usize;
                if count == 0 {
                    return Ok(BAD_SIZE);
                }
                if service == PlayGoPrefetch && !locus(a[3] as u8) {
                    return Ok(BAD_LOCUS);
                }
                let stride = if service == PlayGoSetToDoList { 4 } else { 2 };
                admit(count, stride, budget)?;
                self.ensure_chunks(files, budget)?;
                for index in 0..count {
                    let address = at(a[1], (index * stride) as u64)?;
                    let mut id = [0; 2];
                    budget.read(guest, address, &mut id)?;
                    if !self.valid(u16::from_le_bytes(id)) {
                        return Ok(BAD_CHUNK);
                    }
                    if service == PlayGoSetToDoList {
                        let mut value = [0];
                        budget.read(guest, at(address, 2)?, &mut value)?;
                        if !locus(value[0]) {
                            return Ok(BAD_LOCUS);
                        }
                    }
                }
                if service == PlayGoGetEta {
                    store(guest, budget, a[3], &0_i64.to_le_bytes())?;
                } else if service == PlayGoGetProgress {
                    let bytes = (count as u64).to_le_bytes();
                    budget.write(
                        guest,
                        &[
                            GuestWrite {
                                address: at(a[3], 8)?,
                                bytes: &bytes,
                            },
                            GuestWrite {
                                address: a[3],
                                bytes: &bytes,
                            },
                        ],
                    )?;
                }
                Ok(0)
            }
            PlayGoGetLanguageMask => {
                if a[1] == 0 {
                    return Ok(BAD_POINTER);
                }
                store(guest, budget, a[1], &u64::MAX.to_le_bytes())?;
                Ok(0)
            }
            PlayGoGetOptionalChunk
            | PlayGoGetSupportedOptionalChunk
            | PlayGoPrefetchOptionalChunk => {
                if a[2] == 0 {
                    return Ok(BAD_POINTER);
                }
                let kind = a[1] as u32 as i32;
                if !matches!(kind, 0 | 1) {
                    return Ok(BAD_OPTIONAL);
                }
                if service != PlayGoPrefetchOptionalChunk {
                    let mask = if kind == 0 { u64::MAX } else { 0x1f };
                    store(guest, budget, a[2], &mask.to_le_bytes())?;
                }
                Ok(0)
            }
            _ => Err(Failure::Unsupported("PlayGo service")),
        }
    }

    fn valid(&self, id: u16) -> bool {
        self.chunks.is_some_and(|count| id < count)
    }
}

fn locus(value: u8) -> bool {
    matches!(value, 0 | 2 | 3)
}

fn at(address: u64, offset: u64) -> Result<u64, Failure> {
    address
        .checked_add(offset)
        .ok_or(GuestError::AddressOverflow.into())
}

fn admit(count: usize, stride: usize, budget: &mut Budget) -> Result<(), Failure> {
    let bytes = count
        .checked_mul(stride)
        .ok_or(Failure::Limit("PlayGo entries"))?;
    if bytes > budget.limits.max_io_bytes {
        return Err(Failure::Limit("PlayGo entries"));
    }
    budget.work(count as u64)
}

fn store(
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
    address: u64,
    bytes: &[u8],
) -> Result<(), Failure> {
    budget.write(guest, &[GuestWrite { address, bytes }])
}

fn list_and_count(
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
    address: u64,
    list: &[u8],
    count_address: u64,
    count: u32,
) -> Result<(), Failure> {
    let bytes = count.to_le_bytes();
    if list.is_empty() {
        return store(guest, budget, count_address, &bytes);
    }
    let end = at(address, list.len() as u64)?;
    let count_end = at(count_address, 4)?;
    let mut writes = [GuestWrite {
        address: 0,
        bytes: &[],
    }; 3];
    let mut length = 0;
    if address < count_end && count_address < end {
        // Coalesce virtual overlap so the source's final count wins. Preserve
        // list-before-count ordering for disjoint physical aliases as well.
        if address < count_address {
            writes[length] = GuestWrite {
                address,
                bytes: &list[..(count_address - address) as usize],
            };
            length += 1;
        }
        if count_end < end {
            writes[length] = GuestWrite {
                address: count_end,
                bytes: &list[(count_end - address) as usize..],
            };
            length += 1;
        }
    } else {
        writes[length] = GuestWrite {
            address,
            bytes: list,
        };
        length += 1;
    }
    writes[length] = GuestWrite {
        address: count_address,
        bytes: &bytes,
    };
    budget.write(guest, &writes[..=length])
}
