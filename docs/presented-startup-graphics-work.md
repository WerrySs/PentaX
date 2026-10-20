# Presented-startup graphics work policy

`probe-entry --present-window yes` selects an explicit, finite graphics work
profile before the native session is created. Draw preparation gets
268,435,456 work units (256 Mi units). Its graphics backend gets
68,719,476,736 work units (64 Gi units). These are conservative admission units,
not bytes, elapsed time, measured GPU instructions or a frame-rate claim.

Ordinary probes retain their existing compact limits. The driver defaults
`graphics_backend_work` to `None`, preserving `gpu.max_work`. A selected override
is applied only to a copy passed to graphics execution. Compute continues to use
the original GPU limits and its 67,108,864-unit ceiling. The cumulative driver
preparation ceiling remains 268,435,456 units. Compiler, shader, command, vertex,
memory and simultaneous staging caps, the existing fence timeout and the
supervisor deadline do not change. Every admission check and charge remains.

The profile addresses two independent compact-work ceilings encountered by the
original full-HD composition. For its one triangle, one instance, 14-instruction
VS and 6-instruction PS, define `N = 1920 * 1080 = 2,073,600` pixels,
`T = N * 4 = 8,294,400` tight bytes and
`P = 1920 * 1152 * 4 = 8,847,360` padded tiled bytes. Preparation and reverse
conversion charge:

| Charge | Work units |
| --- | ---: |
| Register configuration and shader-range checks | 130 |
| Six independently bounded compiler/decode envelopes | 25,165,824 |
| Two bounded source walkers, `2 * 256 * 256 * 64` | 8,388,608 |
| Vertex work, `3 * 14` | 42 |
| Conservative fragment work, `N * 6 * 4` | 49,766,400 |
| Padded LOAD snapshot, `P` | 8,847,360 |
| Actual tiled LOAD conversion, `4 * T` | 33,177,600 |
| Reserved reverse conversion, `4 * T` | 33,177,600 |
| Total before native target/alias preflight | 158,523,564 |

The old per-draw 134,217,728-unit ceiling refused this composition before Vulkan.
For its 540 target pages and three single-page command/shader ranges, native
preflight adds 301,336 units. The following EOP capture is independently
pre-admitted at `P + 5 * T + 256 = 50,319,616` units. Including the bounded
command plan, the complete original submission fits the unchanged cumulative
256 Mi-unit driver policy. This does not promise repeated frames or arbitrary
draw counts under that cumulative ceiling.

The Vulkan backend uses another conservative model:
`instances * (vertices * VS_SPIRV_words + triangles * N * PS_SPIRV_words)`.
It deliberately counts complete emitted modules, including non-executable
metadata, rather than claiming to know dynamic GPU instruction counts. Upload,
owned readback, shader words and bounded device/object/command visits are also
charged. Keeping the existing 32,768-word per-stage compiler cap gives the
following one-triangle full-HD upper bound, including both byte copies and
4,096 units covering the bounded device/object/command visits:

`N * 32768 + 3 * 32768 + 2 * 32768 + 2 * T + 4096 = 67,964,481,536`.

That bound fits 64 Gi units. It is a workload admission policy; it neither
predicts execution duration nor makes every possible 16 MiB target or shader
combination admissible. More triangles, instances or cumulative submissions
still encounter finite refusal. All required Vulkan capabilities remain live
runtime checks.

Pixel and compiler storage retain their separate phase contracts. This tiled
draw's logical pixel peak is `P + 3 * T = 33,730,560` bytes: original padded
LOAD, tight decoded input, upload and completed readback. Those draw buffers
are dropped before the EOP capture's `P + T = 17,141,760`-byte peak. Consumer
allocations enter the shared staging budget when actually retained; future
decoder/window ceilings are not counted as already allocated. Actual vector
capacities and queried Vulkan requirements can still refuse independently.
No physical playback, scanout, commercial frame or compatibility result follows
from these work calculations. The unchanged original GPU/window fixture must
exercise the composed draw, EOP, transport and actual presentation receipt.
