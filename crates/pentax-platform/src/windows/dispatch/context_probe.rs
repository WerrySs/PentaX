// SPDX-License-Identifier: GPL-2.0-only

//! Closed native context round-trip diagnostic.
//!
//! Three fixed original blocks preserve, change, then preserve known guest
//! state. All sixteen GPRs, all sixteen distinct XMM values, a valid nondefault
//! MXCSR with an intentionally raised masked status bit, DF/CF, the other
//! arithmetic flags and a fixed complete 128-byte guest red zone are checked.
//! A separate assembly caller verifies the Windows host
//! nonvolatile state and repairs the original host state before returning to
//! Rust, even when an observed value disagrees.
//!
//! No caller supplies bytes, a context, an entry, a host pointer or a layout.
//! This is native execution for a dedicated worker, not a sandbox. Nothing in
//! this interface accepts caller-supplied executable data.

use super::*;
use std::mem::{align_of, offset_of, size_of};

const DF: u64 = 1 << 10;
const CF: u64 = 1;
const ARITHMETIC_FLAGS: u64 = (1 << 0) | (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
const CHECKED_FLAGS: u64 = ARITHMETIC_FLAGS | DF | (1 << 1) | (1 << 9);
const GUEST_FLAGS: u64 = 0x202 | (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
const GUEST_MXCSR: u32 = 0x3f80; // All exceptions masked; round toward -infinity.
const HOST_MXCSR: u32 = 0x5f80; // All exceptions masked; round toward +infinity.
const HOST_X87_CW: u16 = 0x027f; // Masked exceptions, double precision.

/// A private assembly record. Expected and observed state have disjoint fixed
/// storage; none of its scalar addresses are ever exposed through a public API.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct HostProbeState {
    expected_gpr: [u64; 8],
    expected_xmm: [u8; 160],
    expected_mxcsr: u32,
    expected_cw: u16,
    expected_pad: u16,
    expected_rsp: u64,
    expected_base: u64,
    expected_limit: u64,
    observed_gpr: [u64; 8],
    observed_xmm: [u8; 160],
    observed_mxcsr: u32,
    observed_cw: u16,
    observed_pad: u16,
    observed_rsp: u64,
    observed_base: u64,
    observed_limit: u64,
    observed_flags: u64,
    enter: u64,
}

impl HostProbeState {
    const fn zeroed() -> Self {
        Self {
            expected_gpr: [0; 8],
            expected_xmm: [0; 160],
            expected_mxcsr: 0,
            expected_cw: 0,
            expected_pad: 0,
            expected_rsp: 0,
            expected_base: 0,
            expected_limit: 0,
            observed_gpr: [0; 8],
            observed_xmm: [0; 160],
            observed_mxcsr: 0,
            observed_cw: 0,
            observed_pad: 0,
            observed_rsp: 0,
            observed_base: 0,
            observed_limit: 0,
            observed_flags: 0,
            enter: 0,
        }
    }

    fn seeded() -> Self {
        let mut value = Self::zeroed();
        // Order: RBX, RBP, RSI, RDI, R12, R13, R14, R15.
        value.expected_gpr = [
            0x1324_3546_5768_798a,
            0x2435_4657_6879_8a9b,
            0x3546_5768_798a_9bac,
            0x4657_6879_8a9b_acbd,
            0x5768_798a_9bac_bdce,
            0x6879_8a9b_acbd_cedf,
            0x798a_9bac_bdce_dfe0,
            0x8a9b_acbd_cedf_e0f1,
        ];
        for (index, byte) in value.expected_xmm.iter_mut().enumerate() {
            *byte = (index as u8).wrapping_mul(29).wrapping_add(0x53);
        }
        value.expected_mxcsr = HOST_MXCSR;
        value.enter = super::pentax_block_enter as *const () as u64;
        value.expected_cw = HOST_X87_CW;
        value
    }
}

const _: () = assert!(offset_of!(HostProbeState, expected_gpr) == 0);
const _: () = assert!(offset_of!(HostProbeState, expected_xmm) == 64);
const _: () = assert!(offset_of!(HostProbeState, expected_mxcsr) == 224);
const _: () = assert!(offset_of!(HostProbeState, expected_cw) == 228);
const _: () = assert!(offset_of!(HostProbeState, expected_rsp) == 232);
const _: () = assert!(offset_of!(HostProbeState, expected_base) == 240);
const _: () = assert!(offset_of!(HostProbeState, expected_limit) == 248);
const _: () = assert!(offset_of!(HostProbeState, observed_gpr) == 256);
const _: () = assert!(offset_of!(HostProbeState, observed_xmm) == 320);
const _: () = assert!(offset_of!(HostProbeState, observed_mxcsr) == 480);
const _: () = assert!(offset_of!(HostProbeState, observed_cw) == 484);
const _: () = assert!(offset_of!(HostProbeState, observed_rsp) == 488);
const _: () = assert!(offset_of!(HostProbeState, observed_base) == 496);
const _: () = assert!(offset_of!(HostProbeState, observed_limit) == 504);
const _: () = assert!(offset_of!(HostProbeState, observed_flags) == 512);
const _: () = assert!(offset_of!(HostProbeState, enter) == 520);
const _: () = assert!(size_of::<HostProbeState>() == 528);
const _: () = assert!(align_of::<HostProbeState>() == 16);

#[repr(transparent)]
struct ProbeCell(UnsafeCell<HostProbeState>);

// SAFETY: Every Rust and assembly access is dominated by this module's owned
// DispatchGuard. The guard serializes the parent's CONTEXT and this record
// process-wide. Assembly runs synchronously on that same thread, and no Rust
// reference to this record remains live across the call.
unsafe impl Sync for ProbeCell {}

static HOST_PROBE: ProbeCell = ProbeCell(UnsafeCell::new(HostProbeState::zeroed()));

struct ProbeReset;

impl Drop for ProbeReset {
    fn drop(&mut self) {
        // SAFETY: Declared after DispatchGuard and therefore dropped before it.
        // No bridge call remains live; clear private saved host addresses too.
        unsafe { *HOST_PROBE.0.get() = HostProbeState::zeroed() };
    }
}

// Windows call frame:
//   [rsp+0..32) shadow space; MXCSR+32; x87 CW+36;
//   XMM6..15 at +48..208; saved GS base+208, limit+216.
// Eight pushed host nonvolatile GPRs plus 248 bytes make RSP 16-byte aligned
// before CALL. The extra slots up to +248 are private padding.
//
// Preprobe one 4 KiB host page BEFORE saving GS bounds. Rust gates this
// diagnostic to a 4 KiB host page. This covers the current parent bridge's
// entire host frame, including its flag PUSH slot (<4 KiB below CALL RSP).
// Any future parent prologue exceeding this bound must update this probe too.
//
// After CALL, capture every nonvolatile before clobbering a scratch register.
// Capture actual RSP through RIP, then repair RSP from the private expected
// record before touching the host frame. Capture actual GS, repair GS before
// PUSHFQ, record DF, and CLD before any possible Rust/CRT operation. Finally
// restore the original caller's complete nonvolatile/control state.
//
// No Rust callback, OS API, guest-stack PUSH, or public target is involved.
std::arch::global_asm!(
    r#"
    .text
    .globl pentax_context_probe_host
    .p2align 4
pentax_context_probe_host:
    push rbx
    push rbp
    push rsi
    push rdi
    push r12
    push r13
    push r14
    push r15
    sub rsp, 248
    stmxcsr [rsp + 32]
    fnstcw [rsp + 36]
    movdqu [rsp + 48], xmm6
    movdqu [rsp + 64], xmm7
    movdqu [rsp + 80], xmm8
    movdqu [rsp + 96], xmm9
    movdqu [rsp + 112], xmm10
    movdqu [rsp + 128], xmm11
    movdqu [rsp + 144], xmm12
    movdqu [rsp + 160], xmm13
    movdqu [rsp + 176], xmm14
    movdqu [rsp + 192], xmm15
    mov byte ptr [rsp], 0
    sub rsp, 4096
    mov byte ptr [rsp], 0
    add rsp, 4096
    mov rax, gs:[8]
    mov [rsp + 208], rax
    mov [rip + {probe} + 240], rax
    mov rax, gs:[0x10]
    mov [rsp + 216], rax
    mov [rip + {probe} + 248], rax
    mov [rip + {probe} + 232], rsp
    mov rbx, [rip + {probe}]
    mov rbp, [rip + {probe} + 8]
    mov rsi, [rip + {probe} + 16]
    mov rdi, [rip + {probe} + 24]
    mov r12, [rip + {probe} + 32]
    mov r13, [rip + {probe} + 40]
    mov r14, [rip + {probe} + 48]
    mov r15, [rip + {probe} + 56]
    movdqu xmm6, [rip + {probe} + 64]
    movdqu xmm7, [rip + {probe} + 80]
    movdqu xmm8, [rip + {probe} + 96]
    movdqu xmm9, [rip + {probe} + 112]
    movdqu xmm10, [rip + {probe} + 128]
    movdqu xmm11, [rip + {probe} + 144]
    movdqu xmm12, [rip + {probe} + 160]
    movdqu xmm13, [rip + {probe} + 176]
    movdqu xmm14, [rip + {probe} + 192]
    movdqu xmm15, [rip + {probe} + 208]
    ldmxcsr [rip + {probe} + 224]
    fldcw [rip + {probe} + 228]
    cld
    call qword ptr [rip + {probe} + 520]
    mov [rip + {probe} + 256], rbx
    mov [rip + {probe} + 264], rbp
    mov [rip + {probe} + 272], rsi
    mov [rip + {probe} + 280], rdi
    mov [rip + {probe} + 288], r12
    mov [rip + {probe} + 296], r13
    mov [rip + {probe} + 304], r14
    mov [rip + {probe} + 312], r15
    mov [rip + {probe} + 488], rsp
    mov rsp, [rip + {probe} + 232]
    mov rax, gs:[8]
    mov [rip + {probe} + 496], rax
    mov rax, gs:[0x10]
    mov [rip + {probe} + 504], rax
    mov rax, [rsp + 208]
    mov gs:[8], rax
    mov rax, [rsp + 216]
    mov gs:[0x10], rax
    pushfq
    pop rax
    mov [rip + {probe} + 512], rax
    cld
    stmxcsr [rip + {probe} + 480]
    fnstcw [rip + {probe} + 484]
    movdqu [rip + {probe} + 320], xmm6
    movdqu [rip + {probe} + 336], xmm7
    movdqu [rip + {probe} + 352], xmm8
    movdqu [rip + {probe} + 368], xmm9
    movdqu [rip + {probe} + 384], xmm10
    movdqu [rip + {probe} + 400], xmm11
    movdqu [rip + {probe} + 416], xmm12
    movdqu [rip + {probe} + 432], xmm13
    movdqu [rip + {probe} + 448], xmm14
    movdqu [rip + {probe} + 464], xmm15
    ldmxcsr [rsp + 32]
    fldcw [rsp + 36]
    movdqu xmm6, [rsp + 48]
    movdqu xmm7, [rsp + 64]
    movdqu xmm8, [rsp + 80]
    movdqu xmm9, [rsp + 96]
    movdqu xmm10, [rsp + 112]
    movdqu xmm11, [rsp + 128]
    movdqu xmm12, [rsp + 144]
    movdqu xmm13, [rsp + 160]
    movdqu xmm14, [rsp + 176]
    movdqu xmm15, [rsp + 192]
    add rsp, 248
    pop r15
    pop r14
    pop r13
    pop r12
    pop rdi
    pop rsi
    pop rbp
    pop rbx
    ret
"#,
    probe = sym HOST_PROBE,
);

unsafe extern "C" {
    fn pentax_context_probe_host();
}

fn require(condition: bool, stage: &'static str) -> Result<(), PlatformError> {
    if condition {
        Ok(())
    } else {
        Err(PlatformError::NativeDispatchFailed { stage })
    }
}

fn guest_seed(rsp: u64, bias: u64) -> GuestContext {
    let mut context = GuestContext::zeroed();
    for (index, register) in context.gpr.iter_mut().enumerate() {
        *register = 0x9021_3243_5465_7687_u64
            .wrapping_add((index as u64).wrapping_mul(0x0102_0304_0506_0709));
    }
    context.gpr[RSP] = rsp;
    context.rflags = GUEST_FLAGS;
    context.mxcsr = GUEST_MXCSR;
    context.guest_base = bias + STACK_OFF + STACK_BYTES;
    context.guest_limit = bias + STACK_OFF;
    for (index, byte) in context.xmm.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(17).wrapping_add(0xa7);
    }
    context
}

fn changed_gpr(index: usize) -> u64 {
    0x3142_5364_7586_97a8_u64.wrapping_add((index as u64).wrapping_mul(0x0305_0709_0b0d_0f11))
}

/// Build only the diagnostic's fixed original instructions. This is a source
/// fixture, not an instruction parser or a caller-controlled code generator.
fn original_program() -> (Vec<u8>, [u64; 3], [u64; 3]) {
    let mut code = Vec::with_capacity(512);
    let mut starts = [0; 3];
    let mut next = [0; 3];

    // First observe the distinct initial state without overwriting it.
    code.extend_from_slice(&[0x90, 0xfd, 0xf9, 0xeb, 0]); // NOP; STD; STC; JMP +0.
    next[0] = code.len() as u64;
    starts[1] = next[0];

    // Then change every non-RSP GPR in hardware. MOV does not change flags.
    for index in 0..16 {
        if index == RSP {
            continue;
        }
        code.extend_from_slice(&[0x48 | u8::from(index >= 8), 0xb8 | (index as u8 & 7)]);
        code.extend_from_slice(&changed_gpr(index).to_le_bytes());
    }
    // All guest SIMD exceptions are masked. Intel SDM Vol. 1, sections
    // 4.9.1.1 and 11.5.2.1: 0/0 raises only the sticky invalid status flag
    // (MXCSR.IE), rather than a host exception while IM remains set. The
    // following integer SIMD writes discard the floating-point result.
    // https://cdrdv2-public.intel.com/843827/253665-sdm-vol-1-dec-24.pdf
    code.extend_from_slice(&[0x66, 0x0f, 0xef, 0xc0]); // PXOR XMM0,XMM0.
    code.extend_from_slice(&[0x0f, 0x5e, 0xc0]); // DIVPS XMM0,XMM0.
    for index in 0_u8..16 {
        code.push(0x66);
        if index >= 8 {
            code.push(0x45); // REX.R/B: both register operands use XMM8..15.
        }
        code.extend_from_slice(&[0x0f, 0x76, 0xc0 | ((index & 7) << 3) | (index & 7)]);
        code.push(0x66);
        if index >= 8 {
            code.push(0x41); // REX.B only: ModRM.reg is fixed opcode /2.
        }
        code.extend_from_slice(&[0x0f, 0x72, 0xd0 | (index & 7), index]);
        // PCMPEQD self; PSRLD by index. Each XMM now has a different value.
    }
    code.extend_from_slice(&[0x48, 0x8d, 0x64, 0x24, 0xf0]); // LEA RSP,[RSP-16].
    code.extend_from_slice(&[0xeb, 0]);
    next[1] = code.len() as u64;
    starts[2] = next[1];

    // The last round must reload changed values, not the initial host state.
    code.extend_from_slice(&[0x90, 0x48, 0x8d, 0x64, 0x24, 0x10]); // NOP; LEA RSP,[RSP+16].
    code.extend_from_slice(&[0xfc, 0xf8, 0xeb, 0]); // CLD; CLC; JMP +0.
    next[2] = code.len() as u64;
    code.push(0xc3); // Unexecuted target keeps every checked exit in the image.
    starts[0] = 0;
    (code, starts, next)
}

fn expected_guest(seed: GuestContext, round: usize) -> GuestContext {
    let mut expected = seed;
    if round != 0 {
        for (index, value) in expected.gpr.iter_mut().enumerate() {
            if index != RSP {
                *value = changed_gpr(index);
            }
        }
        for index in 0..16 {
            let lane = (u32::MAX >> index).to_le_bytes();
            for element in 0..4 {
                let start = index * 16 + element * 4;
                expected.xmm[start..start + 4].copy_from_slice(&lane);
            }
        }
        expected.mxcsr |= 1; // Hardware DIVPS invalid-operation sticky flag.
    }
    if round == 1 {
        expected.gpr[RSP] -= 16; // Stage_guest returned an owned two-page stack.
    }
    expected.rflags = if round < 2 {
        GUEST_FLAGS | DF | CF
    } else {
        GUEST_FLAGS
    };
    expected
}

fn verify_host(observed: &HostProbeState) -> Result<(), PlatformError> {
    require(
        observed.observed_gpr == observed.expected_gpr,
        "context-probe-host-gpr",
    )?;
    require(
        observed.observed_xmm == observed.expected_xmm,
        "context-probe-host-xmm",
    )?;
    require(
        observed.observed_mxcsr == observed.expected_mxcsr,
        "context-probe-host-mxcsr",
    )?;
    require(
        observed.observed_cw == observed.expected_cw,
        "context-probe-host-x87",
    )?;
    require(
        observed.expected_rsp != 0 && observed.observed_rsp == observed.expected_rsp,
        "context-probe-host-rsp",
    )?;
    require(
        observed.expected_limit <= observed.expected_rsp
            && observed.expected_rsp < observed.expected_base
            && observed.observed_base == observed.expected_base
            && observed.observed_limit == observed.expected_limit,
        "context-probe-host-teb",
    )?;
    require(observed.observed_flags & DF == 0, "context-probe-host-df")
}

/// Execute only this fixed original diagnostic. The parent wires it from the
/// isolated worker before its principal fixture.
pub(super) fn verify_context_roundtrip() -> Result<(), PlatformError> {
    verify_context_roundtrip_mode(super::ymm_bridge::VectorMode::Sse2)
}

pub(super) fn verify_context_roundtrip_mode(
    mode: super::ymm_bridge::VectorMode,
) -> Result<(), PlatformError> {
    // This probe touches one host page before the inner bridge. Do not guess a
    // page geometry, or silently claim coverage of a larger future prologue.
    require(
        crate::host_layout()?.page_size == 4096,
        "context-probe-host-page",
    )?;
    let _guard = DispatchGuard::acquire()?;
    let _reset = ProbeReset;

    // Every transfer is a typed exit; no guest raw JMP reaches the cache.
    // Independent counts prevent an unsupported instruction being mistaken
    // for the intended terminating branch before all observations are made.
    const EMITTED: [usize; 3] = [3, 50, 4];
    let (code, starts, next) = original_program();
    require(
        code.len() == 340 && starts == [0, 5, 329] && next == [5, 329, 339],
        "context-probe-source-layout",
    )?;
    let image = build_elf(&code);
    let view = open_executable(&image).map_err(|_| PlatformError::NativeDispatchFailed {
        stage: "context-probe-image",
    })?;
    let arena = fresh_guest_space()?;
    let bias = arena
        .reserved_ranges()
        .next()
        .ok_or(PlatformError::OutOfBounds)?
        .address;
    let inputs = [ModuleInput::new(view, bias)];
    let plan = plan_symbolic(&inputs, &SymbolicLimits::default()).map_err(|_| {
        PlatformError::NativeDispatchFailed {
            stage: "context-probe-plan",
        }
    })?;
    let (mut memory, rsp) = stage_guest(&plan, arena)?;
    let mut cache = CodeCache::reserve_near(bias, CACHE_BYTES)?;
    let mut seed = guest_seed(rsp, bias);
    let low = seed.xmm;
    let mut upper = [0; 256];
    for (index, byte) in upper.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(19).wrapping_add(0x53);
    }
    mode.load(&mut seed, &low, &upper);
    let red_zone_address = rsp.checked_sub(128).ok_or(PlatformError::OutOfBounds)?;
    let mut red_zone = [0_u8; 128];
    for (index, byte) in red_zone.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(37).wrapping_add(0x69);
    }
    memory
        .pause()
        .map_err(native)?
        .write(red_zone_address, &red_zone)
        .map_err(native)?;
    // SAFETY: Exclusive DispatchGuard, no assembly call live, exact initialized
    // primitive fields only. No mutable Rust reference survives this statement.
    unsafe { *CONTEXT.0.get() = seed };

    for round in 0..3 {
        let guest_pc = bias + ENTRY as u64 + starts[round];
        let block = plan_block(
            &plan,
            0,
            guest_pc,
            CachePlacement {
                address: cache.address(),
                capacity: cache.capacity(),
            },
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default(),
        )
        .map_err(|_| PlatformError::NativeDispatchFailed {
            stage: "context-probe-block",
        })?;
        require(
            matches!(block.exit().kind(), ExitKind::Jump { target: ControlTarget::Direct(target) }
            if target == bias + ENTRY as u64 + next[round]),
            "context-probe-exit",
        )?;
        require(
            block
                .instructions()
                .iter()
                .filter(|instruction| instruction.emitted_len() != 0)
                .count()
                == EMITTED[round],
            "context-probe-body",
        )?;
        // Same publication/host-feature/source/emission attestation path as
        // run_one_block, with a stronger native caller around its enter call.
        publish_block_with_mode(&mut cache, &mut memory, &block, guest_pc, mode)?;
        // SAFETY: Guard held, owned RX cache just attested. The private wrapper
        // captures observations and restores host RSP/TEB/DF/nonvolatile state
        // before returning; unexpected native faults remain worker-fatal.
        unsafe {
            (*CONTEXT.0.get()).rip = cache.address();
            *HOST_PROBE.0.get() = HostProbeState::seeded();
            (*HOST_PROBE.0.get()).enter = mode.entry() as *const () as u64;
            pentax_context_probe_host();
        }
        // SAFETY: Same exclusive guard, assembly returned and no Rust borrow
        // aliases either private record while assembly modifies it.
        let (mut actual, host) = unsafe { (*CONTEXT.0.get(), *HOST_PROBE.0.get()) };
        let snapshot = actual;
        let mut actual_upper = upper;
        mode.save(&snapshot, &mut actual.xmm, &mut actual_upper);
        require(actual_upper == upper, "context-probe-guest-ymm-upper")?;
        verify_host(&host)?;
        let expected = expected_guest(seed, round);
        require(actual.gpr == expected.gpr, "context-probe-guest-gpr")?;
        require(actual.xmm == expected.xmm, "context-probe-guest-xmm")?;
        require(actual.mxcsr == expected.mxcsr, "context-probe-guest-mxcsr")?;
        require(
            actual.rflags & CHECKED_FLAGS == expected.rflags & CHECKED_FLAGS,
            "context-probe-guest-flags",
        )?;
        require(
            actual.guest_base == seed.guest_base && actual.guest_limit == seed.guest_limit,
            "context-probe-guest-bounds",
        )?;
        let mut actual_red_zone = [0_u8; 128];
        memory
            .pause()
            .map_err(native)?
            .read(red_zone_address, &mut actual_red_zone)
            .map_err(native)?;
        require(actual_red_zone == red_zone, "context-probe-red-zone")?;
    }
    Ok(())
}
