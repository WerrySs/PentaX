// SPDX-License-Identifier: GPL-2.0-only
// Typed diagnostics of the KytyPS5-Performance compiler port (209dffe). The
// pinned source reports most of these conditions through EXIT/abort; this port
// returns them with the original byte PC and opcode where one exists.

use std::fmt;

use pentax_shader::{DecodeError, Opcode};

use crate::LimitKind;

/// Program-boundary failures of the ported `DecodeProgram` walker and the
/// `BuildGraph` label checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgramErrorKind {
    /// The source exits on empty recompiler input.
    Empty,
    /// The code boundary was reached before a terminating S_ENDPGM.
    MissingTermination,
    /// Words remain after the terminating S_ENDPGM; the caller must delimit
    /// exactly one program (the source walker would silently ignore them).
    TrailingWords { words: usize },
    /// A direct branch targets a byte offset outside `[0, end]`.
    BranchOutsideProgram { target: i64 },
    /// The source TranslateProgram rejects Unknown and Fetch stages.
    InvalidStage,
    /// A direct branch targets a byte offset that is not an instruction start
    /// (a literal, modifier or body word).
    BranchIntoInstruction { target: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgramError {
    pub pc: u32,
    pub kind: ProgramErrorKind,
}

/// The source `CFG::FailureKind` values that this port can report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CfgErrorKind {
    InvalidBranchTarget,
    MissingFallthrough,
    InvalidLabel,
    /// A cyclic strongly connected component with several entries. The
    /// source falls back to a dispatcher loop, which is not ported.
    IrreducibleControlFlow,
    /// A structurization failure; the static text is the source reason.
    StructuredControlFlow(&'static str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CfgError {
    /// Start PC of the failing original block, when one is known.
    pub pc: Option<u32>,
    pub kind: CfgErrorKind,
}

/// Why an accepted instruction, modifier, stage feature or profile requirement
/// is refused instead of being approximated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedReason {
    /// The pinned translator has a lowering, but this port has none yet.
    NotPorted,
    /// A stage the source supports (mesh, local, tessellation) but this port does not.
    StageNotPorted,
    /// The pinned translator itself has no IR translation.
    NoSourceTranslation,
    DppModifier,
    SdwaSignExtendedDestination,
    IntegerResultModifier,
    Float16Operation,
    Float64Operation,
    Integer16Operation,
    PackedOperation,
    CrossLaneOperation,
    WholeQuadMode,
    SubvectorLoop,
    ProgramCounter,
    ConditionalDebugBranch,
    Trap,
    SetHardwareRegister,
    SendMessage,
    ThreadTrace,
    RealtimeClock,
    /// S_BARRIER outside a compute workgroup; the source drops it silently.
    BarrierOutsideWorkgroup,
    /// S_LOAD through a scalar address: an SRT or BDA path that is not ported.
    ScalarAddressLoad,
    /// A descriptor that is not four consecutive unmodified user SGPRs.
    DynamicDescriptor,
    MisalignedDescriptor,
    FormattedBuffer,
    TypedBuffer,
    SubdwordBuffer,
    BufferStatusReturn,
    BufferAtomic,
    FlatOrGlobalMemory,
    LocalDataShare,
    GlobalDataShare,
    ImageOperation,
    PerVertexInterpolation,
    InterpolationOnNonPixel,
    ExportTarget(u8),
    ExportOutsideGraphics,
    CompressedExport,
    PixelValidMask,
    ColorExportPacking,
    PositionAuxiliary,
    /// A live subgroup operation without an exact `SubgroupProfile`.
    RequiresExactSubgroup,
    /// The source routes shared selection arms through goto variables; that
    /// rewrite is not ported.
    GotoRouting,
    /// Indirect S_SETPC_B64 (dispatcher fallback in the source).
    IndirectBranch,
    /// A live value reached a lowering slot that has no SPIR-V form here.
    EmitterValue,
}

impl UnsupportedReason {
    pub fn message(self) -> &'static str {
        match self {
            Self::NotPorted => "the pinned source lowering is not ported by this crate",
            Self::StageNotPorted => "this shader stage is not ported",
            Self::NoSourceTranslation => "the pinned translator has no IR translation",
            Self::DppModifier => "DPP lane permutation modifiers are not ported",
            Self::SdwaSignExtendedDestination => "SDWA sign-extended destination is not ported",
            Self::IntegerResultModifier => "clamp/omod on an integer result is not lowered",
            Self::Float16Operation => "16-bit float operations are not ported",
            Self::Float64Operation => "64-bit float operations are not ported",
            Self::Integer16Operation => "16-bit integer operations are not ported",
            Self::PackedOperation => "packed VOP3P operations are not ported",
            Self::CrossLaneOperation => "cross-lane read/write/permute operations are not ported",
            Self::WholeQuadMode => "whole quad mode is not ported",
            Self::SubvectorLoop => "subvector loops are not ported",
            Self::ProgramCounter => "program counter reads and jumps are not ported",
            Self::ConditionalDebugBranch => "the source disables conditional debug branches",
            Self::Trap => "S_TRAP is not a no-op; the source drops it",
            Self::SetHardwareRegister => "S_SETREG changes hardware mode; the source drops it",
            Self::SendMessage => "S_SENDMSG side effects are not ported",
            Self::ThreadTrace => "thread trace data is not ported",
            Self::RealtimeClock => "S_MEMREALTIME needs a host clock",
            Self::BarrierOutsideWorkgroup => "S_BARRIER outside a compute workgroup",
            Self::ScalarAddressLoad => "scalar address loads (SRT/BDA) are not ported",
            Self::DynamicDescriptor => {
                "descriptor is not four consecutive unmodified user SGPRs (dynamic SRT/BDA)"
            }
            Self::MisalignedDescriptor => "buffer descriptor SGPR base is not 4-aligned",
            Self::FormattedBuffer => "formatted buffer access is not ported",
            Self::TypedBuffer => "typed (MTBUF) buffer access is not ported",
            Self::SubdwordBuffer => "8/16-bit buffer access is not ported",
            Self::BufferStatusReturn => "TFE/LWE fetch status is not ported",
            Self::BufferAtomic => "buffer atomics are not ported",
            Self::FlatOrGlobalMemory => "FLAT/GLOBAL/SCRATCH memory (BDA) is not ported",
            Self::LocalDataShare => "LDS (DS) operations are not ported",
            Self::GlobalDataShare => "GDS operations are not ported",
            Self::ImageOperation => "MIMG image operations are not ported",
            Self::PerVertexInterpolation => {
                "raw per-vertex interpolation reads (PerVertexKHR) are not ported"
            }
            Self::InterpolationOnNonPixel => "interpolation outside a pixel shader",
            Self::ExportTarget(_) => "export target is not supported for this stage",
            Self::ExportOutsideGraphics => "EXP outside a vertex or pixel shader",
            Self::CompressedExport => "compressed export is not ported",
            Self::PixelValidMask => "pixel valid-mask export and discard are not ported",
            Self::ColorExportPacking => "non-default color export mode is not ported",
            Self::PositionAuxiliary => "auxiliary position exports are not ported",
            Self::RequiresExactSubgroup => {
                "live subgroup operation requires an exact host subgroup profile"
            }
            Self::GotoRouting => "shared selection arm goto routing is not ported",
            Self::IndirectBranch => "indirect S_SETPC_B64 dispatch is not ported",
            Self::EmitterValue => "value has no SPIR-V lowering in this port",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Unsupported {
    pub pc: u32,
    pub opcode: Option<Opcode>,
    pub reason: UnsupportedReason,
}

/// Invalid caller-supplied stage/profile descriptions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileError {
    StageMismatch,
    UnsupportedStage,
    WaveSize,
    SubgroupSize,
    UserDataRange,
    PushDataStart,
    WorkgroupSize,
    ComputeThreadIds,
    ComputeWorkgroupRegisters,
    PixelInputCount,
    PixelInputLocation,
    PixelFlatAlias,
    PixelBarycentricRegister,
    PixelSystemRegister,
    MrtOutputMode,
    StartInstanceRegister,
    VertexRegisterOverlap,
    ResourceCount,
    /// Staged discovery requires an unspecialized buffer profile.
    PredeclaredBuffers,
    DuplicateResource,
    ResourceRegister,
    BufferStride,
    BufferIndexStride,
    NativeIndirectInstanceContract,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceErrorKind {
    /// A buffer is accessed through user SGPRs that the profile does not declare.
    MissingBuffer { user_sgpr: u8 },
    /// A declared buffer is never accessed (the profile must be exact).
    UnusedBuffer { user_sgpr: u8 },
    /// The program writes a buffer declared read-only.
    AccessMismatch { user_sgpr: u8 },
    /// The program reads a buffer declared write-only.
    ReadOfWriteOnly { user_sgpr: u8 },
    /// Descriptor dwords read from user SGPRs outside the declared user range.
    OutsideUserData { user_sgpr: u8 },
    /// The source's 64-buffer table is full.
    BufferLimit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceError {
    pub pc: Option<u32>,
    pub kind: ResourceErrorKind,
}

/// Stage interface violations (exports, interpolation and outputs).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterfaceErrorKind {
    /// V_INTERP reads a pixel input index the profile does not declare.
    PixelInputOutOfRange {
        input: u32,
    },
    InterpolationChannel {
        channel: u32,
    },
    /// An MRT export with no colour target under the profile's shader mask.
    MrtWithoutTarget {
        index: u32,
    },
    /// A vertex shader without a position export.
    MissingPosition,
    /// Two parameter exports with the same index carry different data paths
    /// are fine, but a vertex parameter index above the 32-entry table is not.
    ParameterOutOfRange {
        index: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterfaceError {
    pub pc: Option<u32>,
    pub kind: InterfaceErrorKind,
}

/// A tracked register read whose value is undefined on some path from entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Register {
    Sgpr(u8),
    Vgpr(u8),
    Scc,
    Vcc,
    VccLo,
    VccHi,
    Exec,
    ExecLo,
    ExecHi,
    M0,
    /// The per-lane mask predicate retained for an SGPR (source ThreadBit).
    SgprMask(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompileError {
    Decode(DecodeError),
    Program(ProgramError),
    Cfg(CfgError),
    Unsupported(Unsupported),
    Profile(ProfileError),
    Resource(ResourceError),
    Interface(InterfaceError),
    /// `UndefinedRegisterPolicy::Reject` found a live read of an undefined
    /// register. `pc` is the first instruction reading it.
    UndefinedRegister {
        register: Register,
        pc: u32,
    },
    LimitExceeded {
        kind: LimitKind,
        pc: Option<u32>,
    },
    Allocation,
    /// An internal invariant failed. Never expected; no partial output exists.
    Internal(&'static str),
}

impl CompileError {
    pub(crate) fn limit(kind: LimitKind) -> Self {
        Self::LimitExceeded { kind, pc: None }
    }

    pub(crate) fn unsupported(pc: u32, opcode: Option<Opcode>, reason: UnsupportedReason) -> Self {
        Self::Unsupported(Unsupported { pc, opcode, reason })
    }

    pub(crate) fn cfg(pc: Option<u32>, kind: CfgErrorKind) -> Self {
        Self::Cfg(CfgError { pc, kind })
    }

    pub(crate) fn resource(pc: Option<u32>, kind: ResourceErrorKind) -> Self {
        Self::Resource(ResourceError { pc, kind })
    }

    pub(crate) fn interface(pc: Option<u32>, kind: InterfaceErrorKind) -> Self {
        Self::Interface(InterfaceError { pc, kind })
    }

    /// Attach a PC to a limit failure that did not know it.
    pub(crate) fn at(self, at: u32) -> Self {
        match self {
            Self::LimitExceeded { kind, pc: None } => Self::LimitExceeded { kind, pc: Some(at) },
            other => other,
        }
    }
}

impl From<DecodeError> for CompileError {
    fn from(error: DecodeError) -> Self {
        Self::Decode(error)
    }
}

impl From<ProfileError> for CompileError {
    fn from(error: ProfileError) -> Self {
        Self::Profile(error)
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(error) => write!(f, "decode: {error}"),
            Self::Program(error) => write!(f, "program at pc 0x{:08x}: {:?}", error.pc, error.kind),
            Self::Cfg(error) => match error.pc {
                Some(pc) => write!(f, "CFG at pc 0x{pc:08x}: {:?}", error.kind),
                None => write!(f, "CFG: {:?}", error.kind),
            },
            Self::Unsupported(u) => write!(
                f,
                "unsupported {:?} at pc 0x{:08x}: {}",
                u.opcode,
                u.pc,
                u.reason.message()
            ),
            Self::Profile(error) => write!(f, "profile: {error:?}"),
            Self::Resource(error) => write!(f, "resource: {:?} (pc {:?})", error.kind, error.pc),
            Self::Interface(error) => write!(f, "interface: {:?} (pc {:?})", error.kind, error.pc),
            Self::UndefinedRegister { register, pc } => {
                write!(f, "undefined {register:?} read at pc 0x{pc:08x}")
            }
            Self::LimitExceeded { kind, pc } => write!(f, "{kind:?} limit exceeded (pc {pc:?})"),
            Self::Allocation => f.write_str("allocation failed"),
            Self::Internal(message) => write!(f, "internal compiler invariant: {message}"),
        }
    }
}

impl std::error::Error for CompileError {}
