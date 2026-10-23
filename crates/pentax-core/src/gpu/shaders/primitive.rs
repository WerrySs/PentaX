// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 from public KytyPS5 AgcCreatePrimState; see NOTICE.
//! Source-ordered owned register-pair computation, without GPU execution.
use super::{fusion::Editor, *};
fn output_primitive(primitive: u32) -> u32 {
    match primitive {
        1 => 0,
        2 | 3 | 10 | 11 | 18 => 1,
        7 => 3,
        17 => 4,
        _ => 2,
    }
}
fn special(editor: &mut Editor<'_, '_, '_>, shader: u64, offset: u64) -> Result<u64, Failure> {
    let pointer = editor.wide(at(shader, 40)?)?;
    at(pointer, offset)
}
fn required(
    editor: &mut Editor<'_, '_, '_>,
    shader: u64,
    offset: u64,
    expected: u32,
) -> Result<(), Failure> {
    let pair = special(editor, shader, offset)?;
    if editor.word(pair)? != expected {
        return Err(Failure::Shader(ShaderFailure::Registers));
    }
    Ok(())
}
fn copy(
    editor: &mut Editor<'_, '_, '_>,
    destination: u64,
    shader: u64,
    offset: u64,
) -> Result<(), Failure> {
    let source = special(editor, shader, offset)?;
    let value = editor.wide(source)?;
    editor.write(destination, &value.to_le_bytes())
}
pub(in crate::gpu) fn create(
    memory: &mut PausedNativeMemory<'_>,
    args: [u64; 6],
    used: &mut u64,
    maximum: u64,
) -> Result<u64, Failure> {
    let [context, user, hull, geometry, primitive, ..] = args;
    // This defined empty operation does not dereference any shader argument.
    if context == 0 && user == 0 {
        return Ok(0);
    }
    if geometry == 0 {
        return Err(Failure::Shader(ShaderFailure::NullArgument));
    }
    let mut editor = Editor::new(memory, used, maximum);
    if hull != 0 && editor.byte(at(hull, 90)?)? != 3 {
        return Err(Failure::Shader(ShaderFailure::Stage));
    }
    if editor.byte(at(geometry, 90)?)? != 2 {
        return Err(Failure::Shader(ShaderFailure::Stage));
    }
    let primitive = primitive as u32;
    if context != 0 {
        if hull != 0 {
            required(&mut editor, hull, 8, 0x2d5)?;
            required(&mut editor, hull, 32, 0x29b)?;
        }
        required(&mut editor, geometry, 8, 0x2d5)?;
        required(&mut editor, geometry, 32, 0x29b)?;
        copy(&mut editor, context, geometry, 8)?;
        if editor.word(at(context, 4)?)? & 0x20 != 0 {
            copy(&mut editor, at(context, 8)?, geometry, 32)?;
        } else {
            editor.write(at(context, 8)?, &0x29bu32.to_le_bytes())?;
            editor.write(at(context, 12)?, &output_primitive(primitive).to_le_bytes())?;
        }
        if hull != 0 {
            let source = special(&mut editor, hull, 12)?;
            let hull_stages = editor.word(source)?;
            let combined = editor.word(at(context, 4)?)? | hull_stages;
            editor.write(at(context, 4)?, &combined.to_le_bytes())?;
            if editor.word(at(context, 4)?)? & 0x20 == 0 {
                copy(&mut editor, at(context, 8)?, hull, 32)?;
            }
        }
    }
    if user != 0 {
        required(&mut editor, geometry, 0, 0x25b)?;
        required(&mut editor, geometry, 40, 0x262)?;
        if hull != 0 {
            required(&mut editor, hull, 40, 0x262)?;
        }
        copy(&mut editor, user, geometry, 0)?;
        copy(&mut editor, at(user, 8)?, geometry, 40)?;
        editor.write(at(user, 16)?, &0x242u32.to_le_bytes())?;
        editor.write(at(user, 20)?, &primitive.to_le_bytes())?;
        if hull != 0 {
            copy(&mut editor, at(user, 8)?, hull, 40)?;
        }
    }
    editor.finish(0)
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests;
