// SPDX-License-Identifier: GPL-2.0-only

//! Opt-in lookup within one retained module, distinct from import resolution.
use super::{SymbolRef, SymbolicError, SymbolicLimits, SymbolicLinkPlan, resolve::Resolver};
use crate::{NidToken, SymbolType};
use std::fmt;

#[derive(Debug, Clone, Copy)]
pub struct ModuleLookupLimits {
    /// Reuse export/key/index-work bounds only. The retained immutable source
    /// was already inspected under its original complete symbolic limits.
    pub symbolic: SymbolicLimits,
    /// Independently bounded query bytes, with an absolute 1 MiB cap.
    pub max_name_bytes: usize,
    pub max_work: u64,
}
impl Default for ModuleLookupLimits {
    fn default() -> Self {
        Self {
            symbolic: Default::default(),
            max_name_bytes: 1024,
            max_work: 16_000_000,
        }
    }
}

#[derive(Debug)]
pub enum ModuleLookupError {
    Symbolic(SymbolicError),
    InvalidModule,
    InvalidName,
    FormattedNameUnsupported,
    Absolute,
    Ambiguous { first: SymbolRef, second: SymbolRef },
    Budget,
}
impl From<SymbolicError> for ModuleLookupError {
    fn from(value: SymbolicError) -> Self {
        Self::Symbolic(value)
    }
}
impl fmt::Display for ModuleLookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("module export lookup refused")
    }
}
impl std::error::Error for ModuleLookupError {}

/// Metadata selected from the retained plan, not live execution authority.
pub struct ModuleExport<'plan, 'image> {
    source: &'plan SymbolicLinkPlan<'image>,
    definition: SymbolRef,
    address: u64,
}
impl<'plan, 'image> ModuleExport<'plan, 'image> {
    pub const fn source_plan(&self) -> &'plan SymbolicLinkPlan<'image> {
        self.source
    }
    pub const fn definition(&self) -> SymbolRef {
        self.definition
    }
    pub const fn address(&self) -> u64 {
        self.address
    }
    pub fn size(&self) -> u64 {
        self.symbol().size
    }
    pub fn symbol_type(&self) -> SymbolType {
        self.symbol().symbol_type()
    }
    fn symbol(&self) -> &crate::Symbol {
        &self.source.modules[self.definition.module.index()]
            .metadata
            .as_ref()
            .expect("selected export")
            .link()
            .symbols()[self.definition.symbol]
    }
}
impl fmt::Debug for ModuleExport<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModuleExport")
            .field("definition", &self.definition)
            .finish_non_exhaustive()
    }
}

/// Attempted work is available on refusal too, for a cumulative runtime owner.
pub struct ModuleLookupAttempt<'plan, 'image> {
    pub result: Result<Option<ModuleExport<'plan, 'image>>, ModuleLookupError>,
    pub work: u64,
}

/// Pinned name-hash, then canonical raw-NID fallback, scoped to ONE module's
/// actual offered exports. FUNC precedes OBJECT. Unsupported types, weak/absolute
/// definitions and ambiguity stay errors. Formatted database prefixes refuse.
/// Ordinary exact import resolution and its work policy remain unchanged.
pub fn lookup_module_export<'plan, 'image>(
    source: &'plan SymbolicLinkPlan<'image>,
    module: usize,
    name: &[u8],
    limits: &ModuleLookupLimits,
) -> ModuleLookupAttempt<'plan, 'image> {
    let mut symbolic = limits.symbolic;
    symbolic.max_lookup_steps = symbolic.max_lookup_steps.min(limits.max_work);
    let mut resolver = Resolver::empty(source.modules(), (&symbolic).into(), true);
    let result = (|| {
        if module >= source.modules().len() {
            return Err(ModuleLookupError::InvalidModule);
        }
        if name.len() > limits.max_name_bytes || name.len() > 1024 * 1024 {
            return Err(ModuleLookupError::Budget);
        }
        let hash_work = (name.len() as u64)
            .checked_add(25)
            .map(|size| size.div_ceil(64))
            .and_then(|blocks| blocks.checked_mul(256))
            .and_then(|steps| {
                steps.checked_add((name.len() as u64).checked_mul(3)?.checked_add(4)?)
            })
            .ok_or(ModuleLookupError::Budget)?;
        resolver.steps(hash_work)?;
        if name.contains(&0) {
            return Err(ModuleLookupError::InvalidName);
        }
        if name.contains(&b'[') || name.contains(&b']') {
            return Err(ModuleLookupError::FormattedNameUnsupported);
        }
        let hashed = super::name_nid::for_name(name);
        resolver.populate(Some(module), source.retired_fallbacks())?;
        let mut selected = resolver.module_export(hashed)?;
        if selected.is_none()
            && let Ok(raw) = NidToken::parse(name)
        {
            selected = resolver.module_export(raw)?;
        }
        Ok(selected.map(|(definition, address)| ModuleExport {
            source,
            definition,
            address,
        }))
    })();
    ModuleLookupAttempt {
        result,
        work: resolver.work_used(),
    }
}
