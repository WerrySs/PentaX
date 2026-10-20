// SPDX-License-Identifier: GPL-2.0-only
//! Original capacity/phase proofs; no guest or upstream shader fixtures.
use super::*;

#[test]
fn retained_products_count_spare_recursive_capacity_and_exclude_dead_compiler_arenas() {
    let limits = Limits::default();
    let program = ShaderProgram::decode(&fixtures::vertex(), ShaderStage::Vertex, &limits).unwrap();
    let compiled = compile(&program, &profile(ShaderStage::Vertex), &limits).unwrap();
    assert!(program.retained_host_bytes() > program.source_words().len() * 4);
    assert!(compiled.retained_host_bytes() > compiled.spirv().len() * 4);
    assert!(compiled.retained_host_bytes() < compiled.charged_bytes());

    // Empty-but-reserved manifest tables are still owned. Counting len() or
    // omitting the nested user-SGPR vector would lose real retained capacity.
    let mut manifest = compiled.manifest().clone();
    manifest.buffers.clear();
    manifest.buffers.reserve_exact(7);
    manifest.inputs.clear();
    manifest.inputs.reserve_exact(9);
    manifest.outputs.clear();
    manifest.outputs.reserve_exact(11);
    let mut user_sgprs = Vec::with_capacity(13);
    user_sgprs.push(2);
    manifest.push_data = Some(PushData {
        start_dword: 0,
        user_sgprs,
        memory_offset_dword: 0,
        memory_offset_count: 0,
    });
    let expected = manifest.buffers.capacity() * std::mem::size_of::<BufferElement>()
        + manifest.inputs.capacity() * std::mem::size_of::<InputInterface>()
        + manifest.outputs.capacity() * std::mem::size_of::<OutputInterface>()
        + manifest.push_data.as_ref().unwrap().user_sgprs.capacity();
    assert_eq!(manifest.retained_host_bytes(), expected);
    let mut words = Vec::with_capacity(37);
    words.push(0x0723_0203);
    let capacity = words.capacity();
    let owned = CompiledShader {
        words,
        manifest,
        work: 0,
        retained_bytes: usize::MAX,
    };
    assert_eq!(owned.retained_host_bytes(), capacity * 4 + expected);
    assert_eq!(owned.charged_bytes(), usize::MAX); // Independent historical API.
}
