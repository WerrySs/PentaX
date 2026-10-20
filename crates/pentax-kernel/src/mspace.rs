// SPDX-License-Identifier: GPL-2.0-only
// Adapted from boykopovar/AnyPS5 Mspace.cpp at
// 68323bae7494bc4b5d19776c02b4be86f35536c9, blob
// 01e8de2a22cb1bfd13a5a2441dc96e3a0e3e28f3. See NOTICE/provenance.
// Modified 2026 for PentaX: bounded guest coordinates and atomic publication.

use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestError, GuestWrite, Service};

mod plan;
mod storage;
use plan::{Plan, rounded};

const PAGE: u64 = 0x4000;
const MAX_CHUNKS: usize = 65_536;
const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MspaceResult {
    Value(u64),
    Errno { value: u64, number: u32 },
}

fn invalid(value: u64) -> MspaceResult {
    MspaceResult::Errno { value, number: 22 }
}
fn exhausted(value: u64) -> MspaceResult {
    MspaceResult::Errno { value, number: 12 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Chunk {
    start: u64,
    end: u64,
    used: bool,
}

struct Arena {
    base: u64,
    end: u64,
    chunks: Vec<Chunk>,
    in_use: u64,
    peak: u64,
}

impl Arena {
    fn used(&self, pointer: u64) -> Option<usize> {
        self.chunks
            .binary_search_by_key(&pointer, |chunk| chunk.start)
            .ok()
            .filter(|index| self.chunks[*index].used)
    }
    fn contains_allocation(&self, start: u64, end: u64) -> bool {
        let index = self.chunks.partition_point(|chunk| chunk.start <= start);
        index != 0
            && self.chunks[index - 1].used
            && self.chunks[index - 1].start <= start
            && end <= self.chunks[index - 1].end
    }
}

#[derive(Default)]
pub(crate) struct MspaceState {
    arenas: Vec<Arena>,
    chunks: usize,
}

impl MspaceState {
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.arenas.len()
    }

    fn metadata(&self) -> Result<usize, Failure> {
        let base = self
            .arenas
            .capacity()
            .checked_mul(core::mem::size_of::<Arena>())
            .ok_or(Failure::Limit("Mspace metadata"))?;
        let total = self.arenas.iter().try_fold(base, |sum, arena| {
            sum.checked_add(storage::chunk_bytes(arena.chunks.capacity())?)
                .ok_or(Failure::Limit("Mspace metadata"))
        })?;
        storage::metadata(total, 0)?;
        Ok(total)
    }

    fn admit_plan(&self, index: usize, plan: &Plan) -> Result<usize, Failure> {
        let chunks = self
            .chunks
            .checked_sub(self.arenas[index].chunks.len())
            .and_then(|count| count.checked_add(plan.arena.chunks.len()))
            .filter(|count| *count <= MAX_CHUNKS)
            .ok_or(Failure::Limit("Mspace chunks"))?;
        Ok(chunks)
    }

    fn commit(&mut self, index: usize, plan: Plan, chunks: usize) {
        self.arenas[index] = plan.arena;
        self.chunks = chunks;
    }

    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<MspaceResult, Failure> {
        // Searches, ordered shifts, prospective splits and private plan moves.
        let records = self
            .chunks
            .checked_add(self.arenas.len())
            .ok_or(Failure::Limit("work"))?;
        budget.work((records as u64 + 1) * 16)?;
        let a = args.0;
        use Service::*;
        match service {
            PafMspaceCreate if a[4] != 0 => {
                Err(Failure::Unsupported("PAF Mspace creation options"))
            }
            MspaceCreate | PafMspaceCreate => self.create(a, guest, budget),
            MspaceDestroy | PafMspaceDestroy => {
                let Some(index) = self.arenas.iter().position(|arena| arena.base == a[0]) else {
                    return Ok(invalid(u64::from(u32::MAX)));
                };
                self.chunks -= self.arenas[index].chunks.len();
                self.arenas.remove(index);
                budget.removed_handle();
                Ok(MspaceResult::Value(0))
            }
            MspaceFree | PafMspaceFree if a[1] == 0 => Ok(MspaceResult::Value(0)),
            MspaceCalloc | PafMspaceCalloc if a[1].checked_mul(a[2]).is_none() => Ok(exhausted(0)),
            MspacePosixMemalign if a[1] == 0 || a[2] < 8 || !a[2].is_power_of_two() => {
                Ok(MspaceResult::Value(22))
            }
            MspaceMemalign | MspaceAlignedAlloc | PafMspaceMemalign if !a[1].is_power_of_two() => {
                Ok(invalid(0))
            }
            MspaceReallocalign | PafMspaceReallocalign if a[2] >> 63 != 0 => {
                Ok(MspaceResult::Value(0))
            }
            MspaceMallocStats
            | MspaceMallocStatsFast
            | PafMspaceMallocStats
            | PafMspaceMallocStatsFast => self.stats(a, guest, budget),
            MspaceMallocUsableSize => {
                if a[0] == 0 {
                    return Ok(MspaceResult::Value(0));
                }
                for arena in self.arenas.iter().rev() {
                    if a[0] < arena.base || a[0] >= arena.end {
                        continue;
                    }
                    if let Some(index) = arena.used(a[0]) {
                        let chunk = arena.chunks[index];
                        storage::prove(guest, chunk.start, chunk.end - chunk.start, budget)?;
                        return Ok(MspaceResult::Value(chunk.end - chunk.start));
                    }
                }
                Ok(invalid(0))
            }
            MspaceMalloc
            | PafMspaceMalloc
            | MspaceCalloc
            | PafMspaceCalloc
            | MspaceFree
            | PafMspaceFree
            | MspaceRealloc
            | PafMspaceRealloc
            | MspacePosixMemalign
            | MspaceMemalign
            | MspaceAlignedAlloc
            | PafMspaceMemalign
            | MspaceReallocalign
            | PafMspaceReallocalign => {
                let Some(index) = self.arenas.iter().position(|arena| arena.base == a[0]) else {
                    return Ok(invalid(if service == MspacePosixMemalign {
                        22
                    } else {
                        0
                    }));
                };
                match service {
                    MspaceFree | PafMspaceFree => self.free(index, a[1], guest, budget),
                    MspaceRealloc | PafMspaceRealloc => {
                        self.reallocate(index, a[1], a[2], None, guest, budget)
                    }
                    MspaceReallocalign | PafMspaceReallocalign => {
                        let alignment = a[2].max(32).next_power_of_two();
                        self.reallocate(index, a[1], a[3], Some(alignment), guest, budget)
                    }
                    _ => self.allocate(index, service, a, guest, budget),
                }
            }
            _ => Err(Failure::Unsupported("Mspace service")),
        }
    }

    fn create(
        &mut self,
        a: [u64; 6],
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<MspaceResult, Failure> {
        let (base, size) = (a[1], a[2]);
        let Some(end) = base.checked_add(size) else {
            return Ok(invalid(0));
        };
        if base == 0 || base & 15 != 0 || size < 96 || (a[3] as u32) & !1 != 0 {
            return Ok(invalid(0));
        }
        for arena in &self.arenas {
            if base < arena.end && arena.base < end && !arena.contains_allocation(base, end) {
                return Ok(invalid(0));
            }
        }
        budget.can_add_handle()?;
        if self.chunks == MAX_CHUNKS {
            return Err(Failure::Limit("Mspace chunks"));
        }
        storage::prove(guest, base, size, budget)?;
        let current = self.metadata()?;
        storage::metadata(current, core::mem::size_of::<Chunk>())?;
        let mut chunks = storage::reserve(1, 0, budget)?;
        let chunk_bytes = storage::chunk_bytes(chunks.capacity())?;
        storage::metadata(current, chunk_bytes)?;
        chunks.push(Chunk {
            start: base + 64,
            end: base + (size & !15),
            used: false,
        });
        let growth = if self.arenas.len() == self.arenas.capacity() {
            let old = self.arenas.capacity() * core::mem::size_of::<Arena>();
            let mut next = storage::reserve(self.arenas.len() + 1, old + chunk_bytes, budget)?;
            let added = next
                .capacity()
                .checked_mul(core::mem::size_of::<Arena>())
                .and_then(|bytes| bytes.checked_add(chunk_bytes))
                .ok_or(Failure::Limit("Mspace metadata"))?;
            storage::metadata(current, added)?;
            next.append(&mut self.arenas);
            Some(next)
        } else {
            None
        };
        // All reserves/proofs/charges precede publication; no guest bytes change.
        if let Some(next) = growth {
            self.arenas = next;
        }
        let index = self.arenas.partition_point(|arena| arena.base < base);
        self.arenas.insert(
            index,
            Arena {
                base,
                end,
                chunks,
                in_use: 0,
                peak: 0,
            },
        );
        self.chunks += 1;
        budget.added_handle();
        Ok(MspaceResult::Value(base))
    }

    fn free(
        &mut self,
        index: usize,
        pointer: u64,
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<MspaceResult, Failure> {
        let Some(chunk_index) = self.arenas[index].used(pointer) else {
            return Ok(invalid(0));
        };
        let chunk = self.arenas[index].chunks[chunk_index];
        storage::prove(guest, pointer, chunk.end - pointer, budget)?;
        let mut plan = Plan::new(&self.arenas[index], self.metadata()?, budget)?;
        plan.release(chunk_index);
        let count = self.admit_plan(index, &plan)?;
        self.commit(index, plan, count);
        Ok(MspaceResult::Value(0))
    }

    fn allocate(
        &mut self,
        index: usize,
        service: Service,
        a: [u64; 6],
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<MspaceResult, Failure> {
        use Service::*;
        let posix = service == MspacePosixMemalign;
        let zero = matches!(service, MspaceCalloc | PafMspaceCalloc);
        let size = if zero {
            a[1] * a[2]
        } else if posix {
            a[3]
        } else if matches!(
            service,
            MspaceMemalign | MspaceAlignedAlloc | PafMspaceMemalign
        ) {
            a[2]
        } else {
            a[1]
        };
        let alignment = if posix {
            a[2]
        } else if matches!(
            service,
            MspaceMemalign | MspaceAlignedAlloc | PafMspaceMemalign
        ) {
            a[1]
        } else {
            32
        };
        let Some(capacity) = rounded(size) else {
            return Ok(exhausted(if posix { 12 } else { 0 }));
        };
        if capacity > u64::MAX - alignment.max(32) {
            return Ok(exhausted(if posix { 12 } else { 0 }));
        }
        let mut plan = Plan::new(&self.arenas[index], self.metadata()?, budget)?;
        let Some(pointer) = plan.allocate(size, alignment) else {
            return Ok(exhausted(if posix { 12 } else { 0 }));
        };
        let count = self.admit_plan(index, &plan)?;
        storage::prove(guest, pointer, capacity, budget)?;
        if zero && size != 0 {
            let bytes = storage::buffer(size, plan.scratch, budget)?;
            budget.write(
                guest,
                &[GuestWrite {
                    address: pointer,
                    bytes: &bytes,
                }],
            )?;
        } else if posix {
            budget.write(
                guest,
                &[GuestWrite {
                    address: a[1],
                    bytes: &pointer.to_le_bytes(),
                }],
            )?;
        }
        self.commit(index, plan, count);
        Ok(MspaceResult::Value(if posix { 0 } else { pointer }))
    }

    fn reallocate(
        &mut self,
        index: usize,
        pointer: u64,
        size: u64,
        alignment: Option<u64>,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<MspaceResult, Failure> {
        let arena = &self.arenas[index];
        let old = if pointer == 0 {
            None
        } else {
            let Some(at) = arena.used(pointer) else {
                return Ok(invalid(0));
            };
            Some((at, arena.chunks[at]))
        };
        if let Some((at, chunk)) = old
            && size == 0
            && alignment.is_none()
        {
            storage::prove(guest, pointer, chunk.end - pointer, budget)?;
            let mut plan = Plan::new(arena, self.metadata()?, budget)?;
            plan.release(at);
            let count = self.admit_plan(index, &plan)?;
            self.commit(index, plan, count);
            return Ok(MspaceResult::Value(0));
        }
        let Some(needed) = rounded(size) else {
            return Ok(exhausted(0));
        };
        if let Some((_, chunk)) = old {
            storage::prove(guest, pointer, chunk.end - pointer, budget)?;
            if needed <= chunk.end - pointer
                && alignment.is_none_or(|alignment| pointer & (alignment - 1) == 0)
            {
                return Ok(MspaceResult::Value(pointer));
            }
        }
        let mut plan = Plan::new(arena, self.metadata()?, budget)?;
        if let Some((at, _)) = old
            && alignment.is_none()
            && plan.grow_adjacent(at, needed)
        {
            storage::prove(guest, pointer, needed, budget)?;
            let count = self.admit_plan(index, &plan)?;
            self.commit(index, plan, count);
            return Ok(MspaceResult::Value(pointer));
        }
        let Some(next) = plan.allocate(size, alignment.unwrap_or(32)) else {
            return Ok(exhausted(0));
        };
        storage::prove(guest, next, needed, budget)?;
        let count = if let Some((_, chunk)) = old {
            let at = plan
                .arena
                .used(pointer)
                .expect("old allocation remains live in private plan");
            // Record the source's temporary old+new peak before release.
            plan.release(at);
            let count = self.admit_plan(index, &plan)?;
            let length = (chunk.end - pointer).min(size);
            if length != 0 {
                let mut bytes = storage::buffer(length, plan.scratch, budget)?;
                budget.read(guest, pointer, &mut bytes)?;
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: next,
                        bytes: &bytes,
                    }],
                )?;
            }
            count
        } else {
            self.admit_plan(index, &plan)?
        };
        self.commit(index, plan, count);
        Ok(MspaceResult::Value(next))
    }

    fn stats(
        &self,
        a: [u64; 6],
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<MspaceResult, Failure> {
        if a[1] == 0 {
            return Ok(MspaceResult::Value(22));
        }
        let mut header = [0; 2];
        budget.read(guest, a[1], &mut header)?;
        if u16::from_le_bytes(header) < 40 {
            return Ok(MspaceResult::Value(22));
        }
        let Some(arena) = self.arenas.iter().find(|arena| arena.base == a[0]) else {
            return Ok(invalid(22));
        };
        let system = arena.end - arena.base;
        let mut bytes = [0; 32];
        for (output, value) in
            bytes
                .chunks_exact_mut(8)
                .zip([system, system, arena.peak, arena.in_use])
        {
            output.copy_from_slice(&value.to_le_bytes());
        }
        let address = a[1].checked_add(8).ok_or(GuestError::AddressOverflow)?;
        budget.write(
            guest,
            &[GuestWrite {
                address,
                bytes: &bytes,
            }],
        )?;
        Ok(MspaceResult::Value(0))
    }
}

#[cfg(test)]
mod tests;
