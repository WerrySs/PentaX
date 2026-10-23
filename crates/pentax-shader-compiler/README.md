# pentax-shader-compiler

A safe Rust adaptation of the shader compiler in
[KytyPS5-Performance at 209dffe9032d72749257ce18680d506b891284de](https://github.com/WerrySs/KytyPS5-Performance/tree/209dffe9032d72749257ce18680d506b891284de/src/graphics/shader).
This is a modified direct port, with explicit limits and unsupported cases.
It is not complete compiler parity or a claim of game compatibility.
See [NOTICE](NOTICE), [LICENSE](LICENSE), [Kyty-MIT.txt](Kyty-MIT.txt), and
[Khronos-MIT.txt](Khronos-MIT.txt) for provenance and retained notices.

`ShaderProgram::decode` owns the source words through `pentax-shader`, checks
the source termination walk, and proves direct branch targets are instruction
starts or the program end. `Cfg::build` constructs and structurizes a graph
whose views borrow that exact program. `compile` translates into a private typed
IR, builds SSA, folds constants, eliminates dead code, tracks resources and
interfaces, allocates the source binding layout, and emits owned SPIR-V 1.3.
Failure returns an error and no partial compiled module. There is no Vulkan
execution, guest memory access, file loading, or title-specific inference here.

## Ported scope

- Vertex, pixel, and compute entry mappings from the pinned compiler, configured
  by validated caller profiles.
- Reducible direct branches, selections and natural loops, structured merges,
  loop-carried phis, scalar condition flags, and EXEC/VCC lane predicates.
- Source integer and bit operations, 32/64-bit scalar operations, 32-bit vector
  integer/float operations, comparisons, conversions, masks and common math
  lowerings. Decoding an opcode does not imply that this compiler lowers it.
- Raw dword SMEM buffer loads and MUBUF loads/stores, with descriptors sourced
  from four consecutive unmodified user SGPRs and explicitly specialized by the
  caller. Buffer accesses retain the source software bounds checks.
- Pixel parameter interpolation and uncompressed vertex position/PARAM and pixel
  MRT exports, with caller-resolved linkage locations.
- The source default code-generation paths for fast float min/max, fast PKRTZ,
  single float-to-integer saturation, and position-slice `NoContraction`.

Unported forms return typed errors when encountered in translated code. These
include 16-bit/packed and float64 operations, DPP/cross-lane forms, relative M0
addressing, WQM, indirect calls/jump tables and irreducible dispatcher/goto
fallbacks; mesh/local/tessellation stages; SRT/BDA, scalar-address/flat/global/
scratch/DS/GDS/image memory, typed/formatted/sub-dword/atomic/status accesses;
and compressed, depth, auxiliary-position, or pixel-valid-mask/discard exports.
This port does not reproduce optional device workarounds or a renderer's
resource caches and synchronization.

Supported implicit interpolation retains real P1/P2 provenance through SSA:
P2 must consume the matching ATTR/channel P1 partial and an unmodified i/j
builtin pair from the same declared interpolation mode. Plain register copies
preserve these identities. Computed coordinates, mismatched/overwritten
partials and live unconsumed P1 results return typed errors instead of silently
discarding their values. The supported nonflat interface remains center
perspective; flat MOV mode2 is separate. Host location mapping and interpolation
class come from the validated caller profile, never from a guessed register
default. Original fragment fixtures explicitly initialize their center pair.

The pinned compiler substitutes zero for ordinary uninitialized registers.
This port deliberately requires either `UndefinedRegisterPolicy::Reject`
(default) or explicit `OpUndef`, counted in the manifest. Neither choice is
presented as a proven hardware initialization rule. Live subgroup operations
require the caller to guarantee the exact guest wave size, 32 or 64; splitting
a guest wave across different host subgroup sizes is not implemented.

## Host contract

Construct a `Profile` from `StageDescription`, then call
`compile(&program, &profile, &limits)`. The resulting `CompiledShader` owns both
the SPIR-V words and a read-only `Manifest`; it does not borrow the source or
profile. The profile supplies stage registers, wave size, workgroup dimensions,
pixel linkage, MRT classes, and direct buffer stride/swizzle/index-stride and
access fields. It never supplies guest pointers or shader code addresses.
The 1024-invocation profile cap is a limit of this interface, not a universal
claim about PS5 or host hardware.

For caller specialization from actual resources, `analyze(&program, profile,
&limits)` consumes an empty-buffer profile and returns an opaque
`ShaderAnalysis`. Its read-only buffer requirements come from the same optimized
IR after SSA, folding, dead-code elimination and policy checks. Dead loads do
not declare resources; copied unmodified user values retain their actual user
SGPR identity, while computed/replaced descriptors are refused. `finish` accepts
only explicit `BufferDescription` values and continues resource validation,
layout and emission on that same IR and monotonic budget. It cannot replace the
stage, entry mapping, user range, interfaces or subgroup policy. No guest-memory
reader or callback enters either operation. Missing/unused/access errors and the
existing 64-buffer compiler bound remain active. The graphics consumer admits
one read-only raw buffer per stage; this does not change the compiler's generic
resource count.

The current vertex entry mapping replaces s2/s3 with its source system values
after user initialization. A descriptor in vertex quad0 therefore lacks the
required unmodified user provenance and is refused. The graphics slice uses
valid direct vertex quads4/8/12 and pixel quads0/4/8/12 within the declared user
count. This boundary is preserved rather than changing entry values to make a
descriptor appear valid.

Bindings retain the source `NativeBinding`: descriptor set 0, stage group times
57 plus the descriptor kind. Buffers are one storage-buffer descriptor array
in first-use order, at kind 0; shader data uses kind 54 if it does not fit in
the 128-byte push block. Vertex/compute use group 0 and pixel uses group 1.
The manifest supplies the resulting binding numbers, buffer/profile association,
read/write/coherence and extent metadata, stage interfaces and entry point.

`PushData` identifies the start dword and ordered user SGPR values, followed by
buffer byte offsets packed four per dword. Its `memory_offset_dword` is relative
to shader data, while `start_dword` is relative to the push block. When
`shader_data_binding()` is present, shader data instead starts at dword 0 in
that storage buffer. Hosts must construct the layout from the manifest and
must not infer resource identities from instruction addresses.

`required_features()` reports the actual emitted SPIR-V version, capabilities,
float-controls extension and entry-point preservation mode. Every current module
requires `SignedZeroInfNanPreserve 32`, including the corresponding Vulkan
`shaderSignedZeroInfNanPreserveFloat32` property and core/extension support.
Int64, draw parameters and subgroup basic/ballot requirements depend on emitted
operations. `required_subgroup_size()` supplies the required exact size where
subgroup operations survive. These requirements must be checked and enabled by
the host; successful compilation or offline SPIR-V validation does not establish
device support. Workgroup and interface limits also remain host responsibilities.

## Limits and validation

`Limits` separately bounds decoder input, graph blocks/edges, IR instructions/
operands, SSA table slots, SPIR-V words/IDs, work, and arena capacity bytes.
Capacity accounting is monotonic across temporary and output arenas; it is a
conservative cumulative allocation charge, not exact live heap usage or allocator
bookkeeping. Owned input decoding has its own decoder limits, independently of
the compilation call. Profile vectors are caller-owned before compilation.
Arena growth uses `try_reserve_exact`; recoverable reservation failures return
`CompileError::Allocation`. This does not promise recovery from an allocator or
operating system that aborts the process. Graph walks are iterative and repeated
identity resolution, interning comparisons and in-place graph sorting consume
the same finite work allowance.

`ShaderProgram::retained_host_bytes`, `CompiledShader::retained_host_bytes` and
the manifest's matching accessor report actual recursive Rust vector capacities
that remain owned after their calls return, excluding inline metadata and
allocator bookkeeping. `CompiledShader::charged_bytes` keeps its distinct
monotonic compilation-arena meaning. A caller may release a temporary phase
envelope only after the call returns; retained source/target tables, SPIR-V and
manifest capacities must still count until their actual owners are dropped.

All fixtures are original synthetic programs. The end-to-end matrix keeps live
results and exercises scalar/vector aliases as well as stage interfaces, buffer
writes and a carried loop value. The tests compare manifest requirements against
the emitted binary declarations. To additionally validate every accepted fixture
with the official validator, set `PENTAX_SPIRV_VAL` to a local `spirv-val`
executable; optionally set `PENTAX_SHADER_OUTPUT` to a private output directory,
then run:

```text
cargo test -p pentax-shader-compiler -- --nocapture
cargo test -p pentax-shader-compiler --release -- --nocapture
cargo clippy -p pentax-shader-compiler --all-targets -- -D warnings
```

Without that environment variable the tests do not invoke an external validator.
The local validation used official SPIRV-Tools
`7f2d9ee926f98fc77a3ed1e1e0f113b8c9c49458` and matching SPIRV-Headers
`01e0577914a75a2569c846778c2f93aa8e6feddd`, the working fork's submodule pins,
with `spirv-val --target-env vulkan1.1`. These checks establish structural validity
of the original output modules, not GPU execution or arbitrary-shader correctness.
A separate `analyze_native_indirect_vertex` entry accepts only the explicit native
indexed-indirect vertex convention. It issues a private profile context with
BaseInstance fixed to0 before IR optimization and SPIR-V emission, rejecting a
start-instance SGPR override or nonvertex program. Buffer specialization retains
that context, and a read-only manifest records the dependency. Actual emitted
capabilities and interfaces are still derived normally. Generic `Profile::new`,
`compile` and `analyze` keep dynamic BaseInstance and their previous requirements.
The dedicated entry grants no GPU authority: the graphics bridge must bind its
sealed prepared mode and the backend must enforce actual firstInstance0.
