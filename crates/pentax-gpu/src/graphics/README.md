# Guest graphics draw planning

`graphics::plan_commands` admits complete flat PM4 streams into an immutable
`CommandPlan`. Supported packets are type-2 padding, ordinary/header-only NOP,
graphics `SET_CONTEXT_REG`, `SET_SH_REG`, `SET_UCONFIG_REG`, `NUM_INSTANCES` and
`DRAW_INDEX_AUTO` with the exact auto-index initiator2. Each draw retains its
own known register snapshot and real vertex/instance counts. State-only streams
are supported. Zero-count draws consume their command position without shader
reads or a render product. Predicates, use-opaque counts, indirect buffers,
indexed draws, barriers, completion writes and unknown registers are refused.
No reset register value is invented. Final state is published by a queue owner
only after execution; a failed plan returns no executable prefix.

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

- Nonindexed triangle lists, explicit `NUM_INSTANCES` and zero known
  `GE_INDX_OFFSET`; real counts are bounded, with no modulo truncation.
- Unmerged ES-to-vertex stage, GS program disabled, no geometry/tessellation,
  scratch or LDS. GS resource/user registers supply the source vertex profile.
  Source `VGT_SHADER_STAGES_EN` selects wave32/64; PS is explicitly wave32.
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
- VS must export only position; PS must export only floating MRT0 and have
  no interpolated/system inputs. VertexIndex/InstanceIndex interfaces are
  allowed only when their manifest needs no unsupported DrawParameters.
  Descriptors, textures, vertex buffers, shader-data buffers, subgroups and
  undefined registers remain typed refusals.

These constraints describe a real guest draw path toward rendering. They do
not establish that a supplied commercial title fits this profile or that a
frame has been rendered or presented. Original encoded PM4/VS/PS fixtures live
only in tests; production contains no diagnostic triangle program.

Default command bounds are64 streams,256KiB command bytes,65536 packets,
64 retained draws,1048576 total vertices including instances,4MiB charged host
storage and4194304 work units. Draw preparation independently bounds256 shader
words per stage,32768 SPIR-V words per stage,16MiB target bytes,4096 dimensions,
1048576 vertices,128MiB charged host storage and134217728 work units. Compiler
phases have independent8MiB/4194304 caps; conservative outer charges also cover
decode/compiler arenas and worst-case raster work. Actual vector capacities
are checked after reservation. No budget is an AMD hardware limit.

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
event or presentation acknowledgement.

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
