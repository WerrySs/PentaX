# Guest graphics draw planning

`graphics::plan_commands` admits complete flat PM4 streams into an immutable
`CommandPlan`. Supported packets are type-2 padding, ordinary/header-only NOP,
graphics `SET_CONTEXT_REG`, `SET_SH_REG`, `SET_UCONFIG_REG`, `NUM_INSTANCES`,
`DRAW_INDEX_AUTO` with the exact auto-index initiator2, and the exact indexed
packets described below. Each draw retains its
own known register snapshot and real vertex/instance counts. State-only streams
are supported. Zero-count draws consume their command position without shader
reads or a render product. This flat API refuses indirect buffers; the separate
bounded capture API below expands ordinary IB calls/chains. Predicates,
use-opaque counts, indirect draws, unported barriers and unknown registers are refused. Selected
ordered memory operations are described below.
No reset register value is invented. Final state is published by a queue owner
only after execution; a failed plan returns no executable prefix.

The source-specific six-dword tagged NOP `0xc004105c` produces an ordered
`Action::Flip`, retaining its handle, buffer, mode and signed64-bit argument.
This is Kyty's R_FLIP convention, not a generic AMD NOP side effect. Only that
exact header is admitted. Other low-header flags and the separate flip/label/
interrupt marker payloads remain unsupported. Every flip requires a preceding
nonzero draw not already consumed by another flip; multiple Draw/Flip cycles
remain ordered. The plan retains private draw snapshots plus bounded action
indices, with at most twice the admitted draw count. A plan grants no fence,
surface ownership, frame readiness or presentation authority.

`prepare_draw` reads the actual ES/PS program addresses, validates terminated
instruction boundaries with the existing shader decoder, refuses backward
branches and compiles both guest programs through the existing Rust compiler.
It returns private `PreparedDraw` products, manifests, stage push data and a
snapshot of the actual target. `prepare_draw` retains its linear-only contract;
`prepare_draw_with_target` additionally binds a bounded pure layout decoder.
The decoder receives immutable captured bytes, exact validated geometry and
remaining host/work limits, with no guest-memory authority. The native core
uses the existing VideoOut pixel conversion. It never substitutes a host shader or writes
guest memory. A numeric plan or range grants no owner, mapping or execution
authority. Native execution must check live GPU access and aliases, retain
exclusive ownership, submit/fence the actual draw, and atomically publish its
completed readback. Queue failure/retry must not replay already retired draws.
`shader_ranges()` exposes only the two actual terminated program extents;
numeric target/source overlap is refused and native integration must additionally
exclude physical aliases of those spans and its captured command ranges.

The first profile is deliberately finite:

- Triangle lists, explicit `NUM_INSTANCES` and zero known
  `GE_INDX_OFFSET`; real counts are bounded, with no modulo truncation.
- Indexed triangles use exact `INDEX_TYPE=0xc0002a00` with value0/1 for
  little-endian U16/U32, and exact six-dword `DRAW_INDEX_2=0xc0042700` with
  payload `[max_count, address_low, address_high, count, 0]`. The high address
  field must fit the existing47-bit address profile, count must not exceed
  max_count, and the actual nonempty source is naturally aligned to2/4 bytes.
  Natural alignment is an explicit finite profile restriction. The final
  flags word is exactly0; the source's additional0x20 option remains refused.
  `GE_MULTI_PRIM_IB_RESET_EN` must be explicitly known0. Base vertex is0,
  primitive restart is disabled and indirect/index8 profiles remain unsupported.
  max_count bounds the element count, never the values stored in the index
  buffer. VertexIndex values greater than or equal to count remain valid.
  An admitted zero-count packet needs no index type or payload access.
- The source indexed-register packet0xc0017a00 has exact selector0x20000243
  and value0x400 plus U16/U32 and the two cache-policy bits. INDEX_BASE and
  INDEX_BUFFER_SIZE retain explicit state. DRAW_INDEX_OFFSET_2 computes the
  checked base+offset*index-width address and captures the exact element count.
  The pinned direct path does not truncate this count to INDEX_BUFFER_SIZE;
  packet maximum count and actual current memory/resource bounds remain active.
- Unmerged ES-to-vertex stage, GS program disabled, no geometry/tessellation,
  scratch or LDS. GS resource/user registers supply the source vertex profile.
  Source `VGT_SHADER_STAGES_EN` selects VS wave32/64; the explicitly known
  `SPI_PS_IN_CONTROL` wave bit selects PS wave32/64.
- Actual `RSRC2` user SGPR counts up to16 per stage. The compiler manifest
  selects exactly the known SGPR values used; absent values are refused.
  VS and PS push placement is respectively dwords0 and16 of128 bytes,
  preserving guest register numbering. Unused bytes are initialized.
- A single standard-channel RGBA8_UNORM target: CB format10, type0,
  `SPI_SHADER_COL_FORMAT=9` (32_ABGR), normal color mode and ROP3copy.
  `CB_COLOR0_INFO=0x8028` selects blend clamp and round-by-half (enum0);
  the2D `CB_COLOR0_ATTRIB3` resource-type field is1, not the descriptor's9.
  One2D level/layer/sample; no DCC, CMASK, FMASK, blend or depth/stencil.
  Target geometry comes from `CB_COLOR0_ATTRIB2`, base from low/ext registers.
- Linear source rows align to256 bytes. Width must be a multiple of64 pixels,
  making its guest stride equal to the tight RGBA8 upload/readback stride.
- One-level32bpp RenderTarget64KB is a separate admitted representation:
  exact `CB_COLOR0_ATTRIB3=0x0106c000` selects2D, zero depth and swizzle0x1b
  (`SW_64KB_R_X`). The base must be64KiB-aligned. Guest pitch and height align
  to128 pixels; the actual padded range is retained independently of the tight
  Vulkan input. Unknown mode/metadata/pipe flags, mip/layer/sample/compression
  profiles remain unsupported. Width must still be a multiple of64 as an
  explicit existing backend restriction, not a hardware tiling requirement.
  The viewport always uses the original visible dimensions.
- Known scale/offset viewport and screen/window/generic/viewport0 scissors
  must equal the complete target. DX clip space, enabled scale/offset transforms,
  depth0..1, normal clipping and no culling/discard are explicitly required.
- VS exports position and supported PARAM locations0..31; PS exports floating
  MRT0 and may consume the linked parameters and supported system inputs.
  VertexIndex/InstanceIndex interfaces are allowed only when their manifest
  needs no unsupported DrawParameters.
  Textures, formatted vertex buffers, shader-data buffers, subgroups and
  undefined registers remain typed refusals.

The direct buffer slice admits an optional read-only raw dword V# in each VS
and PS. Live requirements are discovered from sealed optimized compiler IR;
register values are never searched heuristically for descriptor-shaped data.
Each source must be an actual four-aligned user quad within its declared user
count. The existing vertex entry overwrites s2/s3 with system values, so raw
vertex quad0 is refused; unmodified vertex quads4/8/12 and pixel quads0/4/8/12
remain eligible. Both scalar `S_BUFFER_LOAD_DWORD*` and raw
`BUFFER_LOAD_DWORD{x1..4}` reads retain the compiler's per-dword bounds checks.

This slice uses the existing compute raw shape: descriptor word1 contains only
base-address high16 bits, word2 is an exact nonzero DWORD-multiple byte count,
and word3 is zero. The base is nonzero and DWORD-aligned and the complete extent
must fit below the existing 47-bit address ceiling. Stride, swizzle, format,
add-TID and unknown fields are explicitly refused. Stores, coherent resources,
multiple resources in one stage, dynamic/SRT descriptors and shader-data-buffer
fallbacks remain unsupported. An access larger than the descriptor does not
authorize a larger snapshot; compiled OOB component reads return zero.

`PreparedDraw` retains independent immutable VS/PS snapshots at the exact guest
bases, with GPU-read permission admitted by `Memory::snapshot_buffer`. Actual
capacity and copy work share the existing host/work bounds. The target range
must not overlap a resource; the native adapter additionally checks physical
aliases before effects while preserving the unique pause. Resource/read aliases
are allowed and each retained snapshot is charged. These bytes do not become
pixel data or grant guest write authority. The backend uploads the exact bytes
and binds manifest storage buffers at set0/binding0 for VS and set0/binding57
for PS, retaining their owners through the actual fence.

Each stage's push footprint includes its actual user values plus the packed
buffer-prefix offset word. It must fit its original16-dword window: VS starts
at0 and PS at16. Exact-base uploads use explicit zero prefixes; no descriptor
alignment padding or synthesized user value is introduced. Existing compiler,
host, work, pixel-transport and device descriptor limits remain in force.
Original CPU and native consumers exercise this bounded contract; source
implementation and fixture declarations alone do not establish game rendering.

Parameter linkage requires explicit known `SPI_VS_OUT_CONFIG`,
`SPI_PS_IN_CONTROL`, equal `SPI_PS_INPUT_ENA/ADDR`, `SPI_BARYC_CNTL=0` and all
declared `SPI_PS_INPUT_CNTL_0..31` words. Export count is the span
`1+((VS_OUT_CONFIG>>1)&31)`, permitting sparse exports; other config bits are
refused. PS count is at most32, with only the wave bit additionally accepted.
Each ordinary input-control word is exactly location bits0..4 plus flat bit10.
Public AGC custom words set bit5 and are refused, as are F16/default fields.
Same-location input aliases are valid only with the same interpolation class.
Every actual consumed PARAM must have a unique matching VS location within
the declared export span; unused VS outputs are valid. Neither absent control
words nor missing producers receive a default value.

The supported smooth profile is center perspective (`ENA=ADDR` bit1), whose
genuine ABI i/j values occupy v0/v1. P1's ATTR/channel and partial-result
provenance are retained through SSA and checked against the same builtin i/j
pair in P2. Computed coordinates, mismatched partials and live unconsumed P1
are typed refusals; ordinary register copies preserve provenance. Flat MOV
mode2 follows the actual mapped flat input. Centroid/sample/linear/pull/stipple,
custom interpolation and raw barycentric operations remain unsupported by this
compiler path. No approximate interpolation or substituted shader is used.

ENA position bits8..11 and front-face bit12 follow the source's compact VGPR
order after the optional center pair. PositionW is reciprocal FragCoord.w;
front-face is float+1/-1. The all-bits front-face mode remains refused. Fixed
`PA_SU_SC_MODE_CNTL=0` retains first-vertex flat interpolation. The input-profile
Vec's actual capacity is charged alongside temporary compiler envelopes and
released before target capture. Native Vulkan admission independently checks
actual unique emitted locations and components, including position/FragCoord/
FrontFacing builtins, against the physical device limits before image/buffer/
pipeline effects. Supporting32 source locations does not imply every device
can execute a32-parameter interface.

These constraints describe a real guest draw path toward rendering. They do
not establish that a supplied commercial title fits this profile or that a
frame has been rendered or presented. Original encoded PM4/VS/PS fixtures live
only in tests; production contains no diagnostic triangle program.

For indexed draws, preparation captures the complete checked count-times-width
payload with GPU-read access before any GPU effects. `PreparedIndices` owns
immutable bytes and retains the exact source range, type and count. Its actual
vector capacity and copy work are admitted alongside compiled programs, LOAD,
upload and readback, with no larger budget. The native core prepares afresh
under one exclusive paused owner, rechecks the complete live index span and
excludes physical aliases with the writable target. Index, command and shader
inputs may share read-only backing. An old public prepared product cannot
manufacture the core's same-pause completion or guest writeback authority.
The backend validates U32 values against the actual enabled Vulkan feature
and reported device limit; it does not infer a value bound from count.

Default command bounds are64 streams,256KiB command bytes,65536 packets,
64 retained draws,1048576 total vertices including instances,4MiB charged host
storage and4194304 work units. Draw preparation independently bounds256 shader
words per stage,32768 SPIR-V words per stage,16MiB target bytes,4096 dimensions,
1048576 vertices,128MiB charged host storage and134217728 work units. Compiler
phases have independent8MiB/4194304 caps; conservative outer charges also cover
decode/compiler peaks and worst-case raster work. A decoder invocation admits
its record/source span and validation-table envelopes while that call is live;
each compilation admits its arena envelope alongside the actual retained
programs/products. The decoded programs are explicitly dropped before target
allocation. `Usage::host_bytes` reports actual retained vector capacities,
while `peak_host_bytes` includes admitted temporary envelopes. It does not add
dead compiler scratch to later target/readback storage. Actual vector capacities
are checked after reservation. No budget is an AMD hardware limit.

Pixel staging has an independent64MiB default ceiling. Tiled drawing retains
the full padded LOAD snapshot and tight decoded input, plus a bounded backend
upload/completed-readback allowance; linear drawing shares its LOAD/input vector.
The native driver subtracts actual captured Frames and consumer-held capacities
from that pixel ceiling. Its independent128MiB graphics workspace bound still
includes actual retained external allocations, command plans, compiled products
and the live phase's scratch. Compiler IR, executable caches and opaque driver
heaps are not represented as session pixel storage. Every ceiling must pass;
these are overlapping bounded accounting categories, not an exact process heap
measurement. Draw buffers die before the following EOP capture's separate peak.

The packet/register and shader-profile contracts adapt the retained authorized
KytyPS5-Performance source at `209dffe9032d72749257ce18680d506b891284de`:
`src/graphics/guest_gpu/pm4.h`,
`src/graphics/guest_gpu/command_processor/pm4Handlers.cpp`
(`CpOpDrawIndexAuto`, `CpOpNumInstances`, direct register setters),
`src/graphics/shader/shader.cpp` (`PrepareProgram`, shader initial registers),
`src/graphics/guest_gpu/gpu_defs.h`, `hardwareContext.h` and `tile.cpp`
(`CalcLinearBlockWidth`, `TileGetTexturePitch`). These extend the existing
compute planner's source attribution and source-normalized compiler interface.
AMD packet-header/type/count definitions are the existing pinned PAL contracts
linked in the parent GPU README; no shader bytes or graphics assets are copied.
Additional AMD-authored field facts use the same PAL pin
`c5e800072a32f68b6ccc4422936d96167c6e0728`:
[gfx9ColorTargetView.cpp](https://github.com/GPUOpen-Drivers/pal/blob/c5e800072a32f68b6ccc4422936d96167c6e0728/src/core/hw/gfxip/gfx9/gfx9ColorTargetView.cpp)
sets UNorm clamp/rounding and directly encodes the image type;
[palImage.h](https://github.com/GPUOpen-Drivers/pal/blob/c5e800072a32f68b6ccc4422936d96167c6e0728/inc/core/palImage.h)
defines Tex2d1; and
[gfx9_plus_merged_enum.h](https://github.com/GPUOpen-Drivers/pal/blob/c5e800072a32f68b6ccc4422936d96167c6e0728/src/core/hw/gfxip/gfx9/chip/gfx9_plus_merged_enum.h)
defines round-by-half0. These are checked field facts; no PAL implementation or
generated header is transplanted into the Rust planner.

The tiled extension independently verifies public KytyPS5 revision
`7b7f9a70c652afd6ec8ea8599884d9b4b3abc9f3`:
`src/graphics/guest_gpu/pm4.h` (blob
`186c8844ce8a6575b72172dd43c83e949d3dfb81`) defines the COLOR_SW_MODE shift14
and RESOURCE_TYPE shift24; `gpu_defs.h` (blob
`507ff702d41580c6a2ed200b9c2313dab2f5b02e`) defines kRenderTarget0x1b;
`tile.cpp` (blob `7043ca40c275473631beb5b37bf1f3729b7671e9`) defines
the one-level32bpp128-pixel block geometry and XOR pixel mapping. Core reuses
its existing detile/retile implementation rather than maintaining a second
address equation. Full padded LOAD, tight input and actual capacities count
together; reverse-conversion work and a shared upload/readback allowance are
admitted before effects. Backend readback actual capacity remains bounded by
that allowance. Active pixels overlay the retained original padded
snapshot, preserving every unused byte. The private completion carries actual
layout, geometry and padded range only after fenced same-owner atomic writeback.
That RGBA8_UNORM receipt is not a VideoOut sRGB transfer conversion, GPU-EOP
event or presentation acknowledgement. The native ordered driver can separately
consume a genuine current paused draw to capture an exactly matching registered
tiled surface; the pure planner cannot manufacture that authority.

The R_FLIP encoder/consumer contract additionally follows public revision
`7b7f9a70c652afd6ec8ea8599884d9b4b3abc9f3`, `src/libs/agc.cpp` (blob
`a29801fdf992948c594f62ec9045853e6158fd6e`) and
`src/graphics/guest_gpu/command_processor/pm4Handlers.cpp` (blob
`19034daafbd6da20d11f634debbf666c1054eed7`). The bounded Rust action walker
is a PentaX modification under the retained GPL-2.0-only/Kyty MIT notices.

Public execution ordering is independently anchored to
[KytyPS5 at7b7f9a70](https://github.com/KytyPS5/KytyPS5/tree/7b7f9a70c652afd6ec8ea8599884d9b4b3abc9f3):
`src/graphics/guest_gpu/graphicsRun.cpp` (Git blob
`41aeee369b497769e0d62f27ad7df94e7d069bdd`) delegates `DrawIndexAuto` to the
render executor; `src/graphics/host_gpu/renderer/renderDraw.cpp` (blob
`fada73b93e4f7c45f3bae23a653916d9445c4fc8`) covers `DrawAuto`,
`PrepareDrawRenderState`, `UpdateGraphicsDynamicState` and actual nonindexed
`vk.draw` emission. The source's broader caches, asynchronous machinery,
environment switches and title overrides are outside this port. See the GPU
NOTICE, root GPLv2 text and retained `LICENSES/Kyty-MIT.txt` grant.

The indexed extension follows that same public7b revision:
`pm4Handlers.cpp` (`CpOpDrawIndex`, `CpOpIndexType` and primitive-reset setter),
`pm4.h` (`GE_MULTI_PRIM_IB_RESET_EN`), `gpu_defs.h` (index types), and
`renderDraw.cpp` (`DrawIndex`, index width/upload and `drawIndexed`). The exact
packet ordering and count bound are source contracts; flag0, natural alignment
and known zero restart/base vertex describe this port's finite admission.
Original U16/U32 fixtures distinguish a degenerate index buffer from a later
fresh capture with different literal pixels, preserving tiled LOAD/padding.
An original indexed Draw/R_FLIP fixture exercises the existing owned GPU-ready
capture route without manufacturing display acknowledgement. These source
fixtures do not establish a commercial frame or compatibility result.

Graphics memory commands retain their position among draws and flips as typed
Write/Wait/Release/Drain actions. They use the compute planner's same closed
memory profiles and privately owned payloads; no caller can attach a detached
numeric action to native execution. Full temporary parser capacities and work
are conservatively charged inside the unchanged graphics limits; no quota is
credited when that temporary storage is dropped. The total action count stays
bounded by twice max_draws. The exact parser/ME Drain packet preserves an
existing completed draw's flip eligibility and current owner-bound receipt:
every prior GPU operation has already fenced and completed its same-owner
writeback, and Drain only admits its work/command counters. It writes no guest
memory, changes no target/configuration, and creates no new draw authority.
All other memory actions, including empty writes, invalidate the preceding
draw's eligibility. A later flip then needs a newly completed draw. Core execution reuses actual write permissions, physical
command-source disjointness, completed GPU fences, checked label writes and
post-write notifications. Unsatisfied waits stop explicitly; prior effects
cannot be replayed. GDS, unknown wait/dispatch modes, indirect draws, opaque
counts and unported destinations remain explicit execution refusals.
Graphics release INT_SEL1 and INT_SEL4 have no label or reference-clock write,
even if raw packet bytes retain a nonzero address. Their interrupt contexts
remain ordered. ACB INT_SEL1 retains its distinct write-before-interrupt path.

Flat CX/SH/UC indirect-register packets retain their complete five-DWORD grammar
and point to arrays of offset/value pairs. Ordered execution captures an array
after preceding writes and completed draws, then applies only known register
offsets to the current bank. A later draw is materialized from its original
packet and the replayed state; shader, target and resource preparation still
use actual current backing. A read-only register update preserves the existing
completed-draw receipt for a following flip. It does not fabricate a frame.

Array scratch, immutable command snapshots, range metadata and actions share
the existing host/work ceilings. Unknown registers and future writes aliasing
captured command/array sources are refused. No Vulkan execution or commercial
first frame is implied by a CPU command plan or shader-preparation fixture.

`CapturedCommands::capture` snapshots complete root and ordinary indirect
command buffers through the caller's read-only paused-memory adapter, before
planning or effects. Its exact four-DWORD `INDIRECT_BUFFER` profile retains
the full64-bit address,20-bit DWORD count and bit20 call/chain semantics. A call
returns to its parent continuation; a chain replaces its current frame and
discards that frame's continuation. Zero counts read no indirect backing.
Conditional IBs, predicates and unknown nonzero control profiles are refused.

Capture uses an iterative frame stack and active call/chain lineage. A repeated
source after a prior call returns remains legal; an active cycle is refused.
The unchanged `max_buffers` bounds aggregate root/IB captures, call depth and
chain lineage. All raw snapshot bytes, expanded packets, provenance, transient
allocation capacities and copy work remain within the existing byte/packet/
host/work ceilings. Source ranges include whole captured buffers, including
tails skipped by a chain, for later physical write/source exclusion.

The sealed capture feeds the existing static or ordered-array planner.
Register arrays remain execution-time reads after prior writes/draws; expanding
an IB does not prefetch arrays or invent shader/target state. Packet admission
errors map to original root buffer/dword or nested source address/dword.
The existing origin allocation moves into the successful plan, without another
allocation; read-only location accessors retain original packet provenance
through ordered execution and late failures. A capture prepares only one plan.
Adjacent emitted packets from the same source coalesce only when original and
expanded DWORD spans are both contiguous; every IB and source boundary remains
explicit. This retains exact location translation without per-packet overhead
for the existing full-size flat NOP profile.
`CaptureUsage` is charged separately from plan usage, so a caller must subtract
the capture's host/work bounds while preparing the plan. Moved provenance
capacity transfers from capture host usage to plan host usage; work is not
charged a second time for the move. Both products own their retained data.
`into_command_ranges` moves complete source metadata
without another allocation. Original CPU fixtures cover call/chain order,
fresh immutable snapshots, address/permission refusal, cycles and aggregate
caps, and nested WriteData/array/draw ordering; source coverage is not a
commercial compatibility or first-frame claim.

Stateful submission uses `capture_attempt`, whose refusal retains all admitted
attempt byte/buffer/work counters with the causal error. These counters are
committed to the session on both success and failure before returning; repeated
malformed or unreadable nested buffers cannot reset the attempted prefix's
budget. The failure contains no command product or executable prefix.

Exact `0xc0001024,0` dispatch-reset packets reset the selected modeled processor
at their command position, including stale shader/uconfig/context authority.
Known compute program addresses, local sizes, user SGPRs and indirect base
return to source zero values. Packed raw controls and other unmapped reset
fields remain unknown and cannot authorize a dispatch or draw. The pinned
processor reset leaves independent index base and instance fields unchanged;
index type/count and mapped uconfig defaults reset as specified by that source.
Exact `0xc0001200,state` packets accept only the low four payload bits and always
clear context alone: the payload does not select push/pop. SH/uconfig and index
or instance state remain intact. Both reset packets invalidate a prior draw's
flip eligibility; a fresh admitted draw can establish it again. Immutable
capture, nested source provenance, live array ordering, quotas and source alias
exclusions remain mandatory. These are bounded state semantics, not a complete
hardware reset-register table, asynchronous queue restart or display reset.
The separate single-flat-compute preparation API refuses a processor reset after
its sole dispatch rather than applying later state retroactively. Stateful
queue plans retain the dispatch-position snapshot and publish final reset state
only after admitted execution succeeds.

Exact pinned209 tagged AcquireMem and a bounded EventWrite subset are retained
as sealed ordered synchronization actions, including their actual payload.
AcquireMem reserved words must be zero. Partial-flush events 7/15/16 admit source
producer indices0/4; cache-flush events16h/31h/2ah/2ch/2eh admit index0. Addressed
queries, upstream ignored events, other indices and native58h AcquireMem are not
newly accepted. No cache-coordinate read or synthetic completion is performed.

Static and immutable indirect plans validate these actions before effects; live
ordered replay preserves their source position and existing work/action limits.
A pure barrier retains eligibility only from an already completed current draw;
it cannot create or repair eligibility invalidated by a write or target change.
The separate single-flat compute API refuses these ordered-only packets. The
execution receiver relies on completed real GPU fences and owned writeback, not
on invented labels, interrupts, presentation or cache contents.

Exact tagged AGC PushMarker/PopMarker packets are admitted as sealed ordered
annotations. Push consumes precisely its captured packet extent, including
binary bytes or a callback-modified terminator; Pop requires the producer's
fixed two words and zero payload. Truncated, unsupported or oversized-header
forms remain explicit refusals. Existing action/work/host bounds cover static,
immutable indirect capture and live replay. No text is exposed in diagnostics,
no host string is read, and no GPU state, cache, fence or event is fabricated.
Annotations preserve only existing draw eligibility; a prior write still
invalidates it. The separate single-flat compute API remains explicitly outside
this ordered annotation profile.
Context-state commands retain one saved context bank in the owned graphics
processor history. Clear resets the current known context while preserving the
saved bank; Push saves it; Pop restores it and releases the saved slot;
PushClear saves then clears. Nested Push and unmatched Pop refuse. SH, UCONFIG,
index and instance state are outside that bank. A full processor reset releases
the saved slot, while DCB context reset preserves it. Unestablished raw reset
fields remain unknown and required values still refuse until written.

Only the producer's exact tagged first3/5-DWORD packet, operation and zero
padding are admitted. Fragmented following R_ZERO packets retain their existing
NOP framing. Static, live-array and captured nested paths apply the operation
at its actual source position, preserving earlier draw snapshots. Every context
operation requires a fresh later draw before flip. Inline saved-state storage
and copies remain charged through the actual state/draw sizes and replay work.
Original consumers exercise owned cross-submission history, real shader/target
preparation, live array updates, precise failures and closed packet grammar;
they are distinct from physical GPU presentation evidence.

Primitive metadata retains raw VGT_GS_OUT_PRIM_TYPE(0x29b), GE_CNTL(0x25b)
and GE_USER_VGPR_EN(0x262) values. The first belongs to the saved context bank;
the two UCONFIG values remain independent. Context Clear resets GS output to its
proven zero default while preserving user configuration. Full processor reset
also restores zero active groups and user-VGPR enables. Push/Pop never restores
the UCONFIG fields, and the larger actual state size remains budgeted.

The existing flat renderer still excludes amplification, geometry and
tessellation stages. Within that profile, known GS output zero is the inactive
reset value and2 is triangle output; other known output modes refuse. Known GE
grouping fields(low18 bits) and extra user VGPR enables(low3 bits) must be zero;
reserved bits are ignored according to the primary decoder. Legacy absent fields
grant no new geometry authority. The CPU primitive helper faithfully computes
all source values even when a later draw profile refuses them. Original cases
cover raw values, active masks, save/reset separation and failure before payload
reads. No new hardware test or commercial compatibility is claimed.
