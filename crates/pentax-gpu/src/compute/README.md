# Direct compute resources

Compute preparation decodes the actual shader and analyzes it with an empty
resource profile. Only live descriptors whose four words come from consecutive,
unmodified user SGPRs are specialized. The retained bank has sixteen words, so
this profile supports zero through four direct raw storage buffers. Unused s0
words are ordinary user data and are never presumed to be a buffer descriptor.

One retained `ShaderAnalysis` continues through `finish`; decoding and compiler
phase reservations are charged once. Descriptor extents share the existing
aggregate buffer, host and work bounds. Raw unswizzled, four-byte aligned spans
retain the compiler's actual read/write/scalar requirements. Coherent accesses,
descriptor tables, transformed/dynamic descriptors and compute images remain
explicit refusals.

Prepared buffers follow the compiler manifest's first-use descriptor-array
order. Vulkan binds that one array with exactly N storage descriptors; a
zero-resource shader has no descriptor set, pool or dummy buffer. All writable
readbacks identify their original resource index. Legacy single-resource views
return `None` for zero or multiple resources, including a multi-resource shader
that happens to have only one output.

The native bridge proves current owner, permissions and physical exclusions for
every span before fetching any resource payload. Any writable overlap with
another resource or retained command backing is refused. It repeats admission
before GPU execution and after the actual fence. The complete indexed output set
and lengths must match the manifest, and one checked native transaction publishes
all outputs. Metadata, output allocations, repeated proofs and the full
cross-output transaction walk are bounded before GPU effects.

The separate LDS compiler contract carries the actual CP allocation in
`ComputeDescription.shared_memory_bytes`. GFX10.3 `COMPUTE_PGM_RSRC2.LDS_SIZE`
occupies bits 23:15, in 512-byte units. This implementation admits at most 64 KiB
per group and checks the physical device's shared-memory ceiling. Other R2
controls and the existing R1 numeric-mode profile keep their strict admission.
DS-only portable logical wave64 does not impersonate a native subgroup64;
genuine emitted subgroup operations retain their exact device requirements.

The allocation field follows the AMD-authored
[GFX10.3 register masks](https://github.com/torvalds/linux/blob/v6.12/drivers/gpu/drm/amd/include/asic_reg/gc/gc_10_3_0_sh_mask.h)
and the CP-register units documented in
[LLVM 19 AMDGPU usage](https://github.com/llvm/llvm-project/blob/llvmorg-19.1.0/llvm/docs/AMDGPUUsage.rst).
The discovery, ownership bridge and synthetic fixtures are original Rust changes;
existing port attribution and licensing remain in the surrounding crates.

The ignored owned-memory hardware fixture contains four original submissions:
zero resources, a live read-only scalar resource, a two-buffer copy using s8/s4,
and four writes in a deliberately different first-use order. It checks actual
fenced results and complete source/data guards. This source does not claim a
commercial frame, presentation or completed hardware validation.
