# pentax-gpu

Bounded flat command inspection and separate compute/graphics preparation paths for PentaX.

The `inspect` API below remains a flat input-validation foundation. Its records
are not execution permissions. The separate `compute` path and native bridge are
described below; [graphics draw planning](src/graphics/README.md) additionally
prepares real guest nonindexed draws with actual compiled VS/PS and retained
linear RGBA8 target contents. Its finite state/resource profile and native
integration requirements are explicit; no title/frame result is implied.
Commercial-game execution remains unfinished. Flat inspection does **not**:

- keep GPU state, register reset values, predicate state, shader bindings or
  draw snapshots;
- apply register writes, derive shader program addresses or produce a
  command plan, executor sink or resume token;
- follow indirect buffers, read guest memory, call memory providers or
  traverse anything recursively;
- implement queues, a submission service, Vulkan or any renderer;
- interpret NOP payloads or implementation-defined NOP protocols;
- read files, perform I/O or depend on other crates.

## API

```rust,ignore
let report = pentax_gpu::inspect(&bytes, &pentax_gpu::Limits::default())?;
for packet in report.packets() {
    match packet.body() {
        pentax_gpu::PacketBody::RegisterWrite(write) => {
            let _ = (write.space(), write.offset(), write.value_range());
            let _ = write.values().collect::<Vec<u32>>();
        }
        pentax_gpu::PacketBody::Nop(payload) => { let _ = payload.bytes(); }
        pentax_gpu::PacketBody::Padding | pentax_gpu::PacketBody::HeaderOnlyNop => {}
    }
}
match report.outcome() {
    pentax_gpu::Outcome::CompleteFlatSubset => {}
    pentax_gpu::Outcome::Stopped { at, reason } => { let _ = (at, reason.category()); }
}
let _ = (report.committed_packets(), report.committed_dwords(), report.accounting());
```

`inspect` takes an explicitly supplied byte slice and `Limits`. The length
must be a multiple of four; dwords are little-endian and read by value, so
host alignment does not matter. The bytes are copied once into an owned,
immutable buffer inside the returned `Inspection`. Pre-ownership problems
(unaligned length, input limit, host-size conversion, allocation failure)
return a typed `InspectError` and no report.

`Inspection` has private fields and immutable accessors. It owns the input
and a vector of fixed-size records, one per committed packet. `Packet`,
`NopPayload` and `RegisterWrite` views borrow that report and expose
validated dword ranges into its own input; there is no constructor or API
combining a record with a different input. Payloads and register values are
never copied per packet, and no names or error strings are allocated.
`Debug` on the report prints a summary, not the input. `input()` exists for
private diagnosis.

A register record is an **encoded write intent**: space, relative offset,
original values and the raw header, including the shader-type and
filter-CAM-reset flags. It is not an applied write, a known register value or
a hardware-state snapshot.

The outcome is `CompleteFlatSubset` or `Stopped { at, reason }`, where `at`
is the dword index of the stopping header and equals `committed_dwords()`.
`StopReason::category()` separates `Unsupported` (outside this subset or
profile; not a claim that hardware rejects it), `Malformed`, `Truncated`,
`Resource` and `IndirectReference`. A stopped prefix is diagnostic only; it
cannot be resumed and is never executable state. A complete report is
syntax only and never permission to execute commands. Reinspection always
starts from a new full input, and two inspections share no state.

Header fields can also be read with the pure `Header::new(raw)` accessors.
Full packet decoding is only reachable through `inspect`, so every decoded
range belongs to the report that owns its bytes.

## Admitted subset

1. **Padding.** Exactly `0x80000000` is one dword of type-2 padding. Any
   other type-2 word stops as `UnsupportedPadding`; this is a subset choice,
   not a hardware-invalid diagnosis. Types 0 and 1 stop as
   `UnsupportedPacketType`.
2. **NOP (opcode 0x10).** The low header byte must be zero; otherwise
   `UnsupportedNopProfile`, before any framing. A zero-flag NOP with count
   `0x3fff` is a **header-only** one-dword packet; this is recognized before
   ordinary `count + 2` framing. Other zero-flag NOPs span `count + 2` dwords
   and keep their body as opaque bytes, with no marker or protocol
   interpretation.
3. **Direct register writes.** Context (0x69), SH (0x76) and uconfig (0x79)
   packets span `count + 2` dwords: header, one offset word, then `count`
   sequential values. In order: header bits 7:3 set stop as
   `UnsupportedHeaderEncoding` (raw header kept); a set predicate stops as
   `PredicatedWrite` with no intent recorded and nothing assumed about any
   inherited predicate; count zero stops as `EmptyRegisterWrite`. After
   framing, the offset word's high 16 bits must be zero, otherwise
   `UnsupportedRegisterSelector`; they are never masked. `offset + count`
   must not exceed `0x10000`, otherwise `RegisterRangeOverflow`. These are
   grammar rules over a 16-bit relative index domain, not claims about which
   PS5 registers exist. No register-bank sizes, selector normalization or
   special registers are applied.
4. **Indirect buffer (opcode 0x3f).** Only count 2 (four dwords: header,
   address low, address high, control) is decoded; any other count stops as
   `UnsupportedIndirectBufferLength` without reading presumed fields. A
   framed reference whose address low bits 1:0 are set stops as
   `MisalignedIndirectBuffer`. Otherwise inspection **always** stops with
   `IndirectBufferReference`, at the reference's header, whatever its size,
   chain bit or control word. The observation keeps raw address words, the
   raw 64-bit composition, size bits 19:0 (zero is not normalized), chain bit
   20 and opaque upper control bits 31:21, whose meaning differs between
   engines. The target is never read, there is no memory provider, and the
   reference contributes neither a record nor committed dwords.
5. **Everything else** stops as `UnsupportedOpcode`, including CLEAR_STATE
   of every command value, draws, dispatches, waits, synchronization, memory
   writes and predication. No reset or default state is inferred.

Header-level rules come first, so an unsupported opcode is reported as such
even when its framed body would be truncated. Then the input must hold the
framed length (`Truncated` otherwise), then body rules apply.

## Limits and accounting

| Field | Default (PentaX policy) | Checked |
| --- | --- | --- |
| `max_input_bytes` | 16 MiB | before the input copy, and against its reserved capacity |
| `max_records` | 262,144 | sizes the record reservation; before each commit |
| `max_record_bytes` | 16 MiB | sizes the record reservation, and against its reserved capacity |
| `max_visited_dwords` | 4,194,304 | header before classification, rest of the frame before body checks |

None of these is a hardware or driver limit; AMD's 20-bit IB size field is a
separate representation fact. All limits are inclusive, and zero or tiny
values are honoured.

The input copy is one `try_reserve_exact` of the input length. The record
vector is one `try_reserve_exact` of
`min(input dwords, max_records, max_record_bytes / RECORD_STORAGE_BYTES)`
slots, made before any packet is examined. Reserved capacities are measured
and checked against their byte limits, and commits stop before the slot
capacity is reached, so neither vector ever grows or reallocates. There are
no snapshots, payload copies or transient replacement allocations. Peak
owned heap storage is therefore exactly `Accounting::owned_capacity_bytes()`:
input capacity plus record capacity times `RECORD_STORAGE_BYTES` (a
host-dependent fixed record size). Allocator-internal bookkeeping and
rounding, and the fixed-size report struct itself, are outside this
accounting. A refused reservation returns `InspectError::AllocationFailed`.

Visited dwords include the stopping packet's charged dwords for diagnostics;
committed counts never do. A budget stop is a positioned `Resource` stop
that leaves the earlier prefix intact. All conversions and arithmetic are
checked, inspection is a single linear pass, and arbitrary bytes never
panic.

## Evidence

Field facts follow AMD's published definitions, pinned to exact revisions:

- A1 PAL PFP packets (`PM4_PFP_TYPE_3_HEADER`, indirect buffer, set context/uconfig):
  <https://github.com/GPUOpen-Drivers/pal/blob/c5e800072a32f68b6ccc4422936d96167c6e0728/src/core/hw/gfxip/gfx9/chip/gfx9_plus_merged_f32_pfp_pm4_packets.h>
- A2 PAL ME packets (type-3 header, CLEAR_STATE, set SH):
  <https://github.com/GPUOpen-Drivers/pal/blob/c5e800072a32f68b6ccc4422936d96167c6e0728/src/core/hw/gfxip/gfx9/chip/gfx9_plus_merged_f32_me_pm4_packets.h>
- A3 PAL packet writer (type-3 header, indirect buffer, NOP and sequential register builders):
  <https://github.com/GPUOpen-Drivers/pal/blob/c5e800072a32f68b6ccc4422936d96167c6e0728/src/core/hw/gfxip/gfx9/gfx9CmdUtil.cpp>
- A4 PAL opcode values:
  <https://github.com/GPUOpen-Drivers/pal/blob/c5e800072a32f68b6ccc4422936d96167c6e0728/src/core/hw/gfxip/gfx9/chip/gfx9_plus_merged_pm4_it_opcodes.h>
- A5 AMD-authored Linux packet type definitions, lines 28–50:
  <https://github.com/torvalds/linux/blob/038d61fd642278bab63ee8ef722c50d10ab01e8f/drivers/gpu/drm/amd/amdgpu/soc15d.h>
- A6 PAL MEC indirect buffer (engine-specific control fields):
  <https://github.com/GPUOpen-Drivers/pal/blob/c5e800072a32f68b6ccc4422936d96167c6e0728/src/core/hw/gfxip/gfx9/chip/gfx9_plus_merged_f32_mec_pm4_packets.h>
- A7 PAL maximum IB size representation:
  <https://github.com/GPUOpen-Drivers/pal/blob/c5e800072a32f68b6ccc4422936d96167c6e0728/src/core/hw/gfxip/gfx9/gfx9CmdUtil.h>

An independent public PS5 implementation was read only as observation that
the same subset appears in PS5 command streams:

- K1 <https://github.com/KytyPS5/KytyPS5/blob/763d3be3a9bc4e5a068d7cf52015030d6c03308f/src/graphics/guest_gpu/pm4.h>
- K2 <https://github.com/KytyPS5/KytyPS5/blob/763d3be3a9bc4e5a068d7cf52015030d6c03308f/src/graphics/guest_gpu/command_processor/pm4Handlers.cpp>
- K3 <https://github.com/KytyPS5/KytyPS5/blob/763d3be3a9bc4e5a068d7cf52015030d6c03308f/src/graphics/guest_gpu/graphicsRun.cpp>

These are AMD Gfx9+ layouts and an emulator's observations, not a Sony
specification. Agreement supports this named subset only, not complete PS5
packet acceptance. That implementation's register selector masking, register
bank sizes, custom NOP operations, payload markers, predicate handling and
indirect-buffer traversal are deliberately **not** reproduced. No code,
tables, identifiers or fixtures were copied; all tests use original
synthetic inputs.

## Deferred

The flat inspector does not provide indirect-buffer traversal (it needs explicit engine profiles, permitted
control semantics, checked guest reads, input snapshots and depth, hop and
cumulative-byte budgets), queue and submit-record parsing, register state,
predication, CLEAR_STATE semantics, shader interpretation and any execution
remain outside that API.

## Compute submission preparation

`compute::prepare` admits one complete, self-contained flat stream before any
resource read. Exact type-2 padding and opaque NOPs are allowed. Compute SET_SH_REG
updates only the named compute program, local/start dimensions, resource controls
and sixteen user SGPR slots. Every read register must have explicit known state;
no reset values or previous queue state are invented. Exactly one five-dword
DISPATCH_DIRECT is admitted with exact header lowbyte0 or2 and wave32 initiator
0x8001,0x8005 or the source builder's0x8041. Modes without FORCE_START_AT_000
require explicitly zero start coordinates. Unknown packets, flags, registers,
truncation, subsequent state writes or a second dispatch refuse the whole job.

The initial resource profile is one raw, unswizzled, stride-zero V# in s0..s3,
four-byte aligned and sized, with an explicit bounded readable guest span.
User SGPR count, local IDs and group IDs derive from observed compute controls.
Only the supported float state, no scratch/LDS/shared VGPRs and explicit resource
configuration are admitted. Wave64 and alternate descriptor/cache modes remain
typed unsupported. This is a PentaX profile, not a complete hardware validator.

Program addresses follow the pinned LO/HI assembly. Bounded word reads continue
until the real shader instruction/branch walker proves termination; a literal
equal to ENDPGM cannot delimit the program. Back edges refuse before GPU use,
bounding execution until loop instrumentation is implemented. Decode records,
program tables and compiler arenas receive separate finite caps, conservatively
reserved together with source storage, upload bytes and instruction/invocation
work before their allocations. The compiled manifest supplies every used SGPR
push value and the single zero-offset storage binding. Unknown live SGPR values,
undefined registers, unsupported compilation and resource layouts fail explicitly.

The opaque `PreparedCompute` owns its compiler product, upload and push block;
callers cannot inject raw SPIR-V or replace its fields. Debug exposes counts only.
`pentax-core::gpu::execute_compute` is the live-memory entry: it keeps one native
owner exclusively paused through preparation, self-alias/permission admission,
Vulkan submission, fence completion and transactional writeback. Repeated physical
bytes within a resource are refused before GPU use; general GPU alias/coherence
tracking remains unfinished. Outputs preserve bytes the shader did not write.

`pentax-worker gpu-fixture pm4-compute` creates only original command/shader/data
pages and verifies two actual Vulkan dispatches with different local sizes and
SGPR parameters. It checks all 112 output bytes, including an untouched suffix,
and unchanged command/shader bytes. This is a compute bridge, not AGC driver
submit ABI handling, a persistent queue, draw/frame presentation or game boot.

The adapted compute paths and retained licensing are recorded in [NOTICE](NOTICE).
The wave32 initiator bit is also described in LLVM's primary
[AMDGPU metadata test](https://github.com/llvm/llvm-project/blob/main/llvm/test/CodeGen/AMDGPU/mixed_wave32_wave64.ll).

## Stateful compute command plans

The separate `compute::plan_commands` admits an entire stream from checked guest
reads and an immutable `ComputeState`. The state has private known/unknown slots
for the supported compute registers; unknown reset values remain unknown. It
returns an opaque `CommandPlan` with ordered `Dispatch` snapshots, final register
state and every captured command span. Register-only streams, changes between
dispatches and trailing register updates are supported. `prepare_dispatch`
compiles one private snapshot using resource bytes read at that later phase, so
earlier fenced GPU results can feed subsequent resource reads.

An iterative owned-frame walker follows ordinary four-dword INDIRECT_BUFFER
packets. Calls retain the parent cursor; bit-20 chains replace the current frame
and discard its remaining tail. Nonzero buffers admit the pinned control profile
`0x0f200000` plus size/chain bits. Zero sizes do not read their target. Predication,
conditional indirect buffers and other control profiles remain unsupported.
Exact active lineage cycles refuse, including cycles through replaced chain
frames; completed targets can be called again. Depth, indirect count, cumulative
captured bytes, packets, dispatch records, actual allocation capacities and work
are all bounded before the corresponding read/walk/allocation. No recursive Rust
calls or ambient memory access are used. A late unsupported packet returns no
plan and publishes neither state nor GPU work.

Core retains independent states for the 56 observed ACB queue IDs. A successful
buffer commits its final known registers, including state-only updates. Resources
are prepared and executed in snapshot order under one exclusive pause. The
driver becomes terminal after a failure once GPU work may have happened; it
cannot replay a partially executed stream with old register state. GPU writes
that overlap captured command backing, including physical aliases, refuse before
submission until command-memory coherence is implemented. General asynchronous
queues, caches, broader resources, draws and presentation remain unfinished.

Ordered command views also admit source-observed memory WRITE_DATA destinations,
including write-one-address's final word; fulfilled 32/64-bit WAIT_REG_MEM; and
32/64-bit and reference-clock labels through the pinned `0xc0061060` and native
opcode0x49 RELEASE_MEM encodings. Immutable `Release` actions preserve optional
labels, the low27-bit interrupt context and selectors0..4. Selectors1/2/4 notify
after successful retirement;0/3 do not. Data selectors0..3 cover no data, dword,
qword and the source100MHz clock. Null destinations perform no write. Source
INT_SEL4 suppresses label and clock on both processors; graphics INT_SEL1 also
suppresses its label, while ACB INT_SEL1 keeps the write before notification.
That private queue distinction is selected by the actual planner path, never
by a guest flag or a detached public execution token. Source
cache/barrier fields cannot leave pending work in this serialized executor:
all preceding dispatch fences and same-owner writebacks complete first. GDS,
unsupported destinations/selectors and wait-operation modes remain typed stops. Write
payloads are privately owned and action/payload capacities and byte totals are
charged before allocation. A complete syntax plan precedes all guest effects.

SET_BASE's compute indirect-arguments base persists per queue. DISPATCH_INDIRECT
supports the pinned absolute-address and base-plus-offset encodings. Its three
u32 dimensions are read at execution preparation, after preceding GPU or command
writes. An admitted zero dimension produces an empty prepared outcome without
shader/resource reads or a Vulkan call; its bounded read work remains charged.
Each real dispatch continues to use the existing checked compiler product.

The synchronous core executor commits command memory writes transactionally on
the same paused owner, after checking distinct backing and separation from all
captured commands. Prior GPU work has fenced and written back before a label or
wait is observed. An unsatisfied wait returns a distinct `Waiting` stop; it does
not spin, invent success or claim asynchronous queue suspension/resumption.
Any failure after possible GPU or guest-memory effects leaves the driver terminal.

The Gen5 tagged wait profile is admitted as a complete14/16-dword compound:
exact lowbyte4 UCONFIG prefix,32/64-bit memory wait and exact end tag. Metadata
selectors, size tags, normalized address identity and complete framing must
match; standalone/malformed tags are refused. No arbitrary lowbyte4 register
write becomes executable. Source cache-policy fields retain the same exclusive
owner and serialized writeback boundary. A zero-count DCB write retains a real
empty write action with no backing access; ACB construction keeps its distinct
source zero-count refusal. Exact PFP_SYNC_ME with zero payload becomes an ordered
Drain action, whose executor has already fenced and published preceding work.
It does not invent an asynchronous fence or successful unsatisfied wait.

The source dispatch builder's0x40 bit is ORDER_MODE=1, which permits waves to
launch out of order. The Vulkan compute profile permits this rather than
promising an ordered launch. Other modifier controls and wave64 remain typed
unsupported. AMD documents this bit in the primary
[CIK register reference, COMPUTE_DISPATCH_INITIATOR](https://www.amd.com/content/dam/amd/en/documents/radeon-tech-docs/programmer-references/CIK_3D_registers_v2.pdf).

Ordered register arrays
-----------------------

The flat command profile decodes the five-DWORD CX, SH and UC indirect-register
packets into bounded descriptors. The executor reads each current array of
offset/value pairs at its position after preceding memory and GPU effects.
Draw and dispatch products use the resulting register state. Zero-count arrays
perform no address access; unknown offsets or queue banks remain explicit
refusals. Generic SH packets with a zero shader-type bit can update the known
compute register bank, matching the public common opcode handler.

Immutable command storage, packet metadata, reusable array scratch and action
capacity are admitted before effects. Captured array ranges participate in
later physical write exclusions. A late failure preserves prior effects and
makes the driver terminal. New array/compute-IB combinations remain unsupported;
streams without arrays retain the existing static planners and IB behavior.
