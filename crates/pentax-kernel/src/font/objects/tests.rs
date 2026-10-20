// SPDX-License-Identifier: GPL-2.0-only
// Original font object calls and output canaries; no game or host pointer data.

use super::*;
use crate::Limits;
use crate::tests::{ARG, TestGuest};

struct Fixture {
    objects: Objects,
    memory: MemoryState,
    guest: TestGuest,
    budget: Budget,
}

impl Fixture {
    fn new(limits: Limits) -> Self {
        Self {
            objects: Objects::default(),
            memory: MemoryState::new(limits).unwrap(),
            guest: TestGuest::new(),
            budget: Budget::new(limits).unwrap(),
        }
    }
    fn invoke(
        &mut self,
        service: Service,
        args: &[u64],
        vectors: Option<[[u8; 16]; 8]>,
        stack: Option<u64>,
    ) -> Result<u64, Failure> {
        let mut values = [0; 6];
        values[..args.len()].copy_from_slice(args);
        let held = self.objects.metadata_bytes();
        self.objects.invoke(
            FontCall {
                service,
                args: CallArgs(values),
                vector_args: vectors,
                stack_pointer: stack,
            },
            &mut self.memory,
            &mut self.guest,
            &mut self.budget,
            held,
        )
    }
    fn call(&mut self, service: Service, args: &[u64]) -> Result<u64, Failure> {
        self.invoke(service, args, None, None)
    }
    fn open(&mut self, offset: u64) -> u64 {
        assert_eq!(
            self.call(
                Service::FontOpenFontSet,
                &[u64::MAX, 11, 13, u64::MAX, ARG + offset]
            ),
            Ok(0)
        );
        self.guest.u64(offset)
    }
}

fn vectors(x: u32, y: u32) -> [[u8; 16]; 8] {
    let mut data = [[0x7b; 16]; 8];
    data[0][..4].copy_from_slice(&x.to_le_bytes());
    data[1][..4].copy_from_slice(&y.to_le_bytes());
    data
}

#[test]
fn font_memory_exact_layout_uses_seventh_gp_and_term_preserves_other_bytes() {
    let mut f = Fixture::new(Limits::default());
    f.guest.put(0x200, &[0xa5; 66]);
    assert_eq!(
        f.invoke(
            Service::FontMemoryInit,
            &[0, u64::MAX, 4, u64::MAX, u64::MAX, u64::MAX],
            None,
            None
        ),
        Ok(FAILURE)
    );
    assert!(f.call(Service::FontMemoryInit, &[ARG + 0x200]).is_err());
    assert_eq!(f.guest.get(ARG + 0x200, 66), vec![0xa5; 66]);
    f.guest.put(0x108, &0x8877_6655_4433_2211u64.to_le_bytes());
    assert_eq!(
        f.invoke(
            Service::FontMemoryInit,
            &[
                ARG + 0x200,
                u64::MAX,
                0x1234_5678_89ab_cdef,
                0x1111,
                0x2222,
                0x3333
            ],
            None,
            Some(ARG + 0x100)
        ),
        Ok(0)
    );
    let mut expected = [0; 64];
    expected[..2].copy_from_slice(&1u16.to_le_bytes());
    expected[4..8].copy_from_slice(&0x89ab_cdefu32.to_le_bytes());
    expected[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    expected[16..24].copy_from_slice(&0x2222u64.to_le_bytes());
    expected[24..32].copy_from_slice(&0x1111u64.to_le_bytes());
    expected[32..40].copy_from_slice(&0x3333u64.to_le_bytes());
    expected[40..48].copy_from_slice(&0x8877_6655_4433_2211u64.to_le_bytes());
    assert_eq!(f.guest.get(ARG + 0x200, 64), expected);
    assert_eq!(f.call(Service::FontMemoryTerm, &[ARG + 0x200]), Ok(0));
    expected[..2].fill(0);
    assert_eq!(
        f.guest.get(ARG + 0x200, 66),
        [expected.as_slice(), &[0xa5; 2]].concat()
    );
    assert_eq!(f.call(Service::FontMemoryTerm, &[0]), Ok(0));
}

#[test]
fn font_library_renderer_are_owned_and_destroy_slot_and_mapping_atomically() {
    let mut f = Fixture::new(Limits::default());
    assert_eq!(
        f.call(
            Service::FontCreateLibraryWithEdition,
            &[u64::MAX, u64::MAX, 9, 0]
        ),
        Ok(FAILURE)
    );
    assert_eq!(
        f.call(
            Service::FontCreateLibraryWithEdition,
            &[u64::MAX, u64::MAX, 9, ARG]
        ),
        Ok(0)
    );
    let library = f.guest.u64(0);
    assert_eq!(
        f.guest.get(library, 16),
        [u64::MAX.to_le_bytes(), 9u64.to_le_bytes()].concat()
    );
    assert_eq!(
        f.call(
            Service::FontCreateRendererWithEdition,
            &[11, 13, 17, ARG + 8]
        ),
        Ok(0)
    );
    let renderer = f.guest.u64(8);
    assert_ne!(library, renderer);
    assert_eq!(
        f.guest.get(renderer, 24),
        [
            11u64.to_le_bytes(),
            13u64.to_le_bytes(),
            17u64.to_le_bytes()
        ]
        .concat()
    );
    assert_eq!(f.budget.usage().handles_live, 4);
    assert_eq!(f.objects.retained_handle_count(), 2);
    assert!(f.call(Service::FontDestroyRenderer, &[ARG]).is_err());
    f.guest.refuse = true;
    assert!(f.call(Service::FontDestroyLibrary, &[ARG]).is_err());
    assert_eq!(f.guest.u64(0), library);
    assert_eq!(f.budget.usage().handles_live, 4);
    f.memory
        .check_runtime_object(f.objects.libraries[0], &mut f.budget)
        .unwrap();
    f.guest.refuse = false;
    assert_eq!(f.call(Service::FontDestroyLibrary, &[ARG]), Ok(0));
    assert_eq!(f.guest.u64(0), 0);
    assert!(f.guest.read(library, &mut [0]).is_err());
    assert_eq!(f.call(Service::FontDestroyLibrary, &[ARG]), Ok(0));
    assert_eq!(f.call(Service::FontDestroyRenderer, &[ARG + 8]), Ok(0));
    assert_eq!(f.budget.usage().handles_live, 0);
    assert_eq!(f.memory.retained_handle_count(), 0);
}

#[test]
fn font_ft_selectors_are_distinct_stable_owned_values_and_count_real_resources() {
    let mut f = Fixture::new(Limits::default());
    let library = f
        .call(Service::FontSelectLibraryFt, &[0x1111_2222_ffff_fffd])
        .unwrap();
    let renderer = f.call(Service::FontSelectRendererFt, &[7]).unwrap();
    assert_ne!(library, renderer);
    assert_eq!(
        f.guest.get(library, 8),
        [(-3i32).to_le_bytes(), 0i32.to_le_bytes()].concat()
    );
    assert_eq!(
        f.guest.get(renderer, 8),
        [7i32.to_le_bytes(), 1i32.to_le_bytes()].concat()
    );
    assert_eq!(f.call(Service::FontSelectLibraryFt, &[29]), Ok(library));
    assert_eq!(
        f.guest.get(library, 8),
        [29i32.to_le_bytes(), 0i32.to_le_bytes()].concat()
    );
    assert_eq!(f.budget.usage().handles_live, 4);
    f.guest.refuse = true;
    assert!(f.call(Service::FontSelectLibraryFt, &[31]).is_err());
    assert_eq!(f.guest.get(library, 4), 29i32.to_le_bytes());
    f.guest.refuse = false;
    let token = f.objects.library_selection.unwrap();
    f.memory
        .release_runtime_object(token, &mut f.guest, &mut f.budget)
        .unwrap();
    assert!(f.call(Service::FontSelectLibraryFt, &[37]).is_err());
    assert_eq!(f.guest.get(renderer, 4), 7i32.to_le_bytes());
}

#[test]
fn font_constructor_has_real_pixels_and_instance_resets_config_with_exact_vector_words() {
    let mut f = Fixture::new(Limits::default());
    let font = f.open(0);
    let descriptor = f.guest.get(font, 24);
    assert_eq!(
        u64::from_le_bytes(descriptor[..8].try_into().unwrap()),
        font + 64
    );
    assert_eq!(
        u32::from_le_bytes(descriptor[8..12].try_into().unwrap()),
        128
    );
    assert!(
        f.guest
            .get(font + 64, engine::IMAGE_BYTES)
            .iter()
            .any(|byte| *byte != 0)
    );
    assert_eq!(
        f.call(
            Service::FontDefineAttribute,
            &[font, 0xffff_fffb, ARG + 0x80]
        ),
        Ok(0)
    );
    assert_eq!(f.guest.get(ARG + 0x80, 4), 0i32.to_le_bytes());
    assert_eq!(f.call(Service::FontBindRenderer, &[font, u64::MAX]), Ok(0));
    let vector = vectors(0x7fc1_2345, (-0.0f32).to_bits());
    assert!(f.call(Service::FontSetScalePixel, &[font]).is_err());
    assert_eq!(
        f.invoke(Service::FontSetScalePixel, &[font], Some(vector), None),
        Ok(0)
    );
    assert_eq!(
        f.invoke(
            Service::FontSetEffectWeight,
            &[font, 0xabcd_ef01_89ab_cdef],
            Some(vector),
            None
        ),
        Ok(0)
    );
    assert_eq!(
        f.invoke(
            Service::FontSetupRenderEffectWeight,
            &[font, 0x1234],
            Some(vector),
            None
        ),
        Ok(0)
    );
    let state = f.objects.font(font, &f.memory, &mut f.budget).unwrap();
    assert_eq!(state.scale_w.to_bits(), 0x7fc1_2345);
    assert_eq!(state.scale_h.to_bits(), (-0.0f32).to_bits());
    assert_eq!(state.effect_weight_mode, 0x89ab_cdef);
    assert_eq!(state.render_effect_weight_mode, 0x1234);
    assert_eq!(
        f.call(Service::FontOpenFontInstance, &[font, u64::MAX, ARG + 8]),
        Ok(0)
    );
    let instance = f.guest.u64(8);
    let state = f.objects.font(instance, &f.memory, &mut f.budget).unwrap();
    assert_eq!(
        (
            state.attribute,
            state.renderer,
            state.scale_w,
            state.scale_h
        ),
        (0, 0, 8.0, 16.0)
    );
    assert_eq!(
        (
            state.library,
            state.source_address,
            state.font_set_type,
            state.open_mode
        ),
        (u64::MAX, u64::MAX, 11, 13)
    );
    assert_eq!(
        (state.effect_weight_mode, state.render_effect_weight_mode),
        (0, 0)
    );
    assert_eq!(
        f.call(Service::FontDefineAttribute, &[font, 19, u64::MAX]),
        Err(Failure::Guest(GuestError::AddressOverflow))
    );
    assert_eq!(
        f.objects
            .font(font, &f.memory, &mut f.budget)
            .unwrap()
            .attribute,
        -5
    );
    assert_eq!(f.call(Service::FontCloseFont, &[font]), Ok(0));
    assert!(f.call(Service::FontRebindRenderer, &[font]).is_err());
    assert_eq!(f.call(Service::FontCloseFont, &[instance]), Ok(0));
    assert_eq!(f.budget.usage().handles_live, 0);
}

#[test]
fn font_memory_admits_immutable_font_bytes_and_rejects_malformed_recognized_data() {
    let mut f = Fixture::new(Limits::default());
    let data = epaint_default_fonts::HACK_REGULAR;
    let input = f
        .memory
        .runtime_object(data.len(), 0, &mut f.guest, &mut f.budget, |_, output| {
            output.copy_from_slice(data);
            Ok(RuntimeDataOutputs::new())
        })
        .unwrap();
    assert_eq!(
        f.call(
            Service::FontOpenFontMemory,
            &[13, input.address, data.len() as u64, u64::MAX, ARG]
        ),
        Ok(0)
    );
    let font = f.guest.u64(0);
    f.guest
        .apply(
            crate::MemoryEffect::None,
            &[GuestWrite {
                address: input.address,
                bytes: &[0xff; 4],
            }],
        )
        .unwrap();
    assert_eq!(
        f.objects
            .font(font, &f.memory, &mut f.budget)
            .unwrap()
            .data
            .as_slice(),
        data
    );
    assert_eq!(
        f.call(Service::FontOpenFontInstance, &[font, 0, ARG + 8]),
        Ok(0)
    );
    let instance = f.guest.u64(8);
    assert_eq!(
        f.objects
            .font(instance, &f.memory, &mut f.budget)
            .unwrap()
            .data
            .as_slice(),
        data
    );
    f.guest.put(0x200, &[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let before = (f.objects.metadata_bytes(), f.budget.usage().handles_live);
    assert!(
        f.call(
            Service::FontOpenFontMemory,
            &[0, ARG + 0x200, 12, 0, ARG + 0x18]
        )
        .is_err()
    );
    assert_eq!(
        (f.objects.metadata_bytes(), f.budget.usage().handles_live),
        before
    );
    assert_eq!(f.guest.u64(0x18), 0);
    // Unsupported magic reads only the four-byte magic, not a claimed huge span.
    f.guest.put(0x200, b"nope");
    assert_eq!(
        f.call(
            Service::FontOpenFontMemory,
            &[0, ARG + 0x200, u32::MAX as u64, 0, ARG + 0x20]
        ),
        Ok(0)
    );
}

#[test]
fn font_publication_refusals_preserve_capacities_outputs_and_combined_handle_accounting() {
    let mut f = Fixture::new(Limits {
        max_handles: 3,
        ..Limits::default()
    });
    let unrelated = f
        .memory
        .runtime_object(1, 0, &mut f.guest, &mut f.budget, |_, _| {
            Ok(RuntimeDataOutputs::new())
        })
        .unwrap();
    assert_eq!(f.call(Service::FontCreateLibrary, &[0, 0, ARG]), Ok(0));
    let before = (
        f.objects.metadata_bytes(),
        f.memory.retained_handle_count(),
        f.budget.usage().handles_live,
    );
    assert!(
        f.call(Service::FontCreateRendererWithEdition, &[0, 0, 0, ARG + 8])
            .is_err()
    );
    assert_eq!(
        (
            f.objects.metadata_bytes(),
            f.memory.retained_handle_count(),
            f.budget.usage().handles_live
        ),
        before
    );
    assert_eq!(f.guest.u64(8), 0);
    f.memory
        .check_runtime_object(unrelated, &mut f.budget)
        .unwrap();
    let mut f = Fixture::new(Limits::default());
    f.guest.put(0, &[0xa5; 8]);
    f.guest.refuse = true;
    assert!(
        f.call(Service::FontCreateRendererWithEdition, &[0, 0, 0, ARG])
            .is_err()
    );
    assert_eq!(f.objects.metadata_bytes(), 0);
    assert_eq!(f.budget.usage().handles_live, 0);
    assert_eq!(f.guest.get(ARG, 8), [0xa5; 8]);
    f.guest.refuse = false;
    f.budget.limits.max_work = f.budget.usage().work;
    assert!(
        f.call(Service::FontOpenFontSet, &[0, 0, 0, 0, ARG])
            .is_err()
    );
    assert_eq!(f.objects.metadata_bytes(), 0);
    assert_eq!(f.memory.retained_handle_count(), 0);
    assert_eq!(f.guest.get(ARG, 8), [0xa5; 8]);
    f.budget.limits.max_work = Limits::default().max_work;
    f.budget.limits.max_scratch_bytes = 1;
    assert!(f.call(Service::FontCreateLibrary, &[0, 0, ARG]).is_err());
    assert_eq!(f.objects.metadata_bytes(), 0);
    assert_eq!(f.budget.usage().handles_live, 0);
    assert_eq!(f.call(Service::FontSetScalePixel, &[0]), Ok(FAILURE));
    assert_eq!(
        f.call(Service::FontOpenFontInstance, &[u64::MAX, u64::MAX, 0]),
        Ok(FAILURE)
    );
}
