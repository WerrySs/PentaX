// SPDX-License-Identifier: GPL-2.0-only
//! Default-off host-only evidence for a failed graphics admission or plan.
//! Private state/commands are never protocol fields, public artifacts or replay authority.
use super::{Failure, PausedNativeMemory, compute, draw};
use std::fmt::{self, Write as _};
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

const ENV: &str = "PENTAX_PRIVATE_GRAPHICS_FAILURE";
const MAX_PAYLOAD: usize = 64 * 1024;
const MAX_HEADER: usize = 4096;
const MARKER: &[u8] = b"\n-- graphics payload bytes (binary) --\n";
const STATE_BYTES: usize = draw::GraphicsState::DIAGNOSTIC_SNAPSHOT_BYTES;
const _: () = assert!(STATE_BYTES * 2 + 2 * 1024 < MAX_PAYLOAD);

/// Closed compiled diagnostic literals only. Unknown text retains the old category.
/// Tokens are at most64 ASCII bytes. No guest value, nested Debug, address or
/// arbitrary string enters public output.
pub(super) fn unsupported_category(reason: &str) -> &'static str {
    match reason {
        "GPU flip needs a fresh draw" => "graphics-unsupported-gpu-flip-needs-a-fresh-draw",
        "LOD statistics feedback is not supported" => {
            "graphics-unsupported-lod-statistics-feedback-is-not-supported"
        }
        "LOD statistics packet fields" => "graphics-unsupported-lod-statistics-packet-fields",
        "LOD statistics packet header" => "graphics-unsupported-lod-statistics-packet-header",
        "RGBA8 visible width profile" => "graphics-unsupported-rgba8-visible-width-profile",
        "buffer extent" => "graphics-unsupported-buffer-extent",
        "command address or alignment" => "graphics-unsupported-command-address-or-alignment",
        "command byte alignment" => "graphics-unsupported-command-byte-alignment",
        "command outside compute queue profile" => {
            "graphics-unsupported-command-outside-compute-queue-profile"
        }
        "command outside flat graphics profile" => {
            "graphics-unsupported-command-outside-flat-graphics-profile"
        }
        "command outside single flat compute profile" => {
            "graphics-unsupported-command-outside-single-flat-compute-profile"
        }
        "compute LDS allocation" => "graphics-unsupported-compute-lds-allocation",
        "compute SH register" => "graphics-unsupported-compute-sh-register",
        "compute buffer offset layout" => "graphics-unsupported-compute-buffer-offset-layout",
        "compute buffer source order" => "graphics-unsupported-compute-buffer-source-order",
        "compute buffer without live access" => {
            "graphics-unsupported-compute-buffer-without-live-access"
        }
        "compute direct buffer access/origin" => {
            "graphics-unsupported-compute-direct-buffer-access-origin"
        }
        "compute direct buffer count" => "graphics-unsupported-compute-direct-buffer-count",
        "compute direct buffer extent" => "graphics-unsupported-compute-direct-buffer-extent",
        "compute direct user register bank" => {
            "graphics-unsupported-compute-direct-user-register-bank"
        }
        "compute dispatch initiator" => "graphics-unsupported-compute-dispatch-initiator",
        "compute indirect register space" => "graphics-unsupported-compute-indirect-register-space",
        "compute initial registers" => "graphics-unsupported-compute-initial-registers",
        "compute push layout" => "graphics-unsupported-compute-push-layout",
        "compute raw unswizzled buffer descriptor" => {
            "graphics-unsupported-compute-raw-unswizzled-buffer-descriptor"
        }
        "compute resource configuration" => "graphics-unsupported-compute-resource-configuration",
        "compute resource specialization access" => {
            "graphics-unsupported-compute-resource-specialization-access"
        }
        "compute resource specialization contract" => {
            "graphics-unsupported-compute-resource-specialization-contract"
        }
        "compute resource specialization origin/order" => {
            "graphics-unsupported-compute-resource-specialization-or-33a52110"
        }
        "compute sampled resource backing is not implemented" => {
            "graphics-unsupported-compute-sampled-resource-backing-i-c0078cd2"
        }
        "compute start coordinates" => "graphics-unsupported-compute-start-coordinates",
        "conflicting pixel parameter aliases" => {
            "graphics-unsupported-conflicting-pixel-parameter-aliases"
        }
        "consumed graphics capture" => "graphics-unsupported-consumed-graphics-capture",
        "context already pushed" => "graphics-unsupported-context-already-pushed",
        "context not pushed" => "graphics-unsupported-context-not-pushed",
        "cross-stage parameter linkage" => "graphics-unsupported-cross-stage-parameter-linkage",
        "draw initiator" => "graphics-unsupported-draw-initiator",
        "draw-state vertex/instance offsets" => {
            "graphics-unsupported-draw-state-vertex-instance-offsets"
        }
        "failed ordered compute stream" => "graphics-unsupported-failed-ordered-compute-stream",
        "failed ordered graphics stream" => "graphics-unsupported-failed-ordered-graphics-stream",
        "graphics buffer count" => "graphics-unsupported-buffer-count",
        "graphics buffer extent" => "graphics-unsupported-buffer-extent-941e80eb",
        "graphics buffer specialization" => "graphics-unsupported-buffer-specialization",
        "graphics direct read-only buffer" => "graphics-unsupported-direct-read-only-buffer",
        "graphics extra user VGPR profile" => "graphics-unsupported-extra-user-vgpr-profile",
        "graphics fixed-function state" => "graphics-unsupported-fixed-function-state",
        "graphics indirect buffer cycle" => "graphics-unsupported-indirect-buffer-cycle",
        "graphics indirect control profile" => "graphics-unsupported-indirect-control-profile",
        "graphics indirect header profile" => "graphics-unsupported-indirect-header-profile",
        "graphics indirect packet length" => "graphics-unsupported-indirect-packet-length",
        "graphics indirect register" => "graphics-unsupported-indirect-register",
        "graphics memory packet profile" => "graphics-unsupported-memory-packet-profile",
        "graphics output primitive profile" => "graphics-unsupported-output-primitive-profile",
        "graphics packet header" => "graphics-unsupported-packet-header",
        "graphics parameter declaration" => "graphics-unsupported-parameter-declaration",
        "graphics parameter declaration mismatch" => {
            "graphics-unsupported-parameter-declaration-mismatch"
        }
        "graphics primitive grouping profile" => "graphics-unsupported-primitive-grouping-profile",
        "graphics push SGPR" => "graphics-unsupported-push-sgpr",
        "graphics push layout" => "graphics-unsupported-push-layout",
        "graphics raw unswizzled buffer descriptor" => {
            "graphics-unsupported-raw-unswizzled-buffer-descriptor"
        }
        "graphics read-only buffer manifest" => "graphics-unsupported-read-only-buffer-manifest",
        "graphics register" => "graphics-unsupported-register",
        "graphics resource/subgroup manifest" => "graphics-unsupported-resource-subgroup-manifest",
        "graphics sampled resource manifest" => "graphics-unsupported-sampled-resource-manifest",
        "graphics shader resources1" => "graphics-unsupported-shader-resources1",
        "graphics shader scratch/LDS/resources2" => {
            "graphics-unsupported-shader-scratch-lds-resources2"
        }
        "graphics user SGPR count" => "graphics-unsupported-user-sgpr-count",
        "image RGBA8 UNORM format" => "graphics-unsupported-image-rgba8-unorm-format",
        "image block swizzle or array/depth fields" => {
            "graphics-unsupported-image-block-swizzle-or-array-depth-fields"
        }
        "image component selection" => "graphics-unsupported-image-component-selection",
        "image compression or metadata fields" => {
            "graphics-unsupported-image-compression-or-metadata-fields"
        }
        "image minimum LOD beyond view" => "graphics-unsupported-image-minimum-lod-beyond-view",
        "image physical mip view remapping" => {
            "graphics-unsupported-image-physical-mip-view-remapping"
        }
        "image reserved fields" => "graphics-unsupported-image-reserved-fields",
        "image warning, sparse or sampling controls" => {
            "graphics-unsupported-image-warning-sparse-or-sampling-controls"
        }
        "incomplete compute command capture" => {
            "graphics-unsupported-incomplete-compute-command-capture"
        }
        "index alignment" => "graphics-unsupported-index-alignment",
        "index base profile" => "graphics-unsupported-index-base-profile",
        "index extent" => "graphics-unsupported-index-extent",
        "index offset extent" => "graphics-unsupported-index-offset-extent",
        "index type profile" => "graphics-unsupported-index-type-profile",
        "indexed draw profile" => "graphics-unsupported-indexed-draw-profile",
        "indexed indirect argument extent" => {
            "graphics-unsupported-indexed-indirect-argument-extent"
        }
        "indexed indirect draw requires live execution" => {
            "graphics-unsupported-indexed-indirect-draw-requires-liv-0cd33ed6"
        }
        "indexed indirect packet profile" => "graphics-unsupported-indexed-indirect-packet-profile",
        "indexed indirect vertex user SGPR" => {
            "graphics-unsupported-indexed-indirect-vertex-user-sgpr"
        }
        "indexed primitive restart" => "graphics-unsupported-indexed-primitive-restart",
        "indexed register profile" => "graphics-unsupported-indexed-register-profile",
        "indirect args extent" => "graphics-unsupported-indirect-args-extent",
        "indirect base address" => "graphics-unsupported-indirect-base-address",
        "indirect buffer cycle" => "graphics-unsupported-indirect-buffer-cycle-88038bd2",
        "indirect buffers in ordered register stream" => {
            "graphics-unsupported-indirect-buffers-in-ordered-register-stream"
        }
        "indirect control profile" => "graphics-unsupported-indirect-control-profile-2f0b29fe",
        "indirect register control" => "graphics-unsupported-indirect-register-control",
        "indirect register extent" => "graphics-unsupported-indirect-register-extent",
        "indirect register length" => "graphics-unsupported-indirect-register-length",
        "indirect register packet profile" => {
            "graphics-unsupported-indirect-register-packet-profile"
        }
        "linear image layout" => "graphics-unsupported-linear-image-layout",
        "memory command extent" => "graphics-unsupported-memory-command-extent",
        "merged/NGG/geometry/tessellation stages" => {
            "graphics-unsupported-merged-ngg-geometry-tessellation-stages"
        }
        "missing compute dispatch" => "graphics-unsupported-missing-compute-dispatch",
        "missing indirect args base" => "graphics-unsupported-missing-indirect-args-base",
        "native indirect compiler context" => {
            "graphics-unsupported-native-indirect-compiler-context"
        }
        "native indirect instance contract" => {
            "graphics-unsupported-native-indirect-instance-contract"
        }
        "no current compute register array" => {
            "graphics-unsupported-no-current-compute-register-array"
        }
        "no current graphics register array" => {
            "graphics-unsupported-no-current-graphics-register-array"
        }
        "no current indirect draw" => "graphics-unsupported-no-current-indirect-draw",
        "null sampled image" => "graphics-unsupported-null-sampled-image",
        "ordered LOD statistics action alignment" => {
            "graphics-unsupported-ordered-lod-statistics-action-alignment"
        }
        "ordered compute action alignment" => {
            "graphics-unsupported-ordered-compute-action-alignment"
        }
        "ordered compute action index" => "graphics-unsupported-ordered-compute-action-index",
        "ordered compute action kind" => "graphics-unsupported-ordered-compute-action-kind",
        "ordered compute memory action" => "graphics-unsupported-ordered-compute-memory-action",
        "ordered graphics action alignment" => {
            "graphics-unsupported-ordered-graphics-action-alignment"
        }
        "ordered graphics action kind" => "graphics-unsupported-ordered-graphics-action-kind",
        "ordered graphics packet profile" => "graphics-unsupported-ordered-graphics-packet-profile",
        "ordered indirect action alignment" => {
            "graphics-unsupported-ordered-indirect-action-alignment"
        }
        "ordered wait-flip action alignment" => {
            "graphics-unsupported-ordered-wait-flip-action-alignment"
        }
        "packet header profile" => "graphics-unsupported-packet-header-profile",
        "paired image minimum LOD/filter profile" => {
            "graphics-unsupported-paired-image-minimum-lod-filter-profile"
        }
        "pixel input register profile" => "graphics-unsupported-pixel-input-register-profile",
        "pixel interface manifest" => "graphics-unsupported-pixel-interface-manifest",
        "pixel parameter control" => "graphics-unsupported-pixel-parameter-control",
        "pixel sampled resource count/stage" => {
            "graphics-unsupported-pixel-sampled-resource-count-stage"
        }
        "queue reset after single dispatch" => {
            "graphics-unsupported-queue-reset-after-single-dispatch"
        }
        "raw unswizzled buffer descriptor" => {
            "graphics-unsupported-raw-unswizzled-buffer-descriptor-83050ab3"
        }
        "register command extent" => "graphics-unsupported-register-command-extent",
        "register command header" => "graphics-unsupported-register-command-header",
        "register count" => "graphics-unsupported-register-count",
        "register selector" => "graphics-unsupported-register-selector",
        "release event profile" => "graphics-unsupported-release-event-profile",
        "release memory destination" => "graphics-unsupported-release-memory-destination",
        "release memory profile" => "graphics-unsupported-release-memory-profile",
        "render target overlaps buffer source" => {
            "graphics-unsupported-render-target-overlaps-buffer-source"
        }
        "render target overlaps index source" => {
            "graphics-unsupported-render-target-overlaps-index-source"
        }
        "render target overlaps shader source" => {
            "graphics-unsupported-render-target-overlaps-shader-source"
        }
        "render-target address high bits" => "graphics-unsupported-render-target-address-high-bits",
        "render-target block alignment" => "graphics-unsupported-render-target-block-alignment",
        "render-target extent" => "graphics-unsupported-render-target-extent",
        "render-target mip levels" => "graphics-unsupported-render-target-mip-levels",
        "render-target swizzle profile" => "graphics-unsupported-render-target-swizzle-profile",
        "sampled image descriptor" => "graphics-unsupported-sampled-image-descriptor",
        "sampled image sampler" => "graphics-unsupported-sampled-image-sampler",
        "sampled image user registers" => "graphics-unsupported-sampled-image-user-registers",
        "sampled nonarray 2D image" => "graphics-unsupported-sampled-nonarray-2d-image",
        "sampled resource specialization" => "graphics-unsupported-sampled-resource-specialization",
        "sampled texture source/target extent" => {
            "graphics-unsupported-sampled-texture-source-target-extent"
        }
        "sampler address mode" => "graphics-unsupported-sampler-address-mode",
        "sampler anisotropic filter" => "graphics-unsupported-sampler-anisotropic-filter",
        "sampler border color table" => "graphics-unsupported-sampler-border-color-table",
        "sampler comparison, unnormalized, reduction or extra controls" => {
            "graphics-unsupported-sampler-comparison-unnormalized-re-02f3393f"
        }
        "sampler image pair" => "graphics-unsupported-sampler-image-pair",
        "sampler image source" => "graphics-unsupported-sampler-image-source",
        "sampler inverted LOD clamp" => "graphics-unsupported-sampler-inverted-lod-clamp",
        "sampler mip filter" => "graphics-unsupported-sampler-mip-filter",
        "sampler source specialization" => "graphics-unsupported-sampler-source-specialization",
        "sampler user registers" => "graphics-unsupported-sampler-user-registers",
        "shader address" => "graphics-unsupported-shader-address",
        "shader address high bits" => "graphics-unsupported-shader-address-high-bits",
        "shader extent" => "graphics-unsupported-shader-extent",
        "single memory packet profile" => "graphics-unsupported-single-memory-packet-profile",
        "static indirect draw" => "graphics-unsupported-static-indirect-draw",
        "static live draw action" => "graphics-unsupported-static-live-draw-action",
        "static register array" => "graphics-unsupported-static-register-array",
        "tagged wait profile" => "graphics-unsupported-tagged-wait-profile",
        "target decode extent" => "graphics-unsupported-target-decode-extent",
        "target extent" => "graphics-unsupported-target-extent",
        "texture extent" => "graphics-unsupported-texture-extent",
        "texture mip extent" => "graphics-unsupported-texture-mip-extent",
        "texture mip geometry" => "graphics-unsupported-texture-mip-geometry",
        "texture row backing" => "graphics-unsupported-texture-row-backing",
        "texture row extent" => "graphics-unsupported-texture-row-extent",
        "tiled target decoder is not bound" => {
            "graphics-unsupported-tiled-target-decoder-is-not-bound"
        }
        "triangle-list primitive" => "graphics-unsupported-triangle-list-primitive",
        "truncated command packet" => "graphics-unsupported-truncated-command-packet",
        "truncated packet" => "graphics-unsupported-truncated-packet",
        "truncated queue reset packet" => "graphics-unsupported-truncated-queue-reset-packet",
        "truncated register command" => "graphics-unsupported-truncated-register-command",
        "unbounded compute shader loop" => "graphics-unsupported-unbounded-compute-shader-loop",
        "unbounded graphics shader loop" => "graphics-unsupported-unbounded-graphics-shader-loop",
        "unconsumed compute actions" => "graphics-unsupported-unconsumed-compute-actions",
        "unconsumed graphics actions" => "graphics-unsupported-unconsumed-graphics-actions",
        "unexpected tiled test target" => "graphics-unsupported-unexpected-tiled-test-target",
        "unfinished ordered compute stream" => {
            "graphics-unsupported-unfinished-ordered-compute-stream"
        }
        "unfinished ordered graphics stream" => {
            "graphics-unsupported-unfinished-ordered-graphics-stream"
        }
        "uninitialized image LOD feedback" => {
            "graphics-unsupported-uninitialized-image-lod-feedback"
        }
        "unknown INDEX_BASE" => "graphics-unsupported-unknown-index-base",
        "unknown INDEX_BUFFER_SIZE" => "graphics-unsupported-unknown-index-buffer-size",
        "unknown INDEX_TYPE" => "graphics-unsupported-unknown-index-type",
        "unknown NUM_INSTANCES" => "graphics-unsupported-unknown-num-instances",
        "unknown draw indirect base" => "graphics-unsupported-unknown-draw-indirect-base",
        "unresolved ordered compute stream" => {
            "graphics-unsupported-unresolved-ordered-compute-stream"
        }
        "unresolved ordered graphics stream" => {
            "graphics-unsupported-unresolved-ordered-graphics-stream"
        }
        "unsupported graphics flip marker" => {
            "graphics-unsupported-unsupported-graphics-flip-marker"
        }
        "vertex interface manifest" => "graphics-unsupported-vertex-interface-manifest",
        "vertex sampled resource profile" => "graphics-unsupported-vertex-sampled-resource-profile",
        "wait memory control" => "graphics-unsupported-wait-memory-control",
        "write data control" => "graphics-unsupported-write-data-control",
        _ => "graphics-unsupported",
    }
}

#[derive(Clone, Copy)]
pub(super) enum Source<'a> {
    SealedExpanded(&'a [u8]),
    LiveRoot(compute::CommandRange),
}
#[derive(Clone, Copy)]
pub(super) enum Phase {
    CaptureAdmission,
    HeadPlanning,
    Execution,
}
pub(super) enum SelectedState<'a> {
    HeadInitial(&'a draw::GraphicsState),
    OrderedCurrent(&'a draw::GraphicsState),
    DrawSnapshot(&'a draw::GraphicsState),
}
pub(super) struct Context<'a> {
    pub retired: &'a draw::GraphicsState,
    pub selected: Option<SelectedState<'a>>,
    pub shader_words: usize,
    pub reset_before_graphics: bool,
    pub phase: Phase,
}

pub(super) fn capture_from_env(
    memory: &PausedNativeMemory<'_>,
    source: Source<'_>,
    context: Context<'_>,
    failure: &Failure,
) {
    // No opt-in means no allocation, state walk, new guest read or filesystem operation.
    if let Some(path) = std::env::var_os(ENV) {
        let _ = capture(Path::new(&path), memory, source, context, failure);
    }
}

struct Header {
    bytes: [u8; MAX_HEADER],
    length: usize,
}
#[derive(Clone, Copy)]
struct Candidate {
    address: Option<u64>,
    bytes: usize,
    status: &'static str,
}
impl fmt::Write for Header {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if value.len() > self.bytes.len() - self.length {
            return Err(fmt::Error);
        }
        self.bytes[self.length..self.length + value.len()].copy_from_slice(value.as_bytes());
        self.length += value.len();
        Ok(())
    }
}

fn capture(
    path: &Path,
    memory: &PausedNativeMemory<'_>,
    source: Source<'_>,
    context: Context<'_>,
    failure: &Failure,
) -> io::Result<()> {
    // Existing paths, including partial earlier captures, are never replaced.
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    // One fixed independent observer envelope is reserved BEFORE state or source walks.
    let mut payload = Vec::new();
    let allocated =
        payload.try_reserve_exact(MAX_PAYLOAD).is_ok() && payload.capacity() <= MAX_PAYLOAD;
    let mut state_bytes = 0;
    let mut captured = 0;
    let mut status = "allocation-unavailable";
    let shader_limit = context.shader_words.min(256) * 4;
    let mut shaders = [Candidate {
        address: None,
        bytes: 0,
        status: "allocation-unavailable",
    }; 2];
    let mut command_offset = 0;
    let (kind, total) = match source {
        Source::SealedExpanded(bytes) => ("sealed-expanded-not-original-root", bytes.len() as u64),
        Source::LiveRoot(root) => ("live-root-after-failure-not-original-sealed", root.size),
    };
    if allocated {
        payload.resize(MAX_PAYLOAD, 0);
        if !context
            .retired
            .diagnostic_snapshot(&mut payload[..STATE_BYTES])
        {
            return Err(io::Error::other("graphics diagnostic state format"));
        }
        state_bytes = STATE_BYTES;
        if let Some(selected) = &context.selected {
            let initial = match selected {
                SelectedState::HeadInitial(state)
                | SelectedState::OrderedCurrent(state)
                | SelectedState::DrawSnapshot(state) => state,
            };
            if !initial.diagnostic_snapshot(&mut payload[state_bytes..state_bytes + STATE_BYTES]) {
                return Err(io::Error::other("graphics diagnostic state format"));
            }
            state_bytes += STATE_BYTES;
        }
        let candidate_state: &draw::GraphicsState = match &context.selected {
            Some(
                SelectedState::HeadInitial(state)
                | SelectedState::OrderedCurrent(state)
                | SelectedState::DrawSnapshot(state),
            ) => state,
            None => context.retired,
        };
        command_offset = state_bytes;
        for (index, address) in candidate_state
            .diagnostic_shader_addresses()
            .into_iter()
            .enumerate()
        {
            shaders[index].address = address;
            shaders[index].status = "address-unavailable";
            if let Some(address) = address {
                shaders[index].status = if shader_limit == 0 {
                    "disabled-by-current-limit"
                } else {
                    "word-limit"
                };
                for byte in (0..shader_limit).step_by(4) {
                    let Some(at) = address.checked_add(byte as u64) else {
                        shaders[index].status = "address-overflow";
                        break;
                    };
                    if memory
                        .read_gpu(at, &mut payload[command_offset..command_offset + 4])
                        .is_err()
                    {
                        shaders[index].status = "gpu-read-unavailable";
                        break;
                    }
                    shaders[index].bytes += 4;
                    command_offset += 4;
                }
            }
        }
        let limit = total.min((MAX_PAYLOAD - command_offset) as u64) as usize & !3;
        let output = &mut payload[command_offset..command_offset + limit];
        match source {
            Source::SealedExpanded(bytes) => {
                output.copy_from_slice(&bytes[..limit]);
                captured = limit;
                status = "sealed-prefix";
            }
            Source::LiveRoot(root) => {
                if root.size == 0 {
                    status = "empty-root";
                } else if limit == 0 {
                    status = "partial-word-root";
                } else if memory.read_gpu(root.address, output).is_ok() {
                    captured = limit;
                    status = "live-prefix";
                } else {
                    status = "gpu-read-unavailable";
                }
            }
        }
        payload.truncate(command_offset + captured);
    }
    let phase = match context.phase {
        Phase::CaptureAdmission => "capture-admission",
        Phase::HeadPlanning => "head-planning",
        Phase::Execution => "execution",
    };
    let raw_reason = match failure {
        Failure::GraphicsPreparation(draw::Error::Unsupported(reason))
            if reason.len() <= 128 && reason.bytes().all(|b| (b' '..=b'~').contains(&b)) =>
        {
            *reason
        }
        _ => "not-an-admitted-static-unsupported-reason",
    };
    let category = failure.preparation_category().unwrap_or("graphics-service");
    let mut header = Header {
        bytes: [0; MAX_HEADER],
        length: 0,
    };
    write!(&mut header,
        "PENTAX_PRIVATE_GRAPHICS_FAILURE_V1\nphase={phase}\nerror_category={category}\nprivate_static_reason={raw_reason}\nsource_kind={kind}\nsource_bytes={total}\ncaptured_command_bytes={captured}\ncommand_prefix_complete={}\nstate_format=graphics-state-v1\nstate_record_bytes={STATE_BYTES}\nstate_bytes={state_bytes}\nstate_count={}\nstate0={}\nstate1={}\nreset_before_graphics={}\npayload_bytes={}\nmaximum_payload_bytes={MAX_PAYLOAD}\nread_status={status}\ncoverage=prefix-does-not-prove-failing-packet-or-complete-stream\n",
        allocated && captured as u64 == total,
        state_bytes / STATE_BYTES,
        if state_bytes != 0 { "last-completed-graphics-state-not-partial-execution" } else { "unavailable" },
        match context.selected { _ if state_bytes < STATE_BYTES * 2 => "not-selected-or-unavailable", Some(SelectedState::HeadInitial(_)) => "exact-selected-head-planning-initial", Some(SelectedState::OrderedCurrent(_)) => "ordered-current-state-not-completion-or-static-final", Some(SelectedState::DrawSnapshot(_)) => "current-static-draw-snapshot-not-completion", None => "not-selected-or-unavailable" },
        context.reset_before_graphics, payload.len(),
    ).map_err(|_| io::Error::other("graphics diagnostic header bound"))?;
    if let Source::LiveRoot(root) = source {
        write!(&mut header, "live_root_address=0x{:016x}\nlive_root_selection=first-source-range-not-necessarily-failing-range\n", root.address)
            .map_err(|_| io::Error::other("graphics diagnostic header bound"))?;
    }
    write!(&mut header, "shader_max_prefix_bytes={shader_limit}\ncommand_offset_bytes={command_offset}\nshader_candidates_note=known-snapshot-addresses-only;prefixes-not-complete-shaders-or-failing-stage-proof\n")
        .map_err(|_| io::Error::other("graphics diagnostic header bound"))?;
    for (label, candidate) in ["vs", "ps"].into_iter().zip(shaders) {
        write!(
            &mut header,
            "{label}_captured_bytes={}\n{label}_read_status={}\n",
            candidate.bytes, candidate.status
        )
        .map_err(|_| io::Error::other("graphics diagnostic header bound"))?;
        if let Some(address) = candidate.address {
            write!(&mut header, "{label}_candidate_address=0x{address:016x}\n")
                .map_err(|_| io::Error::other("graphics diagnostic header bound"))?;
        }
    }
    file.write_all(&header.bytes[..header.length])?;
    file.write_all(MARKER)?;
    file.write_all(&payload)?;
    Ok(())
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests {
    use super::*;
    use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

    fn command_bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }
    fn limits() -> draw::CommandLimits {
        draw::CommandLimits {
            max_draws: 1,
            max_buffers: 1,
            max_host_bytes: 1024 * 1024,
            ..Default::default()
        }
    }
    fn path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "pentax-original-graphics-{label}-{}-{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    fn split(raw: &[u8]) -> (&str, &[u8]) {
        let at = raw
            .windows(MARKER.len())
            .position(|bytes| bytes == MARKER)
            .unwrap();
        assert!(at <= MAX_HEADER);
        assert!(raw.len() <= MAX_HEADER + MARKER.len() + MAX_PAYLOAD);
        (
            std::str::from_utf8(&raw[..at]).unwrap(),
            &raw[at + MARKER.len()..],
        )
    }

    #[test]
    fn closed_graphics_reasons_come_from_actual_typed_refusals_and_reject_unlisted_payloads() {
        for (words, expected) in [
            (
                vec![0xc001_1068, 2, 0],
                "graphics-unsupported-context-not-pushed",
            ),
            (vec![0xc001_6900, 0x3ff, 7], "graphics-unsupported-register"),
        ] {
            let bytes = command_bytes(&words);
            let error = draw::plan_commands(&[&bytes], draw::GraphicsState::empty(), &limits())
                .err()
                .expect("original refusal missing");
            assert!(matches!(error, draw::Error::Unsupported(_)));
            let failure = Failure::GraphicsPreparation(error);
            let before = failure;
            assert_eq!(failure.preparation_category(), Some(expected));
            assert_eq!(failure, before);
        }
        let specific = Failure::GraphicsPreparation(draw::Error::Unsupported("shader address"));
        assert_eq!(
            specific.preparation_category(),
            Some("graphics-shader-profile")
        );
        let unlisted = Failure::GraphicsPreparation(draw::Error::Unsupported(
            "unlisted-private-payload-0xfeedbeef",
        ));
        assert_eq!(
            unlisted.preparation_category(),
            Some("graphics-unsupported")
        );
        assert_eq!(
            unsupported_category("a\tprivate\naddress"),
            "graphics-unsupported"
        );
    }

    #[test]
    fn private_graphics_evidence_distinguishes_sealed_and_live_bytes_states_caps_and_primary_error()
    {
        let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x30000).unwrap();
        let base = owner.reserved_ranges().next().unwrap().address;
        owner
            .map_at(
                base,
                0x20000,
                NativePermissions::READ_WRITE.with_gpu(true, false),
            )
            .unwrap();
        owner
            .map_at(base + 0x20000, 0x10000, NativePermissions::READ_WRITE)
            .unwrap();
        let mut memory = owner.pause().unwrap();
        let words = vec![0x8000_0000; MAX_PAYLOAD / 4 + 4];
        let stream = command_bytes(&words);
        memory.write(base, &stream).unwrap();
        let root = compute::CommandRange {
            address: base,
            size: stream.len() as u64,
        };
        let captured = draw::CapturedCommands::capture_attempt(
            &[root],
            &super::super::super::Memory(&memory),
            &limits(),
        )
        .unwrap_or_else(|_| panic!("original owned capture missing"));
        memory.write(base, &0xdead_beefu32.to_le_bytes()).unwrap();
        let retired = draw::plan_commands(
            &[&command_bytes(&[0xc001_7900, 0x242, 4])],
            draw::GraphicsState::for_new_processor(),
            &limits(),
        )
        .unwrap()
        .final_state();
        let initial = draw::GraphicsState::for_new_processor();
        let failure = Failure::GraphicsPreparation(draw::Error::Unsupported("context not pushed"));
        let before = failure;
        let sealed_path = path("sealed");
        capture(
            &sealed_path,
            &memory,
            Source::SealedExpanded(captured.diagnostic_bytes()),
            Context {
                retired: &retired,
                selected: Some(SelectedState::HeadInitial(&initial)),
                shader_words: 256,
                reset_before_graphics: true,
                phase: Phase::HeadPlanning,
            },
            &failure,
        )
        .unwrap();
        let raw = std::fs::read(&sealed_path).unwrap();
        let (header, payload) = split(&raw);
        assert!(header.contains("source_kind=sealed-expanded-not-original-root"));
        assert!(header.contains("state1=exact-selected-head-planning-initial"));
        assert!(header.contains("reset_before_graphics=true"));
        assert!(header.contains("command_prefix_complete=false"));
        assert!(header.contains("private_static_reason=context not pushed"));
        let scalar = (0x400 * 2 + 0x100) * 5;
        assert_eq!(&payload[scalar..scalar + 9], &[1, 4, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            &payload[STATE_BYTES + scalar..STATE_BYTES + scalar + 9],
            &[0; 9]
        );
        let instances = STATE_BYTES + scalar + 5 * 9;
        assert_eq!(
            &payload[instances..instances + 9],
            &[1, 1, 0, 0, 0, 0, 0, 0, 0]
        );
        let command = &payload[2 * STATE_BYTES..];
        assert!(command.len().is_multiple_of(4));
        assert_eq!(command, &stream[..command.len()]);
        assert!(command.len() < stream.len() && payload.len() <= MAX_PAYLOAD);
        let existing = capture(
            &sealed_path,
            &memory,
            Source::LiveRoot(root),
            Context {
                retired: &retired,
                selected: None,
                shader_words: 256,
                reset_before_graphics: false,
                phase: Phase::CaptureAdmission,
            },
            &failure,
        )
        .unwrap_err();
        assert_eq!(existing.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&sealed_path).unwrap(), raw);
        std::fs::remove_file(&sealed_path).unwrap();

        let live_path = path("live");
        capture(
            &live_path,
            &memory,
            Source::LiveRoot(root),
            Context {
                retired: &retired,
                selected: None,
                shader_words: 256,
                reset_before_graphics: false,
                phase: Phase::Execution,
            },
            &failure,
        )
        .unwrap();
        let raw = std::fs::read(&live_path).unwrap();
        let (header, payload) = split(&raw);
        assert!(header.contains("source_kind=live-root-after-failure-not-original-sealed"));
        assert!(header.contains("state1=not-selected-or-unavailable"));
        assert_eq!(
            &payload[STATE_BYTES..STATE_BYTES + 4],
            &0xdead_beefu32.to_le_bytes()
        );
        std::fs::remove_file(&live_path).unwrap();

        let denied_path = path("denied");
        capture(
            &denied_path,
            &memory,
            Source::LiveRoot(compute::CommandRange {
                address: base + 0x20000,
                size: 16,
            }),
            Context {
                retired: &retired,
                selected: None,
                shader_words: 256,
                reset_before_graphics: false,
                phase: Phase::CaptureAdmission,
            },
            &failure,
        )
        .unwrap();
        let raw = std::fs::read(&denied_path).unwrap();
        let (header, payload) = split(&raw);
        assert!(header.contains("read_status=gpu-read-unavailable"));
        assert!(header.contains("captured_command_bytes=0"));
        assert_eq!(payload.len(), STATE_BYTES);
        std::fs::remove_file(&denied_path).unwrap();

        // A late array failure still owns its sealed program and actual
        // partially advanced ordered state. Neither is a static future final
        // state, and changing caller bytes cannot replace this borrowed view.
        let table = base + 0x18000;
        memory
            .write(table, &command_bytes(&[0x80, 19, 0x3ff, 23]))
            .unwrap();
        let program = command_bytes(&[
            0xc003_9f00,
            table as u32,
            (table >> 32) as u32,
            0x8000_0000,
            2,
        ]);
        let ordered_root = compute::CommandRange {
            address: base + 0x12000,
            size: program.len() as u64,
        };
        memory.write(ordered_root.address, &program).unwrap();
        let mut ordered = draw::OrderedCommands::prepare(
            &[&program],
            &[ordered_root],
            initial,
            &draw::CommandLimits {
                max_buffers: 4,
                ..limits()
            },
        )
        .unwrap();
        assert!(matches!(
            ordered.next_step().unwrap(),
            Some(draw::OrderedStep::Array)
        ));
        let error = ordered
            .resolve_array(&super::super::super::Memory(&memory))
            .unwrap_err();
        assert!(matches!(
            error,
            draw::Error::Unsupported("graphics register")
        ));
        memory
            .write(ordered_root.address, &0xdead_beefu32.to_le_bytes())
            .unwrap();
        let (sealed, current) = ordered.diagnostic_view();
        let ordered_path = path("ordered");
        capture(
            &ordered_path,
            &memory,
            Source::SealedExpanded(sealed),
            Context {
                retired: &retired,
                selected: Some(SelectedState::OrderedCurrent(current)),
                shader_words: 256,
                reset_before_graphics: false,
                phase: Phase::Execution,
            },
            &Failure::GraphicsPreparation(error),
        )
        .unwrap();
        let raw = std::fs::read(&ordered_path).unwrap();
        let (header, payload) = split(&raw);
        assert!(header.contains("state1=ordered-current-state-not-completion-or-static-final"));
        assert!(header.contains("command_prefix_complete=true"));
        assert_eq!(&payload[0x80 * 5..0x80 * 5 + 5], &[0; 5]);
        let advanced = STATE_BYTES + 0x80 * 5;
        assert_eq!(&payload[advanced..advanced + 5], &[1, 19, 0, 0, 0]);
        assert_eq!(&payload[STATE_BYTES * 2..], program.as_slice());
        assert!(ordered.final_state().is_err());
        std::fs::remove_file(&ordered_path).unwrap();

        let vertex = base + 0x1a000;
        let pixel = base + 0x20000; // Owned CPU backing deliberately denies GPU reads.
        memory
            .write(
                vertex,
                &command_bytes(&[0x1357_2468, 0x89ab_cdef, 0xfeed_beef]),
            )
            .unwrap();
        let programs = command_bytes(&[
            0xc002_7600,
            0xc8,
            (vertex >> 8) as u32,
            (vertex >> 40) as u32,
            0xc002_7600,
            0x08,
            (pixel >> 8) as u32,
            (pixel >> 40) as u32,
        ]);
        let candidates = draw::plan_commands(&[&programs], initial, &limits())
            .unwrap()
            .final_state();
        assert_eq!(
            candidates.diagnostic_shader_addresses(),
            [Some(vertex), Some(pixel)]
        );
        let shader_path = path("candidate-prefixes");
        capture(
            &shader_path,
            &memory,
            Source::SealedExpanded(&programs),
            Context {
                retired: &retired,
                selected: Some(SelectedState::HeadInitial(&candidates)),
                shader_words: 2,
                reset_before_graphics: false,
                phase: Phase::HeadPlanning,
            },
            &Failure::GraphicsPreparation(draw::Error::Compile),
        )
        .unwrap();
        let raw = std::fs::read(&shader_path).unwrap();
        let (header, payload) = split(&raw);
        assert!(header.contains("shader_max_prefix_bytes=8"));
        assert!(header.contains("vs_captured_bytes=8\nvs_read_status=word-limit"));
        assert!(header.contains("ps_captured_bytes=0\nps_read_status=gpu-read-unavailable"));
        assert!(header.contains("prefixes-not-complete-shaders-or-failing-stage-proof"));
        assert_eq!(
            &payload[2 * STATE_BYTES..2 * STATE_BYTES + 8],
            &command_bytes(&[0x1357_2468, 0x89ab_cdef])
        );
        assert_eq!(&payload[2 * STATE_BYTES + 8..], programs.as_slice());
        std::fs::remove_file(&shader_path).unwrap();
        let mut word = [0; 4];
        memory.read(base, &mut word).unwrap();
        assert_eq!(word, 0xdead_beefu32.to_le_bytes());
        assert_eq!(failure, before);
    }
    #[test]
    fn owned_driver_and_real_suspend_fifo_choose_only_the_public_processor_defaults() {
        use super::super::{Driver, Function, GraphicsProgress};
        let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
        let base = owner.reserved_ranges().next().unwrap().address;
        owner
            .map_at(
                base,
                0x10000,
                NativePermissions::READ_WRITE.with_gpu(true, true),
            )
            .unwrap();
        let mut memory = owner.pause().unwrap();
        let mut driver = Driver::new(Default::default());
        assert_eq!(driver.graphics, draw::GraphicsState::for_new_processor());
        assert_ne!(driver.graphics, draw::GraphicsState::empty());
        let mut packet = [0; 16];
        packet[..8].copy_from_slice(&(base + 0x1000).to_le_bytes());
        packet[8..12].copy_from_slice(&4u32.to_le_bytes());
        memory.write(base, &packet).unwrap();
        memory
            .write(
                base + 0x1000,
                &command_bytes(&[0xc000_2f00, 7, 0xc000_1200, 0]),
            )
            .unwrap();
        assert_eq!(
            driver.submit(&mut memory, Function::Dcb, [base, 0, 0, 0, 0, 0]),
            Ok(0)
        );
        let GraphicsProgress::Waiting(boundary) = driver
            .queue_graphics_suspend(&mut memory, None, None)
            .unwrap()
        else {
            panic!("SuspendPoint overtook the accepted DCB");
        };
        assert!(
            !driver
                .pump_direct_graphics(&mut memory, None, None)
                .unwrap()
        );
        // Real ContextClear touches only its context bank: NUM_INSTANCES7
        // remains explicit until the separate trusted SuspendPoint boundary.
        let explicit = draw::plan_commands(
            &[&command_bytes(&[0xc000_2f00, 7, 0xc000_1200, 0])],
            draw::GraphicsState::for_new_processor(),
            &limits(),
        )
        .unwrap()
        .final_state();
        assert_eq!(driver.graphics, explicit);
        assert_ne!(driver.graphics, draw::GraphicsState::for_new_processor());
        assert!(!driver.frame.reset_before_graphics);
        assert!(matches!(
            driver
                .resume_graphics_execution(&mut memory, boundary, None, None)
                .unwrap(),
            GraphicsProgress::Complete(0)
        ));
        assert!(driver.frame.reset_before_graphics);
        assert_eq!(driver.frame.boundaries, 1);
        // Suspend marks the trusted next-head reset; it does not retroactively
        // relabel the last completed state or acknowledge any presentation.
        assert_eq!(driver.graphics, explicit);
        packet[8..12].copy_from_slice(&1u32.to_le_bytes());
        memory.write(base, &packet).unwrap();
        memory
            .write(base + 0x1000, &0x8000_0000u32.to_le_bytes())
            .unwrap();
        assert_eq!(
            driver.submit(&mut memory, Function::Dcb, [base, 0, 0, 0, 0, 0]),
            Ok(0)
        );
        assert!(
            !driver
                .pump_direct_graphics(&mut memory, None, None)
                .unwrap()
        );
        assert_eq!(driver.graphics, draw::GraphicsState::for_new_processor());
        assert!(!driver.frame.reset_before_graphics);
        assert_eq!(
            (
                driver.usage.draws,
                driver.usage.dispatches,
                driver.usage.written_bytes
            ),
            (0, 0, 0)
        );
    }
}
