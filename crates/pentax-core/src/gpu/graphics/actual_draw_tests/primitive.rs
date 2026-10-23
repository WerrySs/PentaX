// SPDX-License-Identifier: GPL-2.0-only
//! Original CreatePrimState -> actual encoders -> shader/target preparation.
use super::*;
use crate::gpu::driver::{CommandBuilderAction, Driver, Function};
fn encode(
    memory: &mut PausedNativeMemory<'_>,
    driver: &mut Driver,
    base: u64,
    group: u32,
    user: u32,
    stages: u32,
    primitive: u32,
) -> Vec<u8> {
    let shader = base + 0x200;
    let specials = base + 0x400;
    let cx = base + 0x1000;
    let uc = base + 0x1100;
    let buffer = base + 0x100;
    let commands = base + 0x6000;
    memory.write(shader + 40, &specials.to_le_bytes()).unwrap();
    memory.write(shader + 90, &[2]).unwrap();
    memory
        .write(
            specials,
            &bytes(&[
                0x25b, group, 0x2d5, stages, 0, 0, 0, 0, 0x29b, 1, 0x262, user,
            ]),
        )
        .unwrap();
    assert_eq!(
        driver.submit(
            memory,
            Function::AgcCreatePrimState,
            [cx, uc, 0, shader, u64::from(primitive), 0]
        ),
        Ok(0)
    );
    let mut descriptor = [0; 56];
    descriptor[16..24].copy_from_slice(&commands.to_le_bytes());
    descriptor[24..32].copy_from_slice(&(commands + 256).to_le_bytes());
    memory.write(buffer, &descriptor).unwrap();
    for index in 0..2 {
        let pair = read(memory, cx + index * 8, 8);
        let register = u32::from_le_bytes(pair[..4].try_into().unwrap());
        let value = u32::from_le_bytes(pair[4..].try_into().unwrap());
        assert!(
            matches!(driver.begin_command_builder(memory,Function::AgcDcbSetCxRegisterDirect,[buffer,u64::from(register),u64::from(value),0,0,0],None).unwrap(),CommandBuilderAction::Returned(value) if value!=0)
        );
    }
    assert!(
        matches!(driver.begin_command_builder(memory,Function::AgcCbSetUcRegistersDirect,[buffer,uc,3,0,0,0],None).unwrap(),CommandBuilderAction::Returned(value) if value!=0)
    );
    let up = u64::from_le_bytes(read(memory, buffer + 16, 8).try_into().unwrap());
    read(memory, commands, (up - commands) as usize)
}
#[test]
fn created_primitive_pairs_flow_through_real_register_encoders_into_owned_draw_preparation() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x40000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x20000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let vertex = base + 0x4000;
    let pixel = base + 0x5000;
    let target = base + 0x10000;
    memory.write(vertex, &bytes(&vertex_shader())).unwrap();
    memory.write(pixel, &bytes(&pixel_shader())).unwrap();
    memory
        .write(target, &SENTINEL.repeat(WIDTH * HEIGHT))
        .unwrap();
    let setup = bytes(&commands(vertex, pixel, target));
    let mut driver = Driver::new(Default::default());
    for (group, user, stages, primitive, expected) in [
        (0, 0, 0x0040_0000, 4, None),
        (0xfffc_0000, 0xffff_fff8, 0x0040_0000, 4, None),
        (
            1,
            0,
            0x0040_0000,
            4,
            Some("graphics primitive grouping profile"),
        ),
        (
            0,
            4,
            0x0040_0000,
            4,
            Some("graphics extra user VGPR profile"),
        ),
        (
            0,
            0,
            0x0040_0020,
            4,
            Some("merged/NGG/geometry/tessellation stages"),
        ),
        (0, 0, 0x0040_0000, 2, Some("triangle-list primitive")),
    ] {
        let mut data = setup.clone();
        data.extend(encode(
            &mut memory,
            &mut driver,
            base,
            group,
            user,
            stages,
            primitive,
        ));
        data.extend(bytes(&[0xc001_2d00, 3, 2]));
        let plan = plan_commands(&[&data], GraphicsState::empty(), &Default::default()).unwrap();
        assert_eq!(plan.draws().len(), 3);
        let result = pentax_gpu::graphics::prepare_draw(
            &plan.draws()[2],
            &crate::gpu::Memory(&memory),
            &Default::default(),
        );
        if let Some(reason) = expected {
            assert!(
                matches!(result,Err(pentax_gpu::graphics::Error::Unsupported(actual)) if actual==reason)
            );
        } else {
            let prepared = result.unwrap();
            assert_eq!(prepared.target_address(), target);
            assert_eq!(prepared.input(), SENTINEL.repeat(WIDTH * HEIGHT));
            assert_eq!(prepared.vertex().spirv()[0], 0x0723_0203);
            assert_eq!(prepared.pixel().spirv()[0], 0x0723_0203);
        }
    }
    assert_eq!(driver.usage().draws, 0);
    assert_eq!(driver.usage().dispatches, 0);
    assert_eq!(driver.usage().command_buffers, 0);
    assert_eq!(
        read(&memory, target, TARGET_BYTES),
        SENTINEL.repeat(WIDTH * HEIGHT)
    );
}
