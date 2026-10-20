// SPDX-License-Identifier: GPL-2.0-only

//! Original commands, shaders and data live in real owned guest pages.
use pentax_core::gpu::execute_compute;
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions, NativeRange};
use pentax_platform::vulkan::GpuLimits;

pub(super) fn run() -> Result<(usize, u64), String> {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x10000)
        .map_err(|_| "original GPU reservation")?;
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x8000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .map_err(|_| "original GPU pages")?;
    let code = base + 0x1000;
    let commands = base + 0x2000;
    let data = base + 0x4000;
    let shader = [
        (0x25_u32 << 25) | (1 << 17) | 4,
        (0x1a << 25) | (2 << 17) | 130,
        0xe070_1000,
        (128 << 24) | (1 << 8) | 2,
        0xbf81_0000,
    ];
    let shader: Vec<u8> = shader.into_iter().flat_map(u32::to_le_bytes).collect();
    let mut total = (0, 0);
    for (lanes, length, seed) in [(16_u32, 64_u32, 17_u32), (8, 48, 23)] {
        owner
            .protect(
                base,
                0x8000,
                NativePermissions::READ_WRITE.with_gpu(true, true),
            )
            .map_err(|_| "original upload access")?;
        let mut words = Vec::new();
        let mut set = |first: u32, values: &[u32]| {
            words.push(0xc000_7602 | (values.len() as u32) << 16);
            words.push(first);
            words.extend(values);
        };
        set(0x207, &[lanes, 1, 1]);
        set(0x20c, &[(code >> 8) as u32, (code >> 40) as u32]);
        set(0x212, &[0x2c_0000, 10]);
        set(0x228, &[0]);
        set(0x240, &[data as u32, (data >> 32) as u32, length, 0, seed]);
        words.extend([0xc003_1502, 1, 1, 1, 0x8005]);
        let words: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
        let mut paused = owner.pause().map_err(|_| "original GPU pause")?;
        paused
            .write(code, &shader)
            .map_err(|_| "original shader write")?;
        paused
            .write(commands, &words)
            .map_err(|_| "original command write")?;
        paused
            .write(data, &vec![0xa5; length as usize])
            .map_err(|_| "original data write")?;
        // The shader only stores. Preserve untouched tail bytes without granting
        // GPU read access; CPU write access is withdrawn before the actual job.
        owner
            .protect(
                base,
                0x4000,
                NativePermissions::new(false, false, false).with_gpu(true, false),
            )
            .map_err(|_| "original GPU source permissions")?;
        owner
            .protect(data, 0x4000, NativePermissions::READ.with_gpu(false, true))
            .map_err(|_| "original GPU output permissions")?;
        let mut paused = owner.pause().map_err(|_| "original GPU pause")?;
        if paused.read(code, &mut [0]).is_ok()
            || paused.read_gpu(data, &mut [0]).is_ok()
            || paused.write(data, &[0]).is_ok()
        {
            return Err("original CPU/GPU permission isolation failed".into());
        }
        let report = execute_compute(
            &mut paused,
            NativeRange {
                address: commands,
                size: words.len() as u64,
            },
            &Default::default(),
            &GpuLimits::default(),
        )
        .map_err(|e| e.to_string())?;
        let mut result = vec![0; length as usize];
        paused
            .read(data, &mut result)
            .map_err(|_| "original GPU result")?;
        for (lane, value) in result.chunks_exact(4).enumerate() {
            let expected = if lane < lanes as usize {
                lane as u32 + seed
            } else {
                0xa5a5_a5a5
            };
            if u32::from_le_bytes(value.try_into().unwrap()) != expected {
                return Err("original PM4 compute value mismatch".into());
            }
        }
        let mut original = vec![0; words.len()];
        paused
            .read_gpu(commands, &mut original)
            .map_err(|_| "original PM4 read")?;
        if original != words {
            return Err("original PM4 bytes changed".into());
        }
        original.resize(shader.len(), 0);
        paused
            .read_gpu(code, &mut original)
            .map_err(|_| "original shader read")?;
        if original != shader {
            return Err("original shader bytes changed".into());
        }
        total.0 += report.written_bytes;
        total.1 += report.preparation.invocations;
    }
    Ok(total)
}
