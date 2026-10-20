# Upstream implementation review

Reviewed on 2026-10-19. Revisions below identify the inspected source, not a
moving branch or a compatibility claim. Source inspection required no external
builds, game-library scans or changes to the user's other projects.

## Sources and useful contracts

| Source and revision | Inspected implementation | Application to PentaX |
| --- | --- | --- |
| [AnyPS5](https://github.com/boykopovar/AnyPS5/tree/3d4da7ad0110f17e05e14a6c3825e7f7b9d35100) | [DirectMemory.cpp](https://github.com/boykopovar/AnyPS5/blob/3d4da7ad0110f17e05e14a6c3825e7f7b9d35100/core/libs/prx/libkernel/DirectMemory/DirectMemory.cpp) retains shared physical backing and per-view offsets; Windows reserves pagefile sections, while Linux uses a sealed-size shared memory file and explicitly zeros reused pages. | Compare alias lifetime and zero-on-reuse semantics. The Windows `SEC_RESERVE` approach agrees with PentaX's existing deferred-section path. Its relinker/library architecture is not a replacement for PentaX's checked Rust runtime. |
| [Original Kyty](https://github.com/InoriRus/Kyty/tree/4733b7e1c91b10554a52007903d74dc76c39a230) | [Memory.cpp](https://github.com/InoriRus/Kyty/blob/4733b7e1c91b10554a52007903d74dc76c39a230/source/emulator/src/Kernel/Memory.cpp) separates physical allocation records from virtual mappings, with whole-block release and one mapped range per block in the inspected implementation. | Useful historical kernel/ABI baseline. Do not regress PentaX's multiple aliases or partial release to that narrower implementation. |
| [KytyPS5-perf](https://github.com/WerrySs/KytyPS5-perf/tree/95012a103d233c903fd8d41014ae73c9d24819d7) | [memory.cpp](https://github.com/WerrySs/KytyPS5-perf/blob/95012a103d233c903fd8d41014ae73c9d24819d7/src/kernel/memory.cpp) validates pool-reserved state and protection before committing backing, and unwinds partial maps. [hostMemory.cpp](https://github.com/WerrySs/KytyPS5-perf/blob/95012a103d233c903fd8d41014ae73c9d24819d7/src/graphics/host_gpu/hostMemory.cpp) walks actual host regions to bound readable/mapped GPU inputs. | A current reference for pool contracts and rollback cases. Host accessibility alone must not replace PentaX owner/provenance and logical CPU/GPU permission checks. This private fork is distinct from the older `KytyPS5-Performance` source already attributed in PentaX. |
| [KytyPS5](https://github.com/KytyPS5/KytyPS5/tree/0cad4672e948b92b92e1aaf6e9bb5b144d8075ac) | [memory.cpp](https://github.com/KytyPS5/KytyPS5/blob/0cad4672e948b92b92e1aaf6e9bb5b144d8075ac/src/kernel/memory.cpp) validates the full expected range before replacement, splits range boundaries, updates page entries and merges compatible neighbors. [virtualMemory.h](https://github.com/KytyPS5/KytyPS5/blob/0cad4672e948b92b92e1aaf6e9bb5b144d8075ac/src/common/virtualMemory.h) separates reserve, commit, decommit and protection operations. | Compare later memory-pool and query behavior against a pinned implementation instead of guessing success values. Keep host mutation and guest metadata publication transactional. |
| [shadPS4](https://github.com/shadps4-emu/shadPS4/tree/f68923fe807f657348e6719ce658bcbd0a2b1c72) | [memory.cpp](https://github.com/shadps4-emu/shadPS4/blob/f68923fe807f657348e6719ce658bcbd0a2b1c72/src/core/memory.cpp) maintains physical/virtual areas and coordinates mappings with the Vulkan rasterizer. Its sparse copy routine resolves 16 KiB backing pages explicitly. | Reference for future GPU cache invalidation and sparse resource handling. PS4-specific region sizes, registers and shader assumptions need separate PS5 validation. Zero-filling a supported sparse GPU resource must not silently permit an invalid ordinary CPU read. |
| [SharpEmu](https://github.com/sharpemu/sharpemu/tree/b1056a449f3290154ae354be75f0a4ae0b9fd292) | [GuestPageTracker.cs](https://github.com/sharpemu/sharpemu/blob/b1056a449f3290154ae354be75f0a4ae0b9fd292/src/SharpEmu.HLE/GpuMemory/GuestPageTracker.cs) distinguishes CPU/GPU dirty state and uses summaries/epochs before detailed range queries. [WindowsHostMemory.cs](https://github.com/sharpemu/sharpemu/blob/b1056a449f3290154ae354be75f0a4ae0b9fd292/src/SharpEmu.HLE/Host/Windows/WindowsHostMemory.cs) exposes distinct reserve/commit/protect/query operations. | Candidate design for later incremental GPU uploads and avoiding repeated expensive host queries. Any fast path must preserve invalidation on aliases, unmap, permission changes and failed uploads. No measured PentaX speedup is claimed from inspection alone. |

## Implementation priorities

1. **Current startup constraint:** keep guest physical-pool capacity separate from
   Windows private pagefile commitment. PentaX's optional private sparse-file
   backend is an original Rust implementation from Microsoft API contracts;
   none of the six inspected implementations is copied into that backend.
   Validate actual shared bytes, zero reuse, resource admission and file lifetime
   before using it for a title probe.
2. **Next observed runtime stop:** use the relevant pinned KytyPS5 contract and
   exact library registration to implement the missing behavior. A source stub
   returning success is not evidence of a working service.
3. **After a working graphics path:** evaluate CPU/GPU dirty epochs and cache
   invalidation against actual upload/readback work. Measure before adopting a
   cache or fast path; preserve shared-backing semantics and failure recovery.

## Licensing and scope

- AnyPS5 and the inspected KytyPS5 forks declare GPL-2.0-only in their project
  documentation; Kyty's retained original MIT notice remains applicable.
- Original InoriRus/Kyty provides an MIT license.
- Inspected shadPS4 and SharpEmu source files carry GPL-2.0-or-later notices.
- These facts do not change existing file grants or dependency licenses.
  Any later adaptation must retain its actual source notices, record the pinned
  file/revision and describe modifications in
  [upstream provenance](upstream-provenance.md).

This review is an implementation map. It does not establish that a repository
boots a particular title, that all of its code can be transplanted to Rust, or
that combining projects automatically provides a complete PS5 emulator.
