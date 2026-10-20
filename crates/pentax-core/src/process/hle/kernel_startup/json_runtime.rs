// SPDX-License-Identifier: GPL-2.0-only

//! Source-bound JSON constructor writes and explicit unsupported virtual calls.
use super::{
    KernelStartupError, KernelStartupLimits, NativeServiceRequest, PreparedNativeKernelStartup,
    charge, require,
};
use crate::{
    memory::PAGE_SIZE,
    process::hle::{
        GeneratedJsonProvider, HleError, ProviderPlacement,
        json_provider::{CODE_FILE_BYTES, JsonFunction},
    },
};
use pentax_linker::{ModuleInput, Resolution, SymbolicLinkPlan};
use pentax_loader::open_executable;
use pentax_platform::native_memory::{NativeMemoryError, PausedNativeMemory};

#[derive(Clone, Copy, Debug)]
pub struct JsonStartupOptions<'image> {
    pub provider: &'image GeneratedJsonProvider,
    pub placement: ProviderPlacement,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct JsonBinding<'image> {
    pub factory: &'image GeneratedJsonProvider,
    pub index: usize,
    pub bias: u64,
}
impl JsonBinding<'_> {
    pub fn contains(&self, pc: u64) -> bool {
        pc.checked_sub(self.bias)
            .is_some_and(|rva| rva < self.factory.mapped_bytes())
    }
}

pub(super) fn append<'image>(
    inputs: &mut Vec<ModuleInput<'image>>,
    options: JsonStartupOptions<'image>,
    limits: &KernelStartupLimits,
) -> Result<JsonBinding<'image>, KernelStartupError> {
    require(
        options.placement.load_bias == options.provider.load_bias(),
        "JSON factory placement",
    )?;
    if options.provider.image().len() as u64 > limits.startup.hle.max_provider_bytes {
        return Err(KernelStartupError::Budget("JSON provider bytes"));
    }
    let binding = JsonBinding {
        factory: options.provider,
        index: inputs.len(),
        bias: options.placement.load_bias,
    };
    let view = open_executable(binding.factory.image())
        .map_err(|e| KernelStartupError::Hle(HleError::Load(e)))?;
    inputs.push(ModuleInput::new(view, binding.bias));
    Ok(binding)
}

fn validate_source(
    plan: &SymbolicLinkPlan<'_>,
    binding: &JsonBinding<'_>,
    limits: &KernelStartupLimits,
    work: &mut u64,
) -> Result<(), KernelStartupError> {
    charge(work, 3, limits.max_provider_work)?;
    let module = plan
        .modules()
        .get(binding.index)
        .ok_or(KernelStartupError::InvalidProvider("JSON module index"))?;
    let runtime = module.runtime_plan();
    require(
        module.load_bias() == binding.bias
            && binding.bias == binding.factory.load_bias()
            && runtime.segments.len() == 2
            && runtime.tls.is_none()
            && runtime.relro.is_empty(),
        "JSON module mapping",
    )?;
    let code = &runtime.segments[0];
    let table = &runtime.segments[1];
    require(
        std::ptr::eq(code.file_bytes, &binding.factory.image()[..CODE_FILE_BYTES])
            && code.vaddr == 0
            && code.mem_size == PAGE_SIZE
            && code.flags.raw() == 5
            && std::ptr::eq(table.file_bytes, binding.factory.data_bytes())
            && table.vaddr == binding.factory.table_rva()
            && table.mem_size == PAGE_SIZE
            && table.flags.raw() == 4,
        "JSON factory source identity",
    )
}

/// The immutable current plan is fully checked at preparation/publication.
/// Individual calls retain constant-work exact factory/source checks plus live
/// attestation, rather than rescanning every unrelated relocation each time.
pub(super) fn validate(
    plan: &SymbolicLinkPlan<'_>,
    binding: &JsonBinding<'_>,
    limits: &KernelStartupLimits,
    work: &mut u64,
) -> Result<(), KernelStartupError> {
    charge(work, plan.patch_count() as u64, limits.max_provider_work)?;
    validate_source(plan, binding, limits, work)?;
    for patch in plan.patches() {
        require(
            patch.consumer().index() != binding.index,
            "JSON provider relocation",
        )?;
        let Resolution::Definition(symbol) = patch.resolution() else {
            continue;
        };
        if symbol.module.index() != binding.index {
            continue;
        }
        let record = patch.source().record;
        let addend = if record.kind() == 1 { record.addend } else { 0 };
        require(
            binding
                .factory
                .export_marker(symbol.symbol)
                .is_some_and(|marker| {
                    (binding.bias + marker.rva).wrapping_add_signed(addend) == patch.value()
                }),
            "JSON binding outside manifest",
        )?;
    }
    Ok(())
}

pub(super) fn attest_all(
    memory: &PausedNativeMemory<'_>,
    binding: &JsonBinding<'_>,
    work: &mut u64,
    maximum: u64,
) -> Result<(), KernelStartupError> {
    // All markers, table words and immutable scalar/String defaults are charged.
    charge(
        work,
        binding.factory.markers().len() as u64 * 8 + 104 + 8,
        maximum,
    )?;
    for marker in binding.factory.markers() {
        let expected = marker.bytes();
        let at = marker.rva as usize;
        require(
            binding.factory.image().get(at..at + 8) == Some(&expected),
            "JSON source marker",
        )?;
        let address = binding.bias + marker.rva;
        require(
            matches!(
                memory.preflight_write(address, 8),
                Err(NativeMemoryError::PermissionDenied(_))
            ) && matches!(
                memory.preflight_gpu_write(address, 8),
                Err(NativeMemoryError::PermissionDenied(_))
            ),
            "JSON marker immutable permission",
        )?;
        let mut actual = [0; 8];
        memory
            .fetch_source(address, &mut actual)
            .map_err(|_| KernelStartupError::InvalidProvider("JSON marker fetch permission"))?;
        require(actual == expected, "JSON live marker bytes")?;
    }
    let address = binding.bias + binding.factory.table_rva();
    require(
        matches!(
            memory.preflight_write(address, PAGE_SIZE),
            Err(NativeMemoryError::PermissionDenied(_))
        ) && matches!(
            memory.preflight_gpu_write(address, PAGE_SIZE),
            Err(NativeMemoryError::PermissionDenied(_))
        ) && matches!(
            memory.fetch_source(address, &mut [0]),
            Err(NativeMemoryError::PermissionDenied(_))
        ),
        "JSON table immutable NX permission",
    )?;
    let mut actual = [0; 104];
    memory
        .read(address, &mut actual)
        .map_err(|_| KernelStartupError::InvalidProvider("JSON table read permission"))?;
    require(
        &actual[..64] == binding.factory.table_bytes()
            && actual[64..] == binding.factory.data_bytes()[64..],
        "JSON live guest table and scalar defaults",
    )
}

pub(super) fn stop(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    source: &SymbolicLinkPlan<'_>,
    request: NativeServiceRequest,
) -> Result<u64, KernelStartupError> {
    let binding = session
        .plan
        .json
        .ok_or(KernelStartupError::InvalidProvider("missing JSON provider"))?;
    validate_source(
        source,
        &binding,
        &session.plan.limits,
        &mut session.provider_work,
    )?;
    charge(
        &mut session.provider_work,
        binding.factory.markers().len() as u64,
        session.plan.limits.max_provider_work,
    )?;
    let marker = binding
        .factory
        .markers()
        .iter()
        .find(|marker| binding.bias + marker.rva + 5 == request.syscall_pc)
        .ok_or(KernelStartupError::SiteMismatch {
            syscall_pc: request.syscall_pc,
        })?;
    if request.service_id != u64::from(marker.service_id) {
        return Err(KernelStartupError::ServiceIdMismatch {
            expected: marker.service_id,
            actual: request.service_id,
        });
    }
    let mut paused = session.hle.pause().map_err(KernelStartupError::Hle)?;
    attest_all(
        &paused,
        &binding,
        &mut session.provider_work,
        session.plan.limits.max_provider_work,
    )?;
    match marker.function {
        JsonFunction::VirtualCall => Err(KernelStartupError::UnavailableJsonAllocatorVirtual),
        JsonFunction::ValueCtor => {
            // Pinned Kyty JsonValueInit initializes the complete 32-byte Null
            // object, including padding. Its constructor explicitly accepts
            // null and returns self; no host-owned Node representation is used.
            if request.args[0] != 0 {
                charge(
                    &mut session.provider_work,
                    32,
                    session.plan.limits.max_provider_work,
                )?;
                paused.write(request.args[0], &[0; 32]).map_err(|e| {
                    KernelStartupError::Kernel(pentax_kernel::Failure::Guest(
                        super::super::kernel_adapter::error(e, true),
                    ))
                })?;
            }
            Ok(request.args[0])
        }
        JsonFunction::MemAllocatorCtor | JsonFunction::MemAllocatorDtor => {
            // AnyPS5's observed SysV void constructor writes only the first word.
            // Whole-span write preflight precedes every effect; no object-size,
            // alignment, allocation method or RTTI contract is fabricated.
            charge(
                &mut session.provider_work,
                8,
                session.plan.limits.max_provider_work,
            )?;
            require(request.args[0] != 0, "null JSON constructor object")?;
            paused
                .write(
                    request.args[0],
                    &binding.factory.address_point().to_le_bytes(),
                )
                .map_err(|e| {
                    KernelStartupError::Kernel(pentax_kernel::Failure::Guest(
                        super::super::kernel_adapter::error(e, true),
                    ))
                })?;
            // Void transport: callers must observe the object, not use a result.
            Ok(0)
        }
        function => {
            let default = |function| {
                binding
                    .factory
                    .scalar_default_rva(function)
                    .and_then(|rva| binding.bias.checked_add(rva))
                    .ok_or(KernelStartupError::InvalidProvider(
                        "JSON scalar default manifest",
                    ))
            };
            let defaults = super::json_values::ScalarDefaults {
                boolean: default(JsonFunction::ValueGetBoolean)?,
                integer: default(JsonFunction::ValueGetInteger)?,
                real: default(JsonFunction::ValueGetReal)?,
            };
            let call = super::json_strings::JsonCall {
                function,
                args: request.args,
                vector_args: request.vector_args,
                defaults,
                empty_string: binding.bias + binding.factory.empty_string_rva(),
                empty_cstr: binding.bias + binding.factory.empty_cstr_rva(),
            };
            session.json.service(
                &mut paused,
                &mut session.kernel,
                call,
                &mut session.provider_work,
                session.plan.limits.max_provider_work,
            )
        }
    }
}

#[cfg(test)]
mod tests;
