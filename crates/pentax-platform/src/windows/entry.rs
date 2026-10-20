// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: fixed original fixture, owned guarded stack, real
// TEB stack bounds, full Windows nonvolatile saves, and normal-return checks.
// Ported from WerrySs/KytyPS5-Performance commit
// 209dffe9032d72749257ce18680d506b891284de, src/loader/runtimeLinker.cpp,
// EntryParams/RunEntry/Execute. Preserve LICENSES/Kyty-MIT.txt and GPLv2.

//! This private bridge restores host state only on a normal SysV return.
//! It supplies no unwind guarantee across the switched stack and no containment.
//! TEB bounds describe the fully committed owned usable interval; inaccessible
//! reservations bound it. They are never zeroed and no automatic growth is used.

use super::stack::GuestStack;
use crate::{
    ExecutableGuestMemory, GUEST_PAGE_SIZE, GuestReservation, OriginalFixtureResult, PlatformError,
};
use std::arch::{asm, global_asm};
use std::cell::Cell;

#[repr(C)]
struct EntryCall {
    entry: u64,
    params: u64,
    atexit: u64,
    rsp: u64,
    rbp: u64,
    base: u64,
    limit: u64,
    host: HostStackRestoration,
}

const _: () = assert!(std::mem::offset_of!(EntryCall, limit) == 48);
const _: () = assert!(std::mem::offset_of!(EntryCall, host) == 56);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StackBounds {
    base: u64,
    limit: u64,
}

/// Private observations taken after the host frame is touched and immediately
/// after restoring it. Windows may normally grow the host stack before this
/// boundary; a Rust snapshot taken before the bridge prologue is not equivalent.
#[repr(C)]
#[derive(Default)]
struct HostStackRestoration {
    saved: StackBounds,
    restored: StackBounds,
    saved_rsp: u64,
    restored_rsp: u64,
}

const _: () = assert!(std::mem::offset_of!(HostStackRestoration, restored) == 16);
const _: () = assert!(std::mem::offset_of!(HostStackRestoration, saved_rsp) == 32);
const _: () = assert!(std::mem::offset_of!(HostStackRestoration, restored_rsp) == 40);

impl HostStackRestoration {
    fn is_restored(&self) -> bool {
        self.saved == self.restored
            && self.saved.base != 0
            && self.saved.limit <= self.saved_rsp
            && self.saved_rsp < self.saved.base
            && self.saved_rsp == self.restored_rsp
    }
}

global_asm!(
    r#"
    .text
    .globl pentax_owned_stack_entry
pentax_owned_stack_entry:
    push rbx
    push rbp
    push rdi
    push rsi
    push r12
    push r13
    push r14
    push r15
    sub rsp, 216
    mov [rsp], rcx
    mov rax, gs:[0x08]
    mov [rsp + 8], rax
    mov rax, gs:[0x10]
    mov [rsp + 16], rax
    stmxcsr [rsp + 24]
    fnstcw [rsp + 28]
    movdqu [rsp + 32], xmm6
    movdqu [rsp + 48], xmm7
    movdqu [rsp + 64], xmm8
    movdqu [rsp + 80], xmm9
    movdqu [rsp + 96], xmm10
    movdqu [rsp + 112], xmm11
    movdqu [rsp + 128], xmm12
    movdqu [rsp + 144], xmm13
    movdqu [rsp + 160], xmm14
    movdqu [rsp + 176], xmm15
    mov r12, rsp
    mov r13, rcx
    mov rdx, [rsp + 8]
    mov [r13 + 56], rdx
    mov rdx, [rsp + 16]
    mov [r13 + 64], rdx
    mov [r13 + 88], rsp
    mov rax, [r13 + 40]
    mov gs:[0x08], rax
    mov rax, [r13 + 48]
    mov gs:[0x10], rax
    mov rdi, [r13 + 8]
    mov rsi, [r13 + 16]
    mov rax, [r13]
    mov rbp, [r13 + 32]
    mov rsp, [r13 + 24]
    call rax
    mov rsp, r12
    mov rdx, [rsp + 8]
    mov gs:[0x08], rdx
    mov rdx, [rsp + 16]
    mov gs:[0x10], rdx
    mov rdx, gs:[0x08]
    mov [r13 + 72], rdx
    mov rdx, gs:[0x10]
    mov [r13 + 80], rdx
    mov [r13 + 96], rsp
    ldmxcsr [rsp + 24]
    fldcw [rsp + 28]
    movdqu xmm6, [rsp + 32]
    movdqu xmm7, [rsp + 48]
    movdqu xmm8, [rsp + 64]
    movdqu xmm9, [rsp + 80]
    movdqu xmm10, [rsp + 96]
    movdqu xmm11, [rsp + 112]
    movdqu xmm12, [rsp + 128]
    movdqu xmm13, [rsp + 144]
    movdqu xmm14, [rsp + 160]
    movdqu xmm15, [rsp + 176]
    add rsp, 216
    pop r15
    pop r14
    pop r13
    pop r12
    pop rsi
    pop rdi
    pop rbp
    pop rbx
    ret

    .globl pentax_original_stack_fixture
pentax_original_stack_fixture:
    cmp dword ptr [rdi], 1
    jne .Lpentax_entry_fail
    cmp dword ptr [rdi + 4], 0
    jne .Lpentax_entry_fail
    cmp qword ptr [rdi + 16], 0
    jne .Lpentax_entry_fail
    cmp qword ptr [rdi + 24], 0
    jne .Lpentax_entry_fail
    mov rax, [rdi + 8]
    test rax, rax
    je .Lpentax_entry_fail
    cmp dword ptr [rax], 0x746e6550
    jne .Lpentax_entry_fail
    cmp word ptr [rax + 4], 0x5861
    jne .Lpentax_entry_fail
    cmp byte ptr [rax + 6], 0
    jne .Lpentax_entry_fail
    mov rax, rsp
    and eax, 15
    cmp eax, 8
    jne .Lpentax_entry_fail
    lea rax, [rsp + 8]
    cmp rbp, rax
    jne .Lpentax_entry_fail
    cmp qword ptr [rbp], 0
    jne .Lpentax_entry_fail
    cmp qword ptr [rbp + 8], 0
    jne .Lpentax_entry_fail
    cmp qword ptr [rsp], 0
    je .Lpentax_entry_fail
    mov qword ptr [rsp - 8], -1
    mov qword ptr [rsp - 128], 23
    cmp qword ptr [rsp - 8], -1
    jne .Lpentax_entry_fail
    cmp qword ptr [rsp - 128], 23
    jne .Lpentax_entry_fail
    test rsi, rsi
    je .Lpentax_entry_fail
    sub rsp, 8
    call rsi
    add rsp, 8
    pxor xmm6, xmm6
    pxor xmm7, xmm7
    pxor xmm8, xmm8
    pxor xmm9, xmm9
    pxor xmm10, xmm10
    pxor xmm11, xmm11
    pxor xmm12, xmm12
    pxor xmm13, xmm13
    pxor xmm14, xmm14
    pxor xmm15, xmm15
    xor edi, edi
    xor esi, esi
    mov eax, 42
    ret
.Lpentax_entry_fail:
    xor eax, eax
    ret
    .globl pentax_original_stack_fixture_end
pentax_original_stack_fixture_end:
"#
);

unsafe extern "system" {
    fn pentax_owned_stack_entry(call: *mut EntryCall) -> u64;
    fn pentax_original_stack_fixture();
    fn pentax_original_stack_fixture_end();
}

#[derive(Clone, Copy)]
struct CallbackState {
    base: u64,
    limit: u64,
    count: u32,
    verified: bool,
}

std::thread_local! {
    static CALLBACK: Cell<Option<CallbackState>> = const { Cell::new(None) };
}

struct CallbackScope;

impl CallbackScope {
    fn arm(stack: &GuestStack) -> Result<Self, PlatformError> {
        let armed = CALLBACK.try_with(|slot| {
            if slot.get().is_some() {
                return false;
            }
            slot.set(Some(CallbackState {
                base: stack.base,
                limit: stack.limit,
                count: 0,
                verified: true,
            }));
            true
        });
        if !matches!(armed, Ok(true)) {
            return Err(PlatformError::TrapScopeUnavailable);
        }
        Ok(Self)
    }

    fn snapshot(&self) -> Option<CallbackState> {
        CALLBACK.try_with(Cell::get).ok().flatten()
    }
}

impl Drop for CallbackScope {
    fn drop(&mut self) {
        let _ = CALLBACK.try_with(|slot| slot.set(None));
    }
}

fn teb_bounds() -> (u64, u64) {
    let base;
    let limit;
    // SAFETY: GS identifies this Windows thread's TEB. These are read-only
    // StackBase/StackLimit observations; no FS/TLS base or other thread is changed.
    unsafe {
        asm!(
            "mov {}, gs:[0x08]",
            "mov {}, gs:[0x10]",
            out(reg) base,
            out(reg) limit,
            options(nostack, preserves_flags, readonly),
        );
    }
    (base, limit)
}

extern "sysv64" fn exit_callback() {
    let bounds = teb_bounds();
    let rsp: u64;
    // SAFETY: Observe the current stack register without touching memory/state.
    unsafe { asm!("mov {}, rsp", out(reg) rsp, options(nostack, nomem, preserves_flags)) };
    let _ = CALLBACK.try_with(|slot| {
        if let Some(mut state) = slot.get() {
            state.count = state.count.saturating_add(1);
            state.verified &=
                bounds == (state.base, state.limit) && state.limit <= rsp && rsp < state.base;
            slot.set(Some(state));
        }
    });
}

/// Only this constructor can publish an entry accepted by the private bridge.
struct AttestedEntry {
    executable: ExecutableGuestMemory,
}

impl AttestedEntry {
    fn prepare() -> Result<Self, PlatformError> {
        let start = pentax_original_stack_fixture as *const () as usize;
        let end = pentax_original_stack_fixture_end as *const () as usize;
        let length = end
            .checked_sub(start)
            .filter(|length| *length > 0 && *length <= GUEST_PAGE_SIZE)
            .ok_or(PlatformError::InvalidFixtureTrap)?;
        // SAFETY: The two labels bound the immutable original assembly above.
        // It contains only internal relative branches and an RSI callback, so
        // copying this exact bounded span preserves all code references. No
        // caller bytes, address, callback, or executable input can reach here.
        let original = unsafe { std::slice::from_raw_parts(start as *const u8, length) };
        let mut staging = GuestReservation::reserve(GUEST_PAGE_SIZE)?.commit_writable()?;
        staging.write(0, original)?;
        Ok(Self {
            executable: staging.seal_executable()?,
        })
    }

    fn request(&self, stack: &GuestStack) -> EntryCall {
        EntryCall {
            entry: self.executable.address(),
            params: stack.params,
            atexit: exit_callback as *const () as usize as u64,
            rsp: stack.rsp,
            rbp: stack.rbp,
            base: stack.base,
            limit: stack.limit,
            host: HostStackRestoration::default(),
        }
    }

    fn call(&self, stack: &GuestStack) -> Result<OriginalFixtureResult, PlatformError> {
        let scope = CallbackScope::arm(stack)?;
        let mut call = self.request(stack);
        // SAFETY: self attests the only closed code span; stack uniquely owns
        // its live committed RW/NX region, initialized parameters, red zone and
        // root frame. The original callee preserves SysV R12/R13 used to recover
        // the saved host frame. The assembly preserves Windows nonvolatiles,
        // control state and TEB bounds on normal return, recording exact host
        // boundary observations in the uniquely borrowed private request. Its
        // first host-frame write precedes the saved TEB snapshot, so normal
        // guard-page growth in the prologue is included. Faults remain fatal to
        // the dedicated worker; this call grants no unwind/containment guarantee.
        let value = unsafe { pentax_owned_stack_entry(&mut call) };
        let state = scope.snapshot();
        let callbacks = state.map_or(0, |state| state.count);
        if value != 42
            || callbacks != 1
            || state.is_none_or(|state| !state.verified)
            || !call.host.is_restored()
        {
            return Err(PlatformError::UnexpectedGuestStackEntryResult { value, callbacks });
        }
        Ok(OriginalFixtureResult { value, traps: 0 })
    }
}

pub(super) fn execute() -> Result<OriginalFixtureResult, PlatformError> {
    let entry = AttestedEntry::prepare()?;
    let stack = GuestStack::new()?;
    entry.call(&stack)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_entry_checks_arguments_stack_red_zone_callback_and_restores_teb() {
        let host_base = teb_bounds().0;
        grow_host_stack();
        for _ in 0..2 {
            assert_eq!(
                execute(),
                Ok(OriginalFixtureResult {
                    value: 42,
                    traps: 0
                })
            );
            // Preparation and ordinary host calls may grow StackLimit. The
            // bridge itself checks exact saved/restored bounds and RSP after
            // its own frame was touched; the original host stack stays selected.
            assert_eq!(teb_bounds().0, host_base);
        }
        assert_eq!(
            crate::execute_original_fixture(crate::OriginalFixture::Return42),
            Ok(42)
        );
        assert_eq!(
            crate::execute_original_fixture(crate::OriginalFixture::GuestStackEntry42),
            Ok(42)
        );
    }

    #[inline(never)]
    fn grow_host_stack() {
        // Let the compiler's normal Windows stack probes handle every guard
        // page. This may grow a cold test-thread stack before entry preparation;
        // an already committed host stack needs no growth. No TEB is modified.
        let mut pages = [0_u8; 64 * 1024];
        for page in pages.chunks_mut(4096) {
            std::hint::black_box(page)[0] = 1;
        }
        std::hint::black_box(&mut pages);
    }

    #[test]
    fn callback_scope_is_thread_bound_and_cleared_on_drop() {
        let stack = GuestStack::new().unwrap();
        let first = CallbackScope::arm(&stack).unwrap();
        assert!(matches!(
            CallbackScope::arm(&stack),
            Err(PlatformError::TrapScopeUnavailable)
        ));
        drop(first);
        assert!(CallbackScope::arm(&stack).is_ok());
    }

    #[test]
    fn normal_return_preserves_windows_rdi_rsi_and_all_nonvolatile_xmm_registers() {
        use std::arch::x86_64::__m128i;

        let entry = AttestedEntry::prepare().unwrap();
        let stack = GuestStack::new().unwrap();
        let scope = CallbackScope::arm(&stack).unwrap();
        let mut call = entry.request(&stack);
        // SAFETY: Every bit pattern is valid for this integer SIMD register type.
        let vector: __m128i = unsafe { std::mem::transmute([u64::MAX; 2]) };
        let (mut x6, mut x7, mut x8, mut x9, mut x10) = (vector, vector, vector, vector, vector);
        let (mut x11, mut x12, mut x13, mut x14, mut x15) =
            (vector, vector, vector, vector, vector);
        let rdi: u64;
        let rsi: u64;
        let value: u64;
        // SAFETY: The only target is the private bridge and its attested fixed
        // fixture with the live owned stack. This nonleaf Windows test has an
        // aligned host stack; reserve the required 32-byte shadow space. Explicit
        // outputs let the compiler preserve its own nonvolatile register values.
        // The original callee deliberately zeros RDI/RSI and XMM6..15, so these
        // sentinels independently observe the bridge's actual restoration.
        unsafe {
            asm!(
                "sub rsp, 32",
                "call {bridge}",
                "add rsp, 32",
                bridge = sym pentax_owned_stack_entry,
                in("rcx") &mut call,
                inlateout("rdi") 0x1122_3344_5566_7788_u64 => rdi,
                inlateout("rsi") 0x8877_6655_4433_2211_u64 => rsi,
                inlateout("xmm6") x6,
                inlateout("xmm7") x7,
                inlateout("xmm8") x8,
                inlateout("xmm9") x9,
                inlateout("xmm10") x10,
                inlateout("xmm11") x11,
                inlateout("xmm12") x12,
                inlateout("xmm13") x13,
                inlateout("xmm14") x14,
                inlateout("xmm15") x15,
                lateout("rax") value,
                clobber_abi("system"),
            );
        }
        assert_eq!(value, 42);
        assert!(call.host.is_restored());
        // Even a plausible one-byte stack-limit change at the actual bridge
        // boundary must fail. Ordinary growth outside that boundary does not
        // relax exact restoration of the bounds saved by this invocation.
        call.host.restored.limit = call.host.saved.limit - 1;
        assert!(!call.host.is_restored());
        assert_eq!(rdi, 0x1122_3344_5566_7788);
        assert_eq!(rsi, 0x8877_6655_4433_2211);
        for value in [x6, x7, x8, x9, x10, x11, x12, x13, x14, x15] {
            // SAFETY: Integer SIMD values have the same initialized 16-byte bits.
            let words: [u64; 2] = unsafe { std::mem::transmute(value) };
            assert_eq!(words, [u64::MAX; 2]);
        }
        assert!(
            scope
                .snapshot()
                .is_some_and(|state| state.verified && state.count == 1)
        );
    }
}
