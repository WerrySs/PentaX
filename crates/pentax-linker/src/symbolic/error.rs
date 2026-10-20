// SPDX-License-Identifier: GPL-2.0-or-later

use super::{DescriptorVersions, ModuleId, SymbolRef};
use crate::{
    DescriptorRole, LinkError, QualifiedNameError, RelocationError, RelocationSource, SceError,
};
use std::{error::Error, fmt};

/// Aggregate resource whose inclusive budget was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolicBudget {
    Modules,
    DynamicEntries,
    DescriptorRecords,
    TableBytes,
    Symbols,
    Relocations,
    Exports,
    Patches,
    KeyBytes,
    LookupSteps,
    MissingFunctions,
}

/// A missing reference that cannot be represented by a callable unavailable
/// function. These refusals never become object storage, nulls or fake success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingFunctionReason {
    SymbolType,
    WeakBinding,
    AddressAddend,
}

/// A symbol category deliberately outside this first resolution policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedSymbol {
    Binding,
    Visibility,
    OtherBits,
    Type,
    Section,
    PreemptibleDefinition,
    WeakProvider,
    AbsoluteFunction,
    TypeMismatch,
}

/// No partial plan is returned for any failure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SymbolicError {
    EmptySession,
    Allocation,
    SourceIdentityExhausted,
    LimitExceeded {
        budget: SymbolicBudget,
        requested: u64,
        maximum: u64,
    },
    SizeOverflow {
        budget: SymbolicBudget,
    },
    Link {
        module: ModuleId,
        error: LinkError,
    },
    Sce {
        module: ModuleId,
        error: SceError,
    },
    Relocation {
        module: ModuleId,
        error: RelocationError,
    },
    UnsupportedDescriptors {
        module: ModuleId,
    },
    MissingDescriptors {
        symbol: SymbolRef,
    },
    QualifiedName {
        symbol: SymbolRef,
        error: QualifiedNameError,
    },
    MissingDeclaration {
        symbol: SymbolRef,
        role: DescriptorRole,
        selector: u16,
    },
    MissingProvider {
        consumer: SymbolRef,
    },
    UnsupportedMissingFunction {
        consumer: SymbolRef,
        reason: MissingFunctionReason,
    },
    /// An exact name/NID identity exists at another version. `provider` is
    /// the first candidate in version order, not a fallback binding or an
    /// exhaustive list. Type is evaluated after an exact-version lookup;
    /// an alternate-version record may also have an unsupported type.
    VersionMismatch {
        consumer: SymbolRef,
        requested: DescriptorVersions,
        available: DescriptorVersions,
        provider: SymbolRef,
    },
    AmbiguousProvider {
        consumer: SymbolRef,
        first: SymbolRef,
        second: SymbolRef,
    },
    UnsupportedSymbol {
        symbol: SymbolRef,
        reason: UnsupportedSymbol,
    },
    SymbolRange {
        symbol: SymbolRef,
    },
    SymbolAddressOverflow {
        symbol: SymbolRef,
    },
    FunctionNotExecutable {
        symbol: SymbolRef,
    },
    BiasedMappingOverflow {
        module: ModuleId,
        segment: usize,
    },
    BiasedRelroOverflow {
        module: ModuleId,
        range: usize,
    },
    OverlappingModulePages {
        first: ModuleId,
        second: ModuleId,
        address: u64,
    },
    PrimaryEntryOutsideExecutable {
        address: u64,
    },
    PrimaryEntryOverflow,
    UnsupportedComposition {
        module: ModuleId,
        first: RelocationSource,
        second: RelocationSource,
    },
    OverlappingPatches {
        first_module: ModuleId,
        first: RelocationSource,
        second_module: ModuleId,
        second: RelocationSource,
        address: u64,
    },
}

impl fmt::Display for SymbolicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySession => f.write_str("symbolic session has no primary image"),
            Self::Link { module, error } => write!(f, "module {}: {error}", module.index()),
            Self::Sce { module, error } => write!(f, "module {}: {error}", module.index()),
            Self::Relocation { module, error } => write!(f, "module {}: {error}", module.index()),
            // Typed fields carry bounded original provenance without copying names into diagnostics.
            other => write!(f, "symbolic session rejected: {other:?}"),
        }
    }
}

impl Error for SymbolicError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Link { error, .. } => Some(error),
            Self::Sce { error, .. } => Some(error),
            Self::Relocation { error, .. } => Some(error),
            Self::QualifiedName { error, .. } => Some(error),
            _ => None,
        }
    }
}

pub(super) fn charge(
    used: &mut u64,
    amount: u64,
    maximum: u64,
    budget: SymbolicBudget,
) -> Result<(), SymbolicError> {
    let requested = used
        .checked_add(amount)
        .ok_or(SymbolicError::SizeOverflow { budget })?;
    if requested > maximum {
        return Err(SymbolicError::LimitExceeded {
            budget,
            requested,
            maximum,
        });
    }
    *used = requested;
    Ok(())
}
