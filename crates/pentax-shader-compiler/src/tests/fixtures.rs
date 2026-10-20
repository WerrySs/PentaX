// SPDX-License-Identifier: GPL-2.0-only
// Original arithmetic, interface and loop fixtures for independent compiler checks.
// These are new programs, not upstream/game fixtures. No production API changes.
pub const fn scalar_move(dst: u32, src: u32) -> u32 {
    0xbe80_0000 | (dst << 16) | (3 << 8) | src
}
pub const fn scalar_add(dst: u32, a: u32, b: u32) -> u32 {
    0x8000_0000 | (dst << 16) | (b << 8) | a
}
pub const fn vector_unary(op: u32, dst: u32, src: u32) -> u32 {
    0x7e00_0000 | (dst << 17) | (op << 9) | src
}
pub const fn vector_binary(op: u32, dst: u32, a: u32, b_vgpr: u32) -> u32 {
    (op << 25) | (dst << 17) | (b_vgpr << 9) | a
}
pub const fn export(target: u32, done: bool, src: [u32; 4]) -> [u32; 2] {
    [
        0xf800_0000 | (target << 4) | 15 | ((done as u32) << 11),
        src[0] | (src[1] << 8) | (src[2] << 16) | (src[3] << 24),
    ]
}
pub const fn store(data: u32, address: u32) -> [u32; 2] {
    [
        0xe000_0000 | (0x1c << 18) | (1 << 12),
        (128 << 24) | (data << 8) | address,
    ]
}
pub const END: u32 = 0xbf81_0000;

// For the pinned vertex entry mapping, v5 is VertexIndex.
// VertexIndex=3 => parameter=(3,1.5,1,1), position=(1,0,0,1).
pub fn vertex() -> Vec<u32> {
    let mut w = vec![
        vector_unary(6, 0, 261),
        vector_binary(8, 1, 240, 0),
        vector_binary(3, 2, 241, 1),
        vector_unary(1, 3, 128),
        vector_unary(1, 4, 242),
    ];
    w.extend(export(0x20, false, [0, 1, 2, 4]));
    w.extend(export(0x0c, true, [2, 3, 3, 4]));
    w.push(END);
    w
}
// Host input parameter0.x=.25 => MRT0=(.25,.5,1,1).
pub fn fragment() -> Vec<u32> {
    let mut w = vec![
        0xc800_0000,
        0xc800_0000 | (1 << 16),
        vector_binary(8, 1, 244, 0),
        vector_binary(3, 2, 240, 1),
        vector_unary(1, 4, 242),
    ];
    w.extend(export(0, true, [0, 1, 2, 4]));
    w.push(END);
    w
}
// LocalInvocationId.x=lane => storage_buffer0[lane]=lane+7.
pub fn compute() -> Vec<u32> {
    let mut w = vec![
        vector_binary(0x25, 1, 135, 0),
        vector_binary(0x1a, 2, 130, 0),
    ];
    w.extend(store(1, 2));
    w.push(END);
    w
}
// A genuine scalar loop: increment s10 from0 to4, then write storage_buffer0[0]=4.
pub fn scalar_loop() -> Vec<u32> {
    let mut w = vec![
        scalar_move(10, 128),
        scalar_move(11, 132),
        scalar_add(10, 10, 129),
        0xbf00_0000 | (0x0a << 16) | (11 << 8) | 10,
        0xbf85_fffd,
        vector_unary(1, 1, 10),
        vector_unary(1, 2, 128),
    ];
    w.extend(store(1, 2));
    w.push(END);
    w
}
