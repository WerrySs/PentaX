// SPDX-License-Identifier: GPL-2.0-or-later

use super::super::{ModuleId, SymbolRef, SymbolicError};
use crate::{RelocationError, RelocationSource};
use std::{error::Error, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsBindingBudget {
    Modules,
    Records,
    Bindings,
    WorkSteps,
}

/// Unsupported profile encodings are distinct from malformed ELF claims.
/// No failure returns a partially bound plan or changes source bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TlsBindingError {
    Budget {
        kind: TlsBindingBudget,
        limit: u64,
    },
    Symbolic(SymbolicError),
    Resolution {
        consumer: ModuleId,
        source: RelocationSource,
        error: Box<SymbolicError>,
    },
    Relocation {
        consumer: ModuleId,
        source: RelocationSource,
        error: Box<RelocationError>,
    },
    UnsupportedSymbolZero {
        consumer: ModuleId,
        source: RelocationSource,
    },
    UnsupportedModuleAddend {
        consumer: ModuleId,
        source: RelocationSource,
    },
    MissingLocalTlsTemplate {
        consumer: ModuleId,
        source: RelocationSource,
    },
    MissingTlsTemplate {
        consumer: ModuleId,
        source: RelocationSource,
        definition: SymbolRef,
    },
    TlsSymbolRange {
        consumer: ModuleId,
        source: RelocationSource,
        definition: SymbolRef,
    },
}

impl fmt::Display for TlsBindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TLS binding rejected: {self:?}")
    }
}

impl Error for TlsBindingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Symbolic(error) => Some(error),
            Self::Resolution { error, .. } => Some(error.as_ref()),
            Self::Relocation { error, .. } => Some(error.as_ref()),
            _ => None,
        }
    }
}

pub(super) fn charge(
    used: &mut u64,
    amount: u64,
    limit: u64,
    kind: TlsBindingBudget,
) -> Result<(), TlsBindingError> {
    *used = used
        .checked_add(amount)
        .filter(|value| *value <= limit)
        .ok_or(TlsBindingError::Budget { kind, limit })?;
    Ok(())
}
