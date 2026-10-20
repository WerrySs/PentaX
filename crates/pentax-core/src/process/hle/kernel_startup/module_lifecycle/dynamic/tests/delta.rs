// SPDX-License-Identifier: GPL-2.0-only

//! Original inputs exercise core authority separately from linker metadata.
use super::*;
use crate::process::hle::build_unavailable_provider;
use pentax_linker::{
    ModuleInput, append_symbolic_modules_with_fallbacks, plan_missing_functions_for_extension,
};

fn importing_input() -> Input {
    let mut bytes = crate::process::hle::unavailable::tests::original_input(1);
    bytes[152..160].copy_from_slice(&(12_u64 * 16).to_le_bytes());
    bytes[160..168].copy_from_slice(&(12_u64 * 16).to_le_bytes());
    bytes[0x2a0..0x2a8].copy_from_slice(&12_u64.to_le_bytes());
    bytes[0x2a8..0x2b0].copy_from_slice(&0x100_u64.to_le_bytes());
    Input::from_bytes(bytes)
}

#[test]
fn exact_delta_is_source_owned_stops_at_use_and_has_no_guest_handle() {
    let file = importing_input();
    session(|owner, scratch| {
        owner
            .register_dynamic_module_file(PATH, file.open())
            .unwrap();
        let block = BlockLimits::default();
        let proof = owner.prepare_native_source(&block).unwrap();
        let request = load_request(owner, scratch);
        let input = read(owner, &proof, request);
        let base = proof.materialized_source().source_plan();
        let added = ModuleInput::new(
            open_executable(input.bytes()).unwrap(),
            owner.dynamic_load_bias(&input).unwrap(),
        );
        let policies = owner.dynamic_fallback_policies(&proof).unwrap();
        let missing = plan_missing_functions_for_extension(
            base,
            added.clone(),
            &policies,
            &owner.dynamic_extension_limits(),
            1,
        )
        .unwrap();
        let remaining = owner.dynamic_unavailable_limits().unwrap();
        let factory = build_unavailable_provider(missing, &remaining).unwrap();
        let bias = owner.dynamic_unavailable_bias(&input).unwrap();
        let next = append_symbolic_modules_with_fallbacks(
            base,
            &[
                added,
                ModuleInput::new(open_executable(factory.image()).unwrap(), bias),
            ],
            &policies,
            &owner.dynamic_extension_limits(),
        )
        .unwrap();
        let old_count = owner.memory().module_count();
        let start = owner
            .publish_dynamic_batch(&proof, &input, &next, Some(&factory), &block)
            .unwrap();
        assert_eq!(start.module, old_count);
        assert_eq!(owner.loaded_modules.resolve_handle(2), Some(old_count));
        assert_eq!(owner.loaded_modules.resolve_handle(3), None);
        assert_eq!(owner.memory().module_count(), old_count + 2);
        assert_eq!(owner.loaded_modules.dynamic.published.len(), 1);
        assert_eq!(
            owner.dynamic_unavailable_limits().unwrap().max_functions,
            remaining.max_functions - 1
        );
        let epoch = owner.prepare_epoch_native_source(&next, &block).unwrap();
        assert_eq!(
            owner.dynamic_fallback_policies(&epoch).unwrap().len(),
            policies.len() + 1
        );
        let site = factory.site(1).unwrap();
        let pc = bias + site.rva + 5;
        let request = NativeServiceRequest {
            owner_id: owner.owner_id(),
            syscall_pc: pc,
            next_pc: pc + 2,
            service_id: site.service_id.into(),
            args: [0; 6],
            vector_args: None,
            stack_pointer: None,
        };
        assert!(matches!(
            owner.service_action(&epoch, request),
            Err(KernelStartupError::UnavailableImport { ordinal: 0, .. })
        ));
        assert!(owner.service_action(&proof, request).is_err());
        assert_eq!(factory.function_count(), 1);
    });
}

#[test]
fn independently_classified_factory_and_retained_budget_are_required_before_effects() {
    for foreign_classification in [true, false] {
        let file = importing_input();
        session(|owner, scratch| {
            owner
                .register_dynamic_module_file(PATH, file.open())
                .unwrap();
            let block = BlockLimits::default();
            let proof = owner.prepare_native_source(&block).unwrap();
            let request = load_request(owner, scratch);
            let input = read(owner, &proof, request);
            let base = proof.materialized_source().source_plan();
            let added = ModuleInput::new(
                open_executable(input.bytes()).unwrap(),
                owner.dynamic_load_bias(&input).unwrap(),
            );
            let policies = owner.dynamic_fallback_policies(&proof).unwrap();
            let missing = if foreign_classification {
                // Same exports and bytes, but classified as a different consumer
                // module. Public symbol names cannot confer origin authority.
                pentax_linker::plan_missing_functions(
                    std::slice::from_ref(&added),
                    &Default::default(),
                    1,
                )
                .unwrap()
            } else {
                plan_missing_functions_for_extension(
                    base,
                    added.clone(),
                    &policies,
                    &owner.dynamic_extension_limits(),
                    1,
                )
                .unwrap()
            };
            let factory =
                build_unavailable_provider(missing, &owner.dynamic_unavailable_limits().unwrap())
                    .unwrap();
            let bias = owner.dynamic_unavailable_bias(&input).unwrap();
            let next = append_symbolic_modules_with_fallbacks(
                base,
                &[
                    added,
                    ModuleInput::new(open_executable(factory.image()).unwrap(), bias),
                ],
                &policies,
                &owner.dynamic_extension_limits(),
            )
            .unwrap();
            if !foreign_classification {
                owner
                    .loaded_modules
                    .dynamic
                    .unavailable_limits
                    .as_mut()
                    .unwrap()
                    .max_functions = owner.loaded_modules.dynamic.unavailable_functions;
            }
            let identity = owner.memory().active_source_identity();
            let modules = owner.memory().module_count();
            let bytes = owner.memory().mapped_bytes();
            let cursor = owner.loaded_modules.dynamic.cursor;
            assert!(
                owner
                    .publish_dynamic_batch(&proof, &input, &next, Some(&factory), &block)
                    .is_err()
            );
            assert_eq!(owner.memory().active_source_identity(), identity);
            assert_eq!(owner.memory().module_count(), modules);
            assert_eq!(owner.memory().mapped_bytes(), bytes);
            assert_eq!(owner.loaded_modules.dynamic.cursor, cursor);
            assert!(owner.loaded_modules.dynamic.published.is_empty());
            assert_eq!(owner.loaded_modules.resolve_handle(2), None);
            assert!(matches!(
                owner.loaded_modules.dynamic.files[0].state,
                FileState::Failed
            ));
        });
    }
}
