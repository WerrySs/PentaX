// SPDX-License-Identifier: GPL-2.0-only

//! Owned generated-origin manifests. Image identities are compared, never dereferenced.
use super::*;
use pentax_linker::{FallbackPolicy, SymbolRef, SymbolicExtensionError, SymbolicExtensionLimits};

#[derive(Clone, Copy)]
struct Diagnostic {
    public_name: Option<&'static str>,
    reference_name: Option<&'static str>,
    reference_match: &'static str,
    reference_library: Option<&'static str>,
}
#[derive(Clone, Copy)]
struct Site {
    marker: UnavailableSite,
    diagnostic: Option<Diagnostic>,
}

/// Private generated-origin data copied before native publication. The worker
/// retains the immutable factory image; active source-plan borrows retain its
/// lifetime. Numeric image identity grants no access to a dangling pointer.
pub(in crate::process::hle::kernel_startup) struct PublishedUnavailable {
    image_address: usize,
    image_bytes: usize,
    index: usize,
    bias: u64,
    mapped_bytes: u64,
    sites: Vec<Site>,
}

impl PublishedUnavailable {
    /// The caller independently verifies the exact missing-import classification
    /// before publishing this descriptor. Only the actual factory is accepted;
    /// no caller-provided image/ELF name/marker pattern creates this authority.
    pub(in crate::process::hle::kernel_startup) fn prepare(
        factory: &GeneratedUnavailableProvider,
        index: usize,
        bias: u64,
        limits: &KernelStartupLimits,
        work: &mut u64,
    ) -> Result<Self, KernelStartupError> {
        let count = factory.function_count();
        let slots = count
            .checked_add(1)
            .ok_or(KernelStartupError::Budget("published stop sites"))?;
        require(
            (index as u64) < limits.startup.hle.linking.max_modules
                && count as u64 <= limits.startup.hle.linking.max_exports
                && bias.is_multiple_of(crate::memory::PAGE_SIZE)
                && factory.mapped_bytes() != 0
                && factory
                    .mapped_bytes()
                    .is_multiple_of(crate::memory::PAGE_SIZE)
                && factory.image().len() as u64 <= factory.mapped_bytes()
                && bias
                    .checked_add(factory.mapped_bytes())
                    .is_some_and(|end| end <= crate::memory::ADDRESS_LIMIT),
            "published stop placement",
        )?;
        let bytes = slots
            .checked_mul(std::mem::size_of::<Site>())
            .ok_or(KernelStartupError::Budget("published stop allocation"))?;
        let maximum_bytes = limits
            .max_provider_metadata_bytes
            .unwrap_or(limits.max_provider_work);
        // Preserve the diagnostic profile's original charged refusal order.
        // An independent interactive byte cap is admitted before allocation.
        if limits.max_provider_metadata_bytes.is_some() && bytes as u64 > maximum_bytes {
            return Err(KernelStartupError::Budget("published stop allocation"));
        }
        let admission = (count as u64)
            .checked_mul(super::super::public_names::DIAGNOSTIC_WORK)
            .and_then(|value| value.checked_add((slots as u64).checked_mul(128)?))
            .and_then(|value| value.checked_add(bytes as u64))
            .ok_or(KernelStartupError::Budget("published stop work"))?;
        charge(work, admission, limits.max_provider_work)?;
        let mut sites = Vec::<Site>::new();
        sites
            .try_reserve_exact(slots)
            .map_err(|_| KernelStartupError::Allocation)?;
        if sites
            .capacity()
            .checked_mul(std::mem::size_of::<Site>())
            .is_none_or(|actual| actual as u64 > maximum_bytes)
        {
            return Err(KernelStartupError::Budget("published stop allocation"));
        }
        for slot in 0..slots {
            let marker = factory
                .site(slot)
                .ok_or(KernelStartupError::InvalidProvider("published stop site"))?;
            let expected = marker.bytes();
            let at = usize::try_from(marker.rva)
                .map_err(|_| KernelStartupError::InvalidProvider("published stop extent"))?;
            require(
                at.checked_add(8)
                    .and_then(|end| factory.image().get(at..end))
                    == Some(&expected[..]),
                "published stop factory marker",
            )?;
            require(
                sites
                    .last()
                    .is_none_or(|previous| previous.marker.rva < marker.rva),
                "published stop site order",
            )?;
            let diagnostic = if let Some(ordinal) = marker.ordinal {
                require(ordinal as usize + 1 == slot, "published stop ordinal")?;
                let import = factory.function(ordinal as usize).ok_or(
                    KernelStartupError::InvalidProvider("published stop function"),
                )?;
                let public_name = super::super::public_names::name(import);
                let (reference_name, reference_match) = if public_name.is_some() {
                    (None, "exact")
                } else {
                    super::super::public_names::reference_hint(import)
                };
                Some(Diagnostic {
                    public_name,
                    reference_name,
                    reference_match,
                    reference_library: super::super::public_names::reference_library(import),
                })
            } else {
                require(slot == 0, "published stop exit ordinal")?;
                None
            };
            sites.push(Site { marker, diagnostic });
        }
        Ok(Self {
            image_address: factory.image().as_ptr() as usize,
            image_bytes: factory.image().len(),
            index,
            bias,
            mapped_bytes: factory.mapped_bytes(),
            sites,
        })
    }

    pub(in crate::process::hle::kernel_startup) const fn index(&self) -> usize {
        self.index
    }
    pub(in crate::process::hle::kernel_startup) const fn bias(&self) -> u64 {
        self.bias
    }
    pub(in crate::process::hle::kernel_startup) const fn mapped_bytes(&self) -> u64 {
        self.mapped_bytes
    }
    pub(in crate::process::hle::kernel_startup) fn function_count(&self) -> usize {
        self.sites.len() - 1
    }
    pub(in crate::process::hle::kernel_startup) fn contains(&self, address: u64) -> bool {
        address
            .checked_sub(self.bias)
            .is_some_and(|offset| offset < self.mapped_bytes())
    }
    pub(in crate::process::hle::kernel_startup) fn symbol_site(
        &self,
        symbol: usize,
    ) -> Option<UnavailableSite> {
        if symbol == 0 {
            None
        } else {
            self.sites.get(symbol).map(|site| site.marker)
        }
    }

    pub(in crate::process::hle::kernel_startup) fn validate_source(
        &self,
        source: &SymbolicLinkPlan<'_>,
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError> {
        charge(
            work,
            (source.patch_count() as u64)
                .checked_add(source.deferred_count() as u64)
                .and_then(|value| value.checked_add(self.sites.len() as u64 + 8))
                .ok_or(KernelStartupError::Budget("published stop source work"))?,
            maximum,
        )?;
        let module = source
            .modules()
            .get(self.index)
            .ok_or(KernelStartupError::InvalidProvider("published stop module"))?;
        let runtime = module.runtime_plan();
        require(
            module.id().index() == self.index
                && module.load_bias() == self.bias
                && module.source_bytes().as_ptr() as usize == self.image_address
                && module.source_bytes().len() == self.image_bytes
                && runtime.segments.len() == 1
                && runtime.tls.is_none()
                && runtime.relro.is_empty(),
            "published stop source identity",
        )?;
        let segment = &runtime.segments[0];
        require(
            segment.file_bytes.as_ptr() as usize == self.image_address
                && segment.file_bytes.len() == self.image_bytes
                && segment.vaddr == 0
                && segment.mem_size == self.mapped_bytes
                && segment.flags.raw() == 5,
            "published stop source mapping",
        )?;
        for record in source.deferred() {
            require(
                record.module().index() != self.index,
                "published stop deferred relocation",
            )?;
        }
        for patch in source.patches() {
            require(
                patch.consumer().index() != self.index,
                "published stop provider relocation",
            )?;
            let Resolution::Definition(reference) = patch.resolution() else {
                continue;
            };
            if reference.module.index() != self.index {
                continue;
            }
            let site = self
                .symbol_site(reference.symbol)
                .ok_or(KernelStartupError::InvalidProvider("published stop symbol"))?;
            require(
                matches!(patch.source().record.kind(), 1 | 6 | 7)
                    && (patch.source().record.kind() != 1 || patch.source().record.addend == 0)
                    && self.bias.checked_add(site.rva) == Some(patch.value()),
                "published stop function binding",
            )?;
        }
        Ok(())
    }

    fn attest_site(
        &self,
        memory: &PausedNativeMemory<'_>,
        marker: UnavailableSite,
    ) -> Result<(), KernelStartupError> {
        let address =
            self.bias
                .checked_add(marker.rva)
                .ok_or(KernelStartupError::InvalidProvider(
                    "published stop address",
                ))?;
        require(
            matches!(
                memory.preflight_write(address, 8),
                Err(NativeMemoryError::PermissionDenied(_))
            ) && matches!(
                memory.preflight_gpu_write(address, 8),
                Err(NativeMemoryError::PermissionDenied(_))
            ),
            "published stop immutable permissions",
        )?;
        let mut actual = [0; 8];
        memory
            .read(address, &mut actual)
            .map_err(|_| KernelStartupError::InvalidProvider("published stop read permission"))?;
        require(actual == marker.bytes(), "published stop live bytes")?;
        memory
            .fetch_source(address, &mut actual)
            .map_err(|_| KernelStartupError::InvalidProvider("published stop fetch permission"))?;
        require(actual == marker.bytes(), "published stop live fetch bytes")
    }

    pub(in crate::process::hle::kernel_startup) fn attest_all(
        &self,
        memory: &PausedNativeMemory<'_>,
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError> {
        charge(
            work,
            (self.sites.len() as u64)
                .checked_mul(48)
                .ok_or(KernelStartupError::Budget("published stop attestation"))?,
            maximum,
        )?;
        for site in &self.sites {
            self.attest_site(memory, site.marker)?;
        }
        Ok(())
    }

    pub(in crate::process::hle::kernel_startup) fn fallback_policy(
        &self,
        source: &SymbolicLinkPlan<'_>,
        limits: &SymbolicExtensionLimits,
    ) -> Result<FallbackPolicy, KernelStartupError> {
        let mut work = 0;
        self.validate_source(source, &mut work, limits.max_preservation_work)?;
        let visits = (self.function_count() as u64)
            .checked_mul(source.retired_fallbacks().len() as u64 + 128)
            .and_then(|value| {
                value.checked_add(
                    (self.function_count() as u64)
                        .checked_mul(std::mem::size_of::<SymbolRef>() as u64)?,
                )
            })
            .ok_or(KernelStartupError::Budget("published fallback work"))?;
        charge(&mut work, visits, limits.max_preservation_work)?;
        let module = &source.modules()[self.index];
        let mut allowed = Vec::new();
        allowed
            .try_reserve_exact(self.function_count())
            .map_err(|_| KernelStartupError::Allocation)?;
        if allowed
            .capacity()
            .checked_mul(std::mem::size_of::<SymbolRef>())
            .is_none_or(|bytes| bytes as u64 > limits.max_preservation_work)
        {
            return Err(KernelStartupError::Budget("published fallback allocation"));
        }
        for symbol in 1..self.sites.len() {
            let reference = SymbolRef {
                module: module.id(),
                symbol,
            };
            if !source.retired_fallbacks().contains(&reference) {
                allowed.push(reference);
            }
        }
        FallbackPolicy::new(source, module.id(), module.source_bytes(), &allowed, limits).map_err(
            |error| match error {
                SymbolicExtensionError::Allocation => KernelStartupError::Allocation,
                SymbolicExtensionError::Budget => {
                    KernelStartupError::Budget("published fallback linking")
                }
                SymbolicExtensionError::Symbolic(error) => {
                    KernelStartupError::Hle(HleError::Link(error))
                }
                _ => KernelStartupError::InvalidProvider("published fallback policy"),
            },
        )
    }

    fn stop(
        &self,
        memory: &PausedNativeMemory<'_>,
        request: NativeServiceRequest,
        work: &mut u64,
        maximum: u64,
    ) -> Result<u64, KernelStartupError> {
        charge(work, 48, maximum)?;
        if request.owner_id != memory.owner_id() {
            return Err(KernelStartupError::NativeOwnerMismatch);
        }
        if request.syscall_pc.checked_add(2) != Some(request.next_pc) {
            return Err(KernelStartupError::ContinuationMismatch {
                syscall_pc: request.syscall_pc,
                next_pc: request.next_pc,
            });
        }
        let site = request
            .syscall_pc
            .checked_sub(self.bias)
            .and_then(|offset| {
                self.sites
                    .binary_search_by_key(&offset, |site| site.marker.rva + 5)
                    .ok()
            })
            .and_then(|slot| self.sites.get(slot))
            .ok_or(KernelStartupError::SiteMismatch {
                syscall_pc: request.syscall_pc,
            })?;
        if request.service_id != u64::from(site.marker.service_id) {
            return Err(KernelStartupError::ServiceIdMismatch {
                expected: site.marker.service_id,
                actual: request.service_id,
            });
        }
        self.attest_site(memory, site.marker)?;
        match (site.marker.ordinal, site.diagnostic) {
            (Some(ordinal), Some(hint)) => Err(KernelStartupError::UnavailableImport {
                ordinal,
                public_name: hint.public_name,
                reference_name: hint.reference_name,
                reference_match: hint.reference_match,
                reference_library: hint.reference_library,
            }),
            (None, None) => Err(KernelStartupError::UnavailableExitCallback),
            _ => Err(KernelStartupError::InvalidProvider(
                "published stop diagnostic",
            )),
        }
    }
}

pub(in crate::process::hle::kernel_startup) fn validate_published(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    source: &SymbolicLinkPlan<'_>,
) -> Result<(), KernelStartupError> {
    let published = &session.loaded_modules.dynamic.published;
    let maximum = session.plan.limits.max_provider_work;
    require(
        published.len() <= source.modules().len(),
        "published stop source count",
    )?;
    charge(
        &mut session.provider_work,
        (published.len() as u64)
            .checked_mul(published.len() as u64 + 1)
            .ok_or(KernelStartupError::Budget("published stop ranges"))?,
        maximum,
    )?;
    for (index, origin) in published.iter().enumerate() {
        require(
            origin.index >= session.plan.link_plan().modules().len(),
            "published stop retained prefix",
        )?;
        for previous in &published[..index] {
            require(
                origin.index != previous.index
                    && !(origin.bias < previous.bias + previous.mapped_bytes
                        && previous.bias < origin.bias + origin.mapped_bytes),
                "published stop range overlap",
            )?;
        }
        origin.validate_source(source, &mut session.provider_work, maximum)?;
        let paused = session.hle.pause().map_err(KernelStartupError::Hle)?;
        origin.attest_all(&paused, &mut session.provider_work, maximum)?;
    }
    Ok(())
}

/// Range ownership is selected before interpreting an image-local service ID.
pub(in crate::process::hle::kernel_startup) fn route_stop(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    source: &SymbolicLinkPlan<'_>,
    request: NativeServiceRequest,
) -> Result<Option<u64>, KernelStartupError> {
    let published = &session.loaded_modules.dynamic.published;
    let maximum = session.plan.limits.max_provider_work;
    if published.is_empty() {
        return Ok(None);
    }
    charge(
        &mut session.provider_work,
        published.len() as u64 + 1,
        maximum,
    )?;
    let mut selected = None;
    for origin in published {
        if origin.contains(request.syscall_pc) {
            require(selected.is_none(), "published stop ambiguous range")?;
            selected = Some(origin);
        }
    }
    let Some(origin) = selected else {
        return Ok(None);
    };
    require(
        !session
            .plan
            .unavailable
            .is_some_and(|binding| binding.contains(request.syscall_pc))
            && !session
                .plan
                .graphics
                .is_some_and(|binding| binding.contains(request.syscall_pc))
            && !session
                .plan
                .json
                .is_some_and(|binding| binding.contains(request.syscall_pc)),
        "published stop foreign range",
    )?;
    origin.validate_source(source, &mut session.provider_work, maximum)?;
    let paused = session.hle.pause().map_err(KernelStartupError::Hle)?;
    origin
        .stop(&paused, request, &mut session.provider_work, maximum)
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::hle::{build_unavailable_provider, unavailable::tests::original_input};

    fn limits() -> KernelStartupLimits {
        KernelStartupLimits {
            max_provider_work: 1024 * 1024 * 1024,
            ..Default::default()
        }
    }

    #[test]
    fn published_origin_requires_exact_factory_image_extent_and_placement() {
        let bytes = original_input(1);
        let input = ModuleInput::new(open_executable(&bytes).unwrap(), 0x1000000);
        let factory = build_unavailable_provider(
            plan_missing_functions(std::slice::from_ref(&input), &Default::default(), 1).unwrap(),
            &Default::default(),
        )
        .unwrap();
        let bias = 0x2000000;
        let source = pentax_linker::plan_symbolic(
            &[
                input.clone(),
                ModuleInput::new(open_executable(factory.image()).unwrap(), bias),
            ],
            &Default::default(),
        )
        .unwrap();
        let mut work = 0;
        let origin =
            PublishedUnavailable::prepare(&factory, 1, bias, &limits(), &mut work).unwrap();
        origin
            .validate_source(&source, &mut work, limits().max_provider_work)
            .unwrap();
        assert_eq!(origin.index(), 1);
        assert_eq!(origin.function_count(), 1);
        assert_eq!(origin.bias(), bias);
        assert_eq!(origin.mapped_bytes(), factory.mapped_bytes());
        assert!(origin.contains(bias));
        assert!(origin.contains(bias + factory.mapped_bytes() - 1));
        assert!(!origin.contains(bias - 1));
        assert!(!origin.contains(bias + factory.mapped_bytes()));
        assert!(origin.symbol_site(0).is_none());
        assert_eq!(origin.symbol_site(1).unwrap().ordinal, Some(0));
        assert!(origin.symbol_site(2).is_none());
        let grant = origin
            .fallback_policy(&source, &Default::default())
            .unwrap();
        assert_eq!(grant.symbols().len(), 1);
        assert_eq!(grant.symbols()[0].module.index(), 1);
        let copied = factory.image().to_vec();
        let copied_source = pentax_linker::plan_symbolic(
            &[
                input.clone(),
                ModuleInput::new(open_executable(&copied).unwrap(), bias),
            ],
            &Default::default(),
        )
        .unwrap();
        assert!(
            origin
                .validate_source(&copied_source, &mut work, limits().max_provider_work)
                .is_err()
        );
        let moved_source = pentax_linker::plan_symbolic(
            &[
                input,
                ModuleInput::new(open_executable(factory.image()).unwrap(), bias + 0x10000),
            ],
            &Default::default(),
        )
        .unwrap();
        assert!(
            origin
                .validate_source(&moved_source, &mut work, limits().max_provider_work)
                .is_err()
        );
        let mut denied = limits();
        denied.max_provider_work = 0;
        assert!(matches!(
            PublishedUnavailable::prepare(&factory, 1, bias, &denied, &mut 0),
            Err(KernelStartupError::Budget(_))
        ));
        assert!(origin.validate_source(&source, &mut 0, 0).is_err());
        let mut lifetime = limits();
        lifetime.max_provider_work = u64::MAX;
        lifetime.max_provider_metadata_bytes =
            Some(((factory.function_count() + 1) * std::mem::size_of::<Site>()) as u64 - 1);
        let mut refused_work = 0;
        assert!(matches!(
            PublishedUnavailable::prepare(&factory, 1, bias, &lifetime, &mut refused_work),
            Err(KernelStartupError::Budget("published stop allocation"))
        ));
        assert_eq!(refused_work, 0); // Requested capacity refused before reserve.
        lifetime.max_provider_metadata_bytes = Some(limits().max_provider_work);
        let bounded = PublishedUnavailable::prepare(&factory, 1, bias, &lifetime, &mut 0).unwrap();
        assert_eq!(bounded.function_count(), origin.function_count());
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    #[test]
    fn published_stop_requires_actual_owner_exact_site_id_and_immutable_live_bytes() {
        use crate::memory::native::{NativeLoadLimits, prepare_in};
        use pentax_platform::native_memory::{
            NativeAddressSpace, NativeMemoryLimits, NativePermissions,
        };
        let bytes = original_input(1);
        let owner =
            NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x40000).unwrap();
        let base = owner.reserved_ranges().next().unwrap().address;
        let input = ModuleInput::new(open_executable(&bytes).unwrap(), base);
        let factory = build_unavailable_provider(
            plan_missing_functions(std::slice::from_ref(&input), &Default::default(), 1).unwrap(),
            &Default::default(),
        )
        .unwrap();
        let bias = base + 0x10000;
        let source = pentax_linker::plan_symbolic(
            &[
                input,
                ModuleInput::new(open_executable(factory.image()).unwrap(), bias),
            ],
            &Default::default(),
        )
        .unwrap();
        let mut memory = prepare_in(&source, &NativeLoadLimits::default(), Some(owner)).unwrap();
        let mut work = 0;
        let maximum = limits().max_provider_work;
        let origin =
            PublishedUnavailable::prepare(&factory, 1, bias, &limits(), &mut work).unwrap();
        origin
            .attest_all(&memory.pause().unwrap(), &mut work, maximum)
            .unwrap();
        let marker = origin.symbol_site(1).unwrap();
        let pc = bias + marker.rva + 5;
        let request = NativeServiceRequest {
            owner_id: memory.owner_id(),
            syscall_pc: pc,
            next_pc: pc + 2,
            service_id: u64::from(marker.service_id),
            args: [0; 6],
            vector_args: None,
            stack_pointer: None,
        };
        assert!(matches!(
            origin.stop(&memory.pause().unwrap(), request, &mut work, maximum),
            Err(KernelStartupError::UnavailableImport { ordinal: 0, .. })
        ));
        let wrong_id = NativeServiceRequest {
            service_id: request.service_id + 1,
            ..request
        };
        assert!(matches!(
            origin.stop(&memory.pause().unwrap(), wrong_id, &mut work, maximum),
            Err(KernelStartupError::ServiceIdMismatch { .. })
        ));
        let wrong_site = NativeServiceRequest {
            syscall_pc: pc + 1,
            next_pc: pc + 3,
            ..request
        };
        assert!(matches!(
            origin.stop(&memory.pause().unwrap(), wrong_site, &mut work, maximum),
            Err(KernelStartupError::SiteMismatch { .. })
        ));
        let exit = origin.sites[0].marker;
        let exit_request = NativeServiceRequest {
            syscall_pc: bias + exit.rva + 5,
            next_pc: bias + exit.rva + 7,
            service_id: u64::from(exit.service_id),
            ..request
        };
        assert!(matches!(
            origin.stop(&memory.pause().unwrap(), exit_request, &mut work, maximum),
            Err(KernelStartupError::UnavailableExitCallback)
        ));
        let foreign =
            NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x10000).unwrap();
        let wrong_owner = NativeServiceRequest {
            owner_id: foreign.owner_id(),
            ..request
        };
        assert_eq!(
            origin.stop(&memory.pause().unwrap(), wrong_owner, &mut work, maximum),
            Err(KernelStartupError::NativeOwnerMismatch)
        );
        memory
            .space
            .protect(bias, factory.mapped_bytes(), NativePermissions::READ_WRITE)
            .unwrap();
        assert!(
            origin
                .attest_all(&memory.pause().unwrap(), &mut work, maximum)
                .is_err()
        );
        memory
            .pause()
            .unwrap()
            .write(bias + marker.rva, &[0x90])
            .unwrap();
        memory
            .space
            .protect(
                bias,
                factory.mapped_bytes(),
                NativePermissions::READ_EXECUTE,
            )
            .unwrap();
        assert!(
            origin
                .attest_all(&memory.pause().unwrap(), &mut work, maximum)
                .is_err()
        );
        memory
            .space
            .protect(bias, factory.mapped_bytes(), NativePermissions::READ_WRITE)
            .unwrap();
        memory
            .pause()
            .unwrap()
            .write(bias + marker.rva, &marker.bytes())
            .unwrap();
        memory
            .space
            .protect(
                bias,
                factory.mapped_bytes(),
                NativePermissions::READ_EXECUTE.with_gpu(false, true),
            )
            .unwrap();
        assert!(
            origin
                .attest_all(&memory.pause().unwrap(), &mut work, maximum)
                .is_err()
        );
        memory
            .space
            .protect(bias, factory.mapped_bytes(), NativePermissions::READ)
            .unwrap();
        assert!(
            origin
                .attest_all(&memory.pause().unwrap(), &mut work, maximum)
                .is_err()
        );
        memory
            .space
            .protect(
                bias,
                factory.mapped_bytes(),
                NativePermissions::READ_EXECUTE,
            )
            .unwrap();
        origin
            .attest_all(&memory.pause().unwrap(), &mut work, maximum)
            .unwrap();
        assert!(
            origin
                .attest_all(&memory.pause().unwrap(), &mut 0, 0)
                .is_err()
        );
    }
}
