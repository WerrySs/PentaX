# Owned Vulkan subsystem

`vulkan::run_original(OriginalGpuFixture, &GpuLimits)` retains the original GPU
diagnostic entry. It takes no shader bytes, paths, handles, contexts or guest
addresses. Each call compiles its fixed original AMD instructions through the
actual `pentax-shader-compiler`; SPIR-V is never canned or read from a file.
The private device, resource, pipeline and command modules retain useful Vulkan
ownership paths also serve `run_compute(&PreparedCompute, &GpuLimits)`. That
entry consumes the opaque bounded command/compiler product from `pentax-gpu`,
uses its actual upload, push parameters and dispatch dimensions, and returns
readback only after a matching barrier and fence. Raw SPIR-V, Vulkan handles and
host pointers cannot be supplied. Core retains the native guest pause and commits
the results on the same owner. See [NOTICE](NOTICE)
for the exact source pin, ported seams, modifications and retained licenses.

The compute program writes eight u32 results, lane+9, into a real 32-byte storage
buffer. Its descriptor array binding and push-data placement come from the
compiler manifest. The explicit original V# words describe a synthetic 32-byte
resource; that integer is never a host pointer. The separate vertex/pixel pair
draws a full-screen triangle into a 4x4 RGBA8 image. Its opaque-red output must
replace a blue clear, then survive an actual image-to-buffer transfer. Every
returned byte must agree with the original program before a report is published.

## Ownership and admission

The Windows loader is opened only at the absolute OS system-directory path.
The retained entry/instance/device lifetime outlives every borrowed resource.
Device/queue owners are neither Send nor Sync, and the only queue is private.
No optional device feature is silently enabled or assumed. The profile requires
Vulkan 1.2 and the actual float32 signed-zero/Inf/NaN preservation property
declared by the compiler. A live subgroup requirement is admitted only for
native subgroup32 and the exact supported shader stages/BASIC/BALLOT operations.
Subgroup-size-control extensions, Int64 and draw-parameter shaders are currently
typed unsupported. Fixed device/family enumeration caps are 32/64; excess or
changing counts are refused without a driver-sized allocation/retry loop.

Buffers/images own dedicated allocations. Exact Vulkan memory requirements
are charged before memory allocation, and every visible object is charged
before creation. Descriptor/pipeline owners retain the actual buffer/image
borrow. Staging/readback Vec allocations are fallible and admitted first;
fixed source programs and driver enumeration use stack arrays. Decode and
compile receive independent finite budgets, conservatively charged in full
before either phase, per shader. These are application-visible resource costs;
driver-internal allocations and OS loader bookkeeping are outside the budget.

Host-visible memory can be coherent or noncoherent. Whole-allocation flush and
invalidate operations preserve noncoherent atom alignment. Explicit host/
compute/transfer barriers, render-pass layout transitions/dependencies and fence
completion precede readback. Uploaded contents, descriptors and shader modules
are not replaced by detached snapshots after recording.

Every partial construction rolls back through RAII in dependency order. A
submitted batch remains owned until its fence succeeds. On timeout/driver error
it conservatively drains the device before destroying any referenced resource.
The fence timeout bounds the requested wait, **not** a malfunctioning driver
call or device-idle cleanup; this diagnostic belongs in an isolated worker.
Success or explicit device loss is required before destroying submitted
resources. If final idle cleanup itself returns another error, the worker
terminates rather than unwinding through potentially live GPU resources.
No driver SDK, global configuration, browser or system feature is installed.

## Validation and remaining integration

Pure tests compile all three original stages and check bindings/push data,
required features and aggregate limits without loading Vulkan or calling FFI.
Native GPU execution requires a separate review of the frozen unsafe changes.
Compilation/official SPIR-V validation alone do not establish GPU execution.

The compute entry admits one manifest buffer of its actual checked length,
bounded dispatch work and host-supported local/group geometry. Compiler-required
features and the exact descriptor/push layout are checked before pipeline use.
Self-aliasing guest resources are refused by core before GPU calls. Readback
preserves unwritten bytes and Debug exposes counts instead of guest payloads.
Original PM4 diagnostics verify two dynamic guest-memory submissions. A separate
native x64 consumer now reaches the same backend through four scoped
Graphics5Driver single/batch submit imports and verifies actual changed results.
Neither diagnostic establishes commercial-game compatibility.

Persistent guest queues, general allocation aliasing/coherence, surface/swapchain
presentation, textures/samplers, caches/invalidation, depth/blend specialization,
multi-queue/timeline scheduling, additional shader capabilities and real game
resource binding remain outside this profile. No game frame, startup or game
compatibility is claimed by an original diagnostic result.
