// SPDX-License-Identifier: GPL-2.0-only

//! Exact unavailable-provider classification, source attestation and typed stops.

use super::{
    KernelStartupError, KernelStartupLimits, NativeServiceRequest, PreparedNativeKernelStartup,
    charge, require,
};
use crate::process::hle::unavailable::UnavailableSite;
use crate::process::hle::{
    GeneratedProviders, GeneratedUnavailableProvider, HleError, ProviderPlacement,
};
use pentax_linker::{ModuleInput, Resolution, SymbolicLinkPlan, plan_missing_functions};
use pentax_loader::open_executable;
use pentax_platform::native_memory::{NativeMemoryError, PausedNativeMemory};

pub(super) mod published;

/// An explicit unavailable factory and guest placement, not a registration or
/// permission to bypass linking. The ordinary strict resolver remains final.
#[derive(Debug, Clone, Copy)]
pub struct UnavailableProviderPlacement<'image> {
    pub provider: &'image GeneratedUnavailableProvider,
    pub placement: ProviderPlacement,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct UnavailableBinding<'image> {
    pub factory: &'image GeneratedUnavailableProvider,
    pub index: usize,
    pub bias: u64,
}
impl UnavailableBinding<'_> {
    pub fn contains(&self, pc: u64) -> bool {
        pc.checked_sub(self.bias)
            .is_some_and(|offset| offset < self.factory.mapped_bytes())
    }
}

pub(super) fn append_classified<'image>(
    supplied: &mut Vec<ModuleInput<'image>>,
    libc: &'image GeneratedProviders,
    libc_placement: ProviderPlacement,
    placement: UnavailableProviderPlacement<'image>,
    limits: &KernelStartupLimits,
    work: &mut u64,
) -> Result<UnavailableBinding<'image>, KernelStartupError> {
    let bias = placement.placement.load_bias;
    require(
        bias.is_multiple_of(crate::memory::PAGE_SIZE)
            && bias
                .checked_add(placement.provider.mapped_bytes())
                .is_some_and(|end| end <= crate::memory::ADDRESS_LIMIT),
        "unavailable provider placement",
    )?;
    // The caller reserved capacity for this temporary real-libc view and the
    // eventual unavailable image before any push. No per-missing retry loop.
    let libc_view =
        open_executable(libc.image()).map_err(|e| KernelStartupError::Hle(HleError::Load(e)))?;
    supplied.push(ModuleInput::new(libc_view, libc_placement.load_bias));
    let expected = plan_missing_functions(
        supplied,
        &limits.startup.hle.linking,
        placement.provider.function_count() as u64,
    );
    supplied.pop();
    let expected = expected.map_err(|e| KernelStartupError::Hle(HleError::Link(e)))?;
    require(
        expected.functions().len() == placement.provider.function_count(),
        "unavailable classification count",
    )?;
    for (index, expected) in expected.functions().iter().enumerate() {
        let actual =
            placement
                .provider
                .function(index)
                .ok_or(KernelStartupError::InvalidProvider(
                    "unavailable classification index",
                ))?;
        let bytes = expected.module_name().len() as u64 + expected.library_name().len() as u64;
        charge(
            work,
            bytes
                .checked_mul(2)
                .and_then(|value| value.checked_add(64))
                .ok_or(KernelStartupError::Budget("unavailable classification"))?,
            limits.max_provider_work,
        )?;
        require(
            expected.module_name() == actual.module_name()
                && expected.library_name() == actual.library_name()
                && expected.versions() == actual.versions()
                && expected.nid() == actual.nid()
                && expected.reference_count() == actual.reference_count()
                && expected.first_consumer() == actual.first_consumer(),
            "unavailable classification identity",
        )?;
    }
    let index = supplied.len();
    let view = open_executable(placement.provider.image())
        .map_err(|e| KernelStartupError::Hle(HleError::Load(e)))?;
    supplied.push(ModuleInput::new(view, bias));
    Ok(UnavailableBinding {
        factory: placement.provider,
        index,
        bias,
    })
}

pub(super) fn validate_source_and_bindings(
    plan: &SymbolicLinkPlan<'_>,
    binding: &UnavailableBinding<'_>,
    limits: &KernelStartupLimits,
    work: &mut u64,
) -> Result<(), KernelStartupError> {
    charge(
        work,
        plan.patch_count() as u64 + 4,
        limits.max_provider_work,
    )?;
    let module = plan
        .modules()
        .get(binding.index)
        .ok_or(KernelStartupError::InvalidProvider(
            "unavailable module index",
        ))?;
    let runtime = module.runtime_plan();
    require(
        module.load_bias() == binding.bias
            && runtime.segments.len() == 1
            && runtime.tls.is_none()
            && runtime.relro.is_empty(),
        "unavailable module mapping",
    )?;
    let segment = &runtime.segments[0];
    require(
        std::ptr::eq(segment.file_bytes, binding.factory.image())
            && segment.vaddr == 0
            && segment.mem_size == binding.factory.mapped_bytes()
            && segment.flags.raw() == 5,
        "unavailable factory source identity",
    )?;
    for patch in plan.patches() {
        require(
            patch.consumer().index() != binding.index,
            "unavailable provider relocation",
        )?;
        let Resolution::Definition(symbol) = patch.resolution() else {
            continue;
        };
        if symbol.module.index() != binding.index {
            continue;
        }
        let site = binding.factory.symbol_site(symbol.symbol).ok_or(
            KernelStartupError::InvalidProvider("unavailable binding symbol"),
        )?;
        require(
            (patch.source().record.kind() != 1 || patch.source().record.addend == 0)
                && patch.value() == binding.bias + site.rva,
            "unavailable function binding",
        )?;
    }
    Ok(())
}

fn attest_site(
    memory: &PausedNativeMemory<'_>,
    binding: &UnavailableBinding<'_>,
    site: UnavailableSite,
) -> Result<(), KernelStartupError> {
    let expected = site.bytes();
    let address = binding.bias + site.rva;
    require(
        binding
            .factory
            .image()
            .get(site.rva as usize..site.rva as usize + 8)
            == Some(&expected),
        "unavailable source marker",
    )?;
    require(
        matches!(
            memory.preflight_write(address, 8),
            Err(NativeMemoryError::PermissionDenied(_))
        ),
        "unavailable marker lost immutable permissions",
    )?;
    let mut actual = [0; 8];
    memory
        .read(address, &mut actual)
        .map_err(|_| KernelStartupError::InvalidProvider("unavailable marker read permission"))?;
    memory
        .fetch_source(address, &mut actual)
        .map_err(|_| KernelStartupError::InvalidProvider("unavailable marker fetch permission"))?;
    require(actual == expected, "unavailable live marker bytes")
}

pub(super) fn attest_all(
    memory: &PausedNativeMemory<'_>,
    binding: &UnavailableBinding<'_>,
    work: &mut u64,
    maximum: u64,
) -> Result<(), KernelStartupError> {
    charge(
        work,
        (binding.factory.function_count() as u64 + 1) * 40,
        maximum,
    )?;
    for slot in 0..=binding.factory.function_count() {
        let site = binding
            .factory
            .site(slot)
            .ok_or(KernelStartupError::InvalidProvider(
                "unavailable marker index",
            ))?;
        attest_site(memory, binding, site)?;
    }
    Ok(())
}

pub(super) fn stop(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    request: NativeServiceRequest,
) -> Result<u64, KernelStartupError> {
    charge(
        &mut session.provider_work,
        40,
        session.plan.limits.max_provider_work,
    )?;
    let binding = session
        .plan
        .unavailable
        .ok_or(KernelStartupError::InvalidProvider(
            "missing unavailable provider",
        ))?;
    let site = binding
        .factory
        .syscall_site(binding.bias, request.syscall_pc)
        .ok_or(KernelStartupError::SiteMismatch {
            syscall_pc: request.syscall_pc,
        })?;
    if request.service_id != u64::from(site.service_id) {
        return Err(KernelStartupError::ServiceIdMismatch {
            expected: site.service_id,
            actual: request.service_id,
        });
    }
    attest_site(&session.pause()?, &binding, site)?;
    match site.ordinal {
        Some(ordinal) => {
            // Bounded lookup over pinned public registration names only. A
            // miss never echoes an input identifier and never supplies a call.
            charge(
                &mut session.provider_work,
                super::public_names::DIAGNOSTIC_WORK,
                session.plan.limits.max_provider_work,
            )?;
            let import = binding.factory.function(ordinal as usize).ok_or(
                KernelStartupError::InvalidProvider("unavailable function index"),
            )?;
            let public_name = super::public_names::name(import);
            let (reference_name, reference_match) = if public_name.is_some() {
                (None, "exact")
            } else {
                super::public_names::reference_hint(import)
            };
            Err(KernelStartupError::UnavailableImport {
                ordinal,
                public_name,
                reference_name,
                reference_match,
                reference_library: super::public_names::reference_library(import),
            })
        }
        None => Err(KernelStartupError::UnavailableExitCallback),
    }
}
