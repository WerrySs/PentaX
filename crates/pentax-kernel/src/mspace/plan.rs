// SPDX-License-Identifier: GPL-2.0-only
// Bounded ordered-chunk adaptation of AnyPS5 Mspace.cpp; see parent provenance.

use super::*;

type Candidate = ((u64, u64), usize, u64);

pub(super) struct Plan {
    pub arena: Arena,
    pub scratch: usize,
}

impl Plan {
    pub fn new(arena: &Arena, metadata: usize, budget: &mut Budget) -> Result<Self, Failure> {
        let old = storage::chunk_bytes(arena.chunks.capacity())?;
        let capacity = arena
            .chunks
            .len()
            .checked_add(2)
            .ok_or(Failure::Limit("Mspace chunks"))?;
        let prospective = storage::chunk_bytes(capacity)?;
        storage::metadata(metadata, prospective)?;
        let mut chunks = storage::reserve(capacity, old, budget)?;
        let actual = storage::chunk_bytes(chunks.capacity())?;
        storage::metadata(metadata, actual)?;
        let scratch = storage::scratch(old, actual, budget)?;
        budget.work(arena.chunks.len() as u64 + 1)?;
        chunks.extend_from_slice(&arena.chunks);
        Ok(Self {
            arena: Arena {
                base: arena.base,
                end: arena.end,
                chunks,
                in_use: arena.in_use,
                peak: arena.peak,
            },
            scratch,
        })
    }

    pub fn allocate(&mut self, size: u64, alignment: u64) -> Option<u64> {
        let alignment = alignment.max(32);
        let needed = rounded(size)?;
        if needed > u64::MAX - alignment {
            return None;
        }
        let mut best: Option<Candidate> = None;
        for (index, chunk) in self.arena.chunks.iter().enumerate() {
            if chunk.used || chunk.end - chunk.start < needed {
                continue;
            }
            let Some(aligned) = align(chunk.start, alignment) else {
                continue;
            };
            if aligned > chunk.end || chunk.end - aligned < needed {
                continue;
            }
            let key = (chunk.end - chunk.start, chunk.start);
            if best.is_none_or(|(previous, _, _)| key < previous) {
                best = Some((key, index, aligned));
            }
        }
        let (_, index, start) = best?;
        let prior = self.arena.chunks[index];
        let finish = start + needed;
        self.arena.chunks[index] = Chunk {
            start,
            end: finish,
            used: true,
        };
        let mut used = index;
        if prior.start < start {
            self.arena.chunks.insert(
                index,
                Chunk {
                    start: prior.start,
                    end: start,
                    used: false,
                },
            );
            used += 1;
        }
        if finish < prior.end {
            self.arena.chunks.insert(
                used + 1,
                Chunk {
                    start: finish,
                    end: prior.end,
                    used: false,
                },
            );
        }
        self.arena.in_use += needed;
        self.arena.peak = self.arena.peak.max(self.arena.in_use);
        Some(start)
    }

    pub fn release(&mut self, index: usize) {
        let old = self.arena.chunks[index];
        self.arena.in_use -= old.end - old.start;
        self.arena.chunks[index].used = false;
        let mut current = index;
        if index != 0 && !self.arena.chunks[index - 1].used {
            self.arena.chunks[index - 1].end = old.end;
            self.arena.chunks.remove(index);
            current -= 1;
        }
        if current + 1 < self.arena.chunks.len() && !self.arena.chunks[current + 1].used {
            self.arena.chunks[current].end = self.arena.chunks[current + 1].end;
            self.arena.chunks.remove(current + 1);
        }
    }

    pub fn grow_adjacent(&mut self, index: usize, needed: u64) -> bool {
        let old = self.arena.chunks[index];
        let Some(next) = self.arena.chunks.get(index + 1).copied() else {
            return false;
        };
        if next.used || next.start != old.end || next.end - old.start < needed {
            return false;
        }
        let finish = old.start + needed;
        self.arena.chunks[index].end = finish;
        if finish == next.end {
            self.arena.chunks.remove(index + 1);
        } else {
            self.arena.chunks[index + 1].start = finish;
        }
        self.arena.in_use += needed - (old.end - old.start);
        self.arena.peak = self.arena.peak.max(self.arena.in_use);
        true
    }
}

pub(super) fn rounded(size: u64) -> Option<u64> {
    size.max(1).checked_add(31).map(|size| size & !31)
}

fn align(address: u64, alignment: u64) -> Option<u64> {
    address
        .checked_add(alignment - 1)
        .map(|address| address & !(alignment - 1))
}
