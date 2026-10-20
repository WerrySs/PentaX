// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: ordered checked interpolant mappings.
// Pinned agc.cpp1215..1443 and shader.h; see NOTICE.

use super::{fusion::Editor, *};
// Normal hardware mappings have at most32 entries. Retain source behavior above
//32 under a finite staging bound, rather than silently truncating the input.
const MAX_INPUTS: u32 = 256;

fn output_semantic(
    editor: &mut Editor<'_, '_, '_>,
    geometry: u64,
    semantic: u8,
    second: bool,
) -> Result<Option<u64>, Failure> {
    let pointer = if second {
        None
    } else {
        Some(editor.wide(at(geometry, 56)?)?)
    };
    if pointer == Some(0) {
        return Ok(None);
    }
    let count = editor.half(at(geometry, 86)?)?;
    if count == 0 {
        return Ok(None);
    }
    let pointer = match pointer {
        Some(pointer) => pointer,
        None => editor.wide(at(geometry, 56)?)?,
    };
    for index in 0..u64::from(count) {
        let address = at(pointer, index * 4)?;
        if editor.byte(address)? == semantic {
            return Ok(Some(address));
        }
    }
    Ok(None)
}
fn field(value: u32, source: u32, shift: u32) -> u32 {
    (value & !(3 << shift)) | ((source & 3) << shift)
}
fn original(source: u32, output: Option<u32>) -> u32 {
    let mut value = if source & 0x0030_0000 != 0 {
        let mut value = (source << 4) & 0x0300_0000;
        if let Some(output) = output {
            let common = source & output;
            value = (value & 0xfff7_ffdf) | ((common >> 15) & 0x20);
            value ^= 0x0008_0020;
            value = (value & !0x0010_0000) | ((!common >> 1) & 0x0010_0000);
        } else {
            value |= 0x0018_0020;
        }
        field(value, source >> 30, 21)
    } else if source & 0x0100_0000 != 0 || output.is_none() {
        0x20
    } else {
        0
    };
    value &= !0x41f;
    if let Some(output) = output {
        value |= (output >> 8) & 31;
        if source & 0x0140_0000 != 0 {
            value |= 0x400;
        }
    }
    field(value, source >> 28, 8)
}
fn second(source: u32, output: Option<u32>) -> u32 {
    let mode = (source >> 20) & 3;
    let mut flags = if mode == 0 {
        field(
            (((source >> 24) & 1) | u32::from(output.is_none())) << 5,
            source >> 28,
            8,
        )
    } else {
        let mut flags = ((source << 4) & 0x0300_0000) + 0x0008_0000;
        if mode == 2 {
            flags &= 0xffef_ffdf;
            flags |= output.map_or(0x20, |mask| (!(mask & source) >> 16) & 0x20);
            flags = field(flags, source >> 30, 8);
        } else {
            if let Some(mask) = output {
                let masked = mask & source;
                flags = (flags & 0xffff_ffdf) | ((masked >> 15) & 0x20);
                flags ^= 0x20;
                flags = (flags & 0xffef_ffff) | ((!masked >> 1) & 0x0010_0000);
            } else {
                flags |= 0x0010_0020;
            }
            flags = field(flags, source >> 28, 8);
        }
        field(flags, source >> 30, 21)
    };
    if let Some(mask) = output {
        flags = (flags & 0xffff_ffe0) | ((mask >> 8) & 31);
        flags = (flags & 0xffff_fbff)
            | if source & 0x0040_0000 != 0 {
                0x400
            } else {
                (source >> 14) & 0x400
            };
    } else {
        flags &= 0xffff_fbe0;
    }
    flags
}
fn register(
    editor: &mut Editor<'_, '_, '_>,
    destination: u64,
    index: u32,
    value: u32,
) -> Result<(), Failure> {
    let mut bytes = [0; 8];
    bytes[..4].copy_from_slice(&(0x191 + index).to_le_bytes());
    bytes[4..].copy_from_slice(&value.to_le_bytes());
    editor.write(at(destination, u64::from(index) * 8)?, &bytes)
}
pub(in crate::gpu) fn create(
    memory: &mut PausedNativeMemory<'_>,
    args: [u64; 6],
    mapping2: bool,
    used: &mut u64,
    maximum: u64,
) -> Result<u64, Failure> {
    let [destination, geometry, pixel, ..] = args;
    if destination == 0 {
        return Err(Failure::Shader(ShaderFailure::NullArgument));
    }
    let mut editor = Editor::new(memory, used, maximum);
    let count = if pixel == 0 {
        0
    } else {
        editor.word(at(pixel, 80)?)?
    };
    if count > 0 && editor.wide(at(pixel, 48)?)? == 0 {
        return Err(Failure::Shader(ShaderFailure::NullArgument));
    }
    if count == 0 {
        for index in 0..32 {
            register(&mut editor, destination, index, index)?;
        }
        return editor.finish(0);
    }
    if geometry == 0 {
        return Err(Failure::Shader(ShaderFailure::NullArgument));
    }
    if editor.half(at(geometry, 86)?)? != 0 && editor.wide(at(geometry, 56)?)? == 0 {
        return Err(Failure::Shader(ShaderFailure::NullArgument));
    }
    let mut index = 0;
    loop {
        // Output aliases may alter the next source count/pointer. Read through
        // staged bytes, bound every updated count, and never clamp success.
        let count = editor.word(at(pixel, 80)?)?;
        if count > MAX_INPUTS {
            return Err(Failure::Limit);
        }
        if index >= count {
            break;
        }
        let pointer = editor.wide(at(pixel, 48)?)?;
        let input = at(pointer, u64::from(index) * 4)?;
        let tag = editor.byte(input)?;
        let source = if mapping2 {
            Some(editor.word(input)?)
        } else {
            None
        };
        let output = output_semantic(&mut editor, geometry, tag, mapping2)?;
        let source = match source {
            Some(source) => source,
            None => editor.word(input)?,
        };
        let output = match output {
            Some(address) => Some(editor.word(address)?),
            None => None,
        };
        let value = if mapping2 {
            second(source, output)
        } else {
            original(source, output)
        };
        register(&mut editor, destination, index, value)?;
        index += 1;
    }
    let first_identity = editor.word(at(pixel, 80)?)?;
    for index in first_identity..32 {
        register(&mut editor, destination, index, index)?;
    }
    editor.finish(0)
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests;
