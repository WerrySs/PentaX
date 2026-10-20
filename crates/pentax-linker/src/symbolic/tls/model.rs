// SPDX-License-Identifier: GPL-2.0-or-later

use super::super::{ModuleId, SessionDeferredRelocation, SymbolRef, SymbolicLinkPlan};
use crate::RelocationSource;
use std::fmt;

/// Only these LP64 AMD64 TLS relocation encodings are admitted by this binder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsRelocationKind {
    DtpMod64,
    DtpOff64,
    TpOff64,
}

impl TlsRelocationKind {
    pub(super) const fn from_code(code: u32) -> Option<Self> {
        match code {
            16 => Some(Self::DtpMod64),
            17 => Some(Self::DtpOff64),
            18 => Some(Self::TpOff64),
            _ => None,
        }
    }
}

/// Explicit generic arithmetic and exact declaration policy, not a PS5 ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsBindingPolicy {
    GenericAmd64ExactDeclarationV1,
}

/// A module-template offset; never an ordinary or biased guest address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsOffset(pub(super) u64);

impl TlsOffset {
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The selected symbol or local module and its retained PT_TLS identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsDefinition {
    pub(super) module: ModuleId,
    pub(super) symbol: Option<SymbolRef>,
    pub(super) offset: TlsOffset,
    pub(super) size: u64,
    pub(super) program_header_index: usize,
    pub(super) template_mem_size: u64,
}

impl TlsDefinition {
    /// Module owning the retained TLS template, independent of symbol lookup.
    pub const fn module(&self) -> ModuleId {
        self.module
    }
    /// None denotes the local-module DTPMOD64 form, not an invented TLS symbol.
    pub const fn symbol(&self) -> Option<SymbolRef> {
        self.symbol
    }
    pub const fn offset(&self) -> TlsOffset {
        self.offset
    }
    pub const fn size(&self) -> u64 {
        self.size
    }
    pub const fn program_header_index(&self) -> usize {
        self.program_header_index
    }
    pub const fn template_mem_size(&self) -> u64 {
        self.template_mem_size
    }
}

/// A source-bound eight-byte target with typed identity, without a numeric value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsBinding {
    pub(super) deferred_index: usize,
    pub(super) consumer: ModuleId,
    pub(super) source: RelocationSource,
    pub(super) address: u64,
    pub(super) kind: TlsRelocationKind,
    pub(super) definition: TlsDefinition,
}

impl TlsBinding {
    pub const fn deferred_index(&self) -> usize {
        self.deferred_index
    }
    pub const fn consumer(&self) -> ModuleId {
        self.consumer
    }
    pub const fn source(&self) -> RelocationSource {
        self.source
    }
    pub const fn address(&self) -> u64 {
        self.address
    }
    pub const fn kind(&self) -> TlsRelocationKind {
        self.kind
    }
    pub const fn definition(&self) -> TlsDefinition {
        self.definition
    }
}

/// Aggregate additional work bounds, independently enforced for this operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsBindingLimits {
    pub max_modules: u64,
    /// All retained RELA/PLT records, including NONE and ordinary supported kinds.
    pub max_records: u64,
    pub max_bindings: u64,
    pub max_exports: u64,
    pub max_key_bytes: u64,
    pub max_lookup_steps: u64,
    /// Module/symbol/record/range visits and ordered-index/merge operations.
    /// Resolver descriptor/key/candidate/cache work has its separate bound above.
    pub max_work_steps: u64,
}

impl Default for TlsBindingLimits {
    fn default() -> Self {
        Self {
            max_modules: 128,
            max_records: 1_000_000,
            max_bindings: 1_000_000,
            max_exports: 1_000_000,
            max_key_bytes: 64 * 1024 * 1024,
            max_lookup_steps: 16_000_000,
            max_work_steps: 16_000_000,
        }
    }
}

/// Immutable TLS identities derived from exactly one retained ordinary plan.
/// No runtime TLS IDs, TP layout, memory writes or execution are authorized.
pub struct TlsBindingPlan<'plan, 'image> {
    pub(super) base: &'plan SymbolicLinkPlan<'image>,
    pub(super) bindings: Vec<TlsBinding>,
    pub(super) remaining: Vec<SessionDeferredRelocation>,
    pub(super) _limits: TlsBindingLimits,
}

impl<'plan, 'image> TlsBindingPlan<'plan, 'image> {
    pub const fn policy(&self) -> TlsBindingPolicy {
        TlsBindingPolicy::GenericAmd64ExactDeclarationV1
    }
    pub const fn base_plan(&self) -> &'plan SymbolicLinkPlan<'image> {
        self.base
    }
    /// Nonoverlapping targets in biased-address order, also disjoint from the
    /// exact base plan's ordinary patches. No target-width claim covers remaining records.
    pub fn bindings(&self) -> &[TlsBinding] {
        &self.bindings
    }
    /// Unrecognized records in the base's original deferred order.
    pub fn remaining_deferred(&self) -> &[SessionDeferredRelocation] {
        &self.remaining
    }
}

impl fmt::Debug for TlsBindingPlan<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsBindingPlan")
            .field("policy", &self.policy())
            .field("module_count", &self.base.modules().len())
            .field("binding_count", &self.bindings.len())
            .field("remaining_count", &self.remaining.len())
            .finish_non_exhaustive()
    }
}
