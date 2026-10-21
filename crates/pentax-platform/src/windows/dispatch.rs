// SPDX-License-Identifier: GPL-2.0-only

//! Private Windows x86-64 translated-block execution engine and full guest
//! context bridge for one closed original diagnostic.
//!
//! # What this is
//!
//! This module hosts a *private*, reusable basic-block dispatcher. It takes the
//! immutable block plans produced by `pentax-codegen` for an original program,
//! publishes each block's ordinary re-encoded instructions into a privately
//! owned code cache, and runs them under a fully owned guest [`GuestContext`].
//! Every control transfer (`CALL`/`JMP`/`Jcc`/`RET`/`SYSCALL`) stays *mediated*
//! in Rust against owned guest memory, exactly as the planner's typed exits
//! require; none is ever copied into the cache as a raw guest transfer.
//!
//! # What this is NOT
//!
//! This is native code execution, not a sandbox and not a game runtime. The
//! safe diagnostic entry accepts only the closed
//! [`crate::OriginalFixture::NativeBlockDispatch`] program, whose complete
//! source bytes, runtime placements, owned stack/data ranges and emission
//! policy are attested before anything runs. The separate [`prepared`] module
//! reuses this bridge behind an explicit unsafe, one-block API. Its caller must
//! bind exact prepared-owner/TLS provenance and prove ordinary native operands
//! stay within live owned memory. A dedicated worker is not that proof.
//!
//! # Verified diagnostic scope
//!
//! The isolated worker checks three actual context round trips before executing
//! the original multi-block program. This covers the named original source,
//! integer/legacy SSE state and normal return only. See
//! `README-native-dispatch.md` for the remaining runtime integration.
//!
//! # Context round-trip invariants
//!
//! The bridge preserves all sixteen guest GPRs, RFLAGS (including DF), MXCSR and
//! XMM0-15 across every host round trip, and restores the Windows nonvolatile
//! registers, x87 control word, MXCSR and the TEB stack base/limit exactly.
//! The guest runs on its own owned stack; the host SysV/Windows red zone is
//! never used as a resumable exit. The exit trailer captures guest state and
//! switches to the saved host stack using only instructions that do not perturb
//! guest flags/registers before they are saved, and it never writes the guest
//! red zone.
//!
//! Primary contracts:
//! - Intel SDM Vol. 1/2 (EFLAGS, MXCSR, instruction semantics).
//! - Microsoft x64 ABI (nonvolatile registers, TEB stack base/limit).
//! - Rust inline/global assembly reference (`sym`, RIP-relative operands).

use crate::native_memory::{
    NATIVE_GUEST_PAGE, NativeAddressSpace, NativeMemoryError, NativeMemoryLimits, NativePermissions,
};
use crate::{OriginalFixtureResult, PlatformError};
use pentax_codegen::{
    BlockLimits, BlockPlan, CachePlacement, ConditionCode, ControlTarget, CpuidFeature, ExitKind,
    MemoryAccessAnalyzer, MemoryAccessKind, Register, TranslationPolicy, plan_block,
};
use pentax_linker::{ModuleInput, SymbolicLimits, SymbolicLinkPlan, plan_symbolic};
use pentax_loader::open_executable;
use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
// Reuse the parent backend's GetLastError wrapper rather than duplicating it.
use super::last_error;
use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READ, PAGE_READWRITE, VirtualAlloc,
    VirtualFree, VirtualProtect,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

#[path = "dispatch/context_probe.rs"]
mod context_probe;
#[path = "dispatch/ymm_bridge.rs"]
mod ymm_bridge;

/// Explicit unsafe one-block execution over retained prepared memory.
#[path = "dispatch/prepared.rs"]
pub mod prepared;

// ---------------------------------------------------------------------------
// Guest context layout
// ---------------------------------------------------------------------------

/// GPR slot indices follow the x86 register-number convention so that the
/// decoder's operand register numbers map directly onto context slots.
const RAX: usize = 0;
const RCX: usize = 1;
const RDX: usize = 2;
const RSP: usize = 4;
const RSI: usize = 6;
const RDI: usize = 7;

/// Owned guest register/flags/SIMD state, shared by value with the asm bridge.
///
/// `#[repr(C)]` with the fixed field order below pins the byte offsets the
/// `global_asm!` bridge encodes. The `const _` assertions keep Rust and the
/// assembly in agreement; a layout change that is not mirrored in the bridge
/// fails the build rather than silently corrupting a round trip.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct GuestContext {
    /// RAX,RCX,RDX,RBX,RSP,RBP,RSI,RDI,R8..R15 by x86 register number.
    gpr: [u64; 16],
    /// Guest resume address: the cache body the bridge jumps to on entry.
    rip: u64,
    /// Full guest RFLAGS, including DF, preserved and restored each round trip.
    rflags: u64,
    /// TEB StackBase installed for the duration of guest execution.
    guest_base: u64,
    /// TEB StackLimit installed for the duration of guest execution.
    guest_limit: u64,
    /// Host RSP captured after the bridge prologue; the trailer restores it.
    host_rsp: u64,
    /// Guest MXCSR, preserved and restored each round trip.
    mxcsr: u32,
    _pad: u32,
    /// XMM0..XMM15, preserved and restored each round trip.
    xmm: [u8; 16 * 16],
    /// Full YMM0..15 used only by the independently admitted AVX bridge. The
    /// legacy context offsets above stay fixed for the SSE2-only entry point.
    ymm: [u8; 32 * 16],
}

const CTX_RIP: usize = 128;
const CTX_RFLAGS: usize = 136;
const CTX_GUEST_BASE: usize = 144;
const CTX_GUEST_LIMIT: usize = 152;
const CTX_HOST_RSP: usize = 160;
const CTX_MXCSR: usize = 168;
const CTX_XMM0: usize = 176;
const CTX_YMM0: usize = 432;

const _: () = assert!(std::mem::offset_of!(GuestContext, gpr) == 0);
const _: () = assert!(std::mem::offset_of!(GuestContext, rip) == CTX_RIP);
const _: () = assert!(std::mem::offset_of!(GuestContext, rflags) == CTX_RFLAGS);
const _: () = assert!(std::mem::offset_of!(GuestContext, guest_base) == CTX_GUEST_BASE);
const _: () = assert!(std::mem::offset_of!(GuestContext, guest_limit) == CTX_GUEST_LIMIT);
const _: () = assert!(std::mem::offset_of!(GuestContext, host_rsp) == CTX_HOST_RSP);
const _: () = assert!(std::mem::offset_of!(GuestContext, mxcsr) == CTX_MXCSR);
const _: () = assert!(std::mem::offset_of!(GuestContext, xmm) == CTX_XMM0);
const _: () = assert!(std::mem::offset_of!(GuestContext, ymm) == CTX_YMM0);
const _: () = assert!(std::mem::size_of::<GuestContext>() == 944);

impl GuestContext {
    /// Every field is initialized under this documented original-fixture policy;
    /// no host register contents are ever observable through an uninitialized
    /// guest slot. GPRs are zero except the explicitly provided arguments and
    /// stack pointer; RFLAGS is the architectural reset value with IF set;
    /// MXCSR is the default masked state; XMM is zero.
    const fn zeroed() -> Self {
        Self {
            gpr: [0; 16],
            rip: 0,
            rflags: 0x202,
            guest_base: 0,
            guest_limit: 0,
            host_rsp: 0,
            mxcsr: 0x1f80,
            _pad: 0,
            xmm: [0; 16 * 16],
            ymm: [0; 32 * 16],
        }
    }
}

/// A process-wide, Sync wrapper around the single bridge context. The bridge
/// references this static's symbol via RIP-relative operands, so it must have a
/// stable address. Concurrency is forbidden by [`DISPATCH_ACTIVE`]; this owner
/// is never shared across a live borrow.
#[repr(transparent)]
struct ContextCell(UnsafeCell<GuestContext>);

// SAFETY: Access is serialized by DISPATCH_ACTIVE (one dispatch at a time,
// process-wide) and only ever occurs between acquiring and releasing that
// guard, on the same thread that drives pentax_block_enter. The bridge only
// touches it while that same thread is parked in the asm call.
unsafe impl Sync for ContextCell {}

static CONTEXT: ContextCell = ContextCell(UnsafeCell::new(GuestContext::zeroed()));

/// Only one native dispatch may be in flight process-wide, because the bridge
/// uses the single [`CONTEXT`] static. This is a documented engine limitation,
/// not a general reentrant runtime.
static DISPATCH_ACTIVE: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// The assembly bridge
// ---------------------------------------------------------------------------
//
// pentax_block_enter: save host state, install guest TEB bounds, load the full
// guest context, and jump to the cache body held in CONTEXT.rip. The jump is a
// RIP-relative memory-indirect jump, so it needs no scratch GPR and sacrifices
// no guest register: all sixteen GPRs hold their guest values at entry.
//
// pentax_block_trailer: the published cache block ends with an engine tail that
// transfers here. It saves every guest GPR first (plain stores never touch
// EFLAGS), captures RFLAGS only after switching to the host stack (so the guest
// red zone is never written), saves MXCSR/XMM, restores host nonvolatile and
// control state and the host TEB bounds, and returns to Rust. Guest flags are
// intact from block end through the point of capture because only EFLAGS-neutral
// instructions execute in between.
std::arch::global_asm!(
    r#"
    .text
    .globl pentax_block_enter
    .p2align 4
pentax_block_enter:
    push rbx
    push rbp
    push rsi
    push rdi
    push r12
    push r13
    push r14
    push r15
    sub rsp, 192
    stmxcsr [rsp]
    fnstcw [rsp + 4]
    movdqu [rsp + 16], xmm6
    movdqu [rsp + 32], xmm7
    movdqu [rsp + 48], xmm8
    movdqu [rsp + 64], xmm9
    movdqu [rsp + 80], xmm10
    movdqu [rsp + 96], xmm11
    movdqu [rsp + 112], xmm12
    movdqu [rsp + 128], xmm13
    movdqu [rsp + 144], xmm14
    movdqu [rsp + 160], xmm15
    # Probe the lowest host scratch slot under the host TEB before saving it.
    push 0
    pop rax
    mov rax, gs:[8]
    mov [rsp + 176], rax
    mov rax, gs:[0x10]
    mov [rsp + 184], rax
    mov [rip + {ctx} + 160], rsp
    mov rax, [rip + {ctx} + 144]
    mov gs:[8], rax
    mov rax, [rip + {ctx} + 152]
    mov gs:[0x10], rax
    ldmxcsr [rip + {ctx} + 168]
    movdqu xmm0, [rip + {ctx} + 176]
    movdqu xmm1, [rip + {ctx} + 192]
    movdqu xmm2, [rip + {ctx} + 208]
    movdqu xmm3, [rip + {ctx} + 224]
    movdqu xmm4, [rip + {ctx} + 240]
    movdqu xmm5, [rip + {ctx} + 256]
    movdqu xmm6, [rip + {ctx} + 272]
    movdqu xmm7, [rip + {ctx} + 288]
    movdqu xmm8, [rip + {ctx} + 304]
    movdqu xmm9, [rip + {ctx} + 320]
    movdqu xmm10, [rip + {ctx} + 336]
    movdqu xmm11, [rip + {ctx} + 352]
    movdqu xmm12, [rip + {ctx} + 368]
    movdqu xmm13, [rip + {ctx} + 384]
    movdqu xmm14, [rip + {ctx} + 400]
    movdqu xmm15, [rip + {ctx} + 416]
    push qword ptr [rip + {ctx} + 136]
    popfq
    mov rax, [rip + {ctx}]
    mov rcx, [rip + {ctx} + 8]
    mov rdx, [rip + {ctx} + 16]
    mov rbx, [rip + {ctx} + 24]
    mov rbp, [rip + {ctx} + 40]
    mov rsi, [rip + {ctx} + 48]
    mov rdi, [rip + {ctx} + 56]
    mov r8, [rip + {ctx} + 64]
    mov r9, [rip + {ctx} + 72]
    mov r10, [rip + {ctx} + 80]
    mov r11, [rip + {ctx} + 88]
    mov r12, [rip + {ctx} + 96]
    mov r13, [rip + {ctx} + 104]
    mov r14, [rip + {ctx} + 112]
    mov r15, [rip + {ctx} + 120]
    mov rsp, [rip + {ctx} + 32]
    jmp qword ptr [rip + {ctx} + 128]

    .globl pentax_block_trailer
    .p2align 4
pentax_block_trailer:
    mov [rip + {ctx}], rax
    mov [rip + {ctx} + 8], rcx
    mov [rip + {ctx} + 16], rdx
    mov [rip + {ctx} + 24], rbx
    mov [rip + {ctx} + 40], rbp
    mov [rip + {ctx} + 48], rsi
    mov [rip + {ctx} + 56], rdi
    mov [rip + {ctx} + 64], r8
    mov [rip + {ctx} + 72], r9
    mov [rip + {ctx} + 80], r10
    mov [rip + {ctx} + 88], r11
    mov [rip + {ctx} + 96], r12
    mov [rip + {ctx} + 104], r13
    mov [rip + {ctx} + 112], r14
    mov [rip + {ctx} + 120], r15
    mov [rip + {ctx} + 32], rsp
    mov rsp, [rip + {ctx} + 160]
    pushfq
    pop rax
    mov [rip + {ctx} + 136], rax
    # Preserve guest DF in CONTEXT; Windows/Rust requires host DF=0.
    cld
    stmxcsr [rip + {ctx} + 168]
    movdqu [rip + {ctx} + 176], xmm0
    movdqu [rip + {ctx} + 192], xmm1
    movdqu [rip + {ctx} + 208], xmm2
    movdqu [rip + {ctx} + 224], xmm3
    movdqu [rip + {ctx} + 240], xmm4
    movdqu [rip + {ctx} + 256], xmm5
    movdqu [rip + {ctx} + 272], xmm6
    movdqu [rip + {ctx} + 288], xmm7
    movdqu [rip + {ctx} + 304], xmm8
    movdqu [rip + {ctx} + 320], xmm9
    movdqu [rip + {ctx} + 336], xmm10
    movdqu [rip + {ctx} + 352], xmm11
    movdqu [rip + {ctx} + 368], xmm12
    movdqu [rip + {ctx} + 384], xmm13
    movdqu [rip + {ctx} + 400], xmm14
    movdqu [rip + {ctx} + 416], xmm15
    ldmxcsr [rsp]
    fldcw [rsp + 4]
    movdqu xmm6, [rsp + 16]
    movdqu xmm7, [rsp + 32]
    movdqu xmm8, [rsp + 48]
    movdqu xmm9, [rsp + 64]
    movdqu xmm10, [rsp + 80]
    movdqu xmm11, [rsp + 96]
    movdqu xmm12, [rsp + 112]
    movdqu xmm13, [rsp + 128]
    movdqu xmm14, [rsp + 144]
    movdqu xmm15, [rsp + 160]
    mov rax, [rsp + 176]
    mov gs:[8], rax
    mov rax, [rsp + 184]
    mov gs:[0x10], rax
    add rsp, 192
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
    ctx = sym CONTEXT,
);

unsafe extern "C" {
    fn pentax_block_enter();
    fn pentax_block_trailer();
}

// ---------------------------------------------------------------------------
// Owned, re-armable code cache (writable -> RX, never RWX)
// ---------------------------------------------------------------------------

/// A privately owned executable cache region. It is reserved and committed
/// PAGE_READWRITE for staging, and sealed PAGE_EXECUTE_READ before
/// each execution. It is never simultaneously writable and executable. The
/// A publication re-arms only its owned page range (RX back to RW), then flushes
/// that range after sealing. Each page tracks writable, unflushed RX and executable
/// states; writes and entry admission consult these states independently.
struct CodeCache {
    base: NonNull<c_void>,
    len: usize,
    /// Usable length exposed to the planner; the tail cell lives above it.
    capacity: usize,
    executable: bool,
    pages: Vec<CachePage>,
    _not_send: PhantomData<Rc<()>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CachePage {
    Writable,
    NeedsFlush,
    Executable,
}

/// Eight bytes reserved at the top of the region hold the trailer address that
/// each block's indirect tail jump reads. The planner only ever sees
/// `capacity`, which excludes this cell.
const TAIL_CELL: usize = 8;
// Shared finite neighborhood for original diagnostics and prepared stepping.
// Concurrent owned arenas can occupy the former 16 MiB search window.
const DEFAULT_CACHE_SEARCH_GRANULES: u16 = 4096;

impl CodeCache {
    /// Reserve a nearby free granule without assuming low guest addresses are
    /// unused by Windows. Every failed exact reservation leaves host owners
    /// intact; the bounded search stays within RIP-relative reach of the arena.
    fn reserve_near(bias: u64, len: usize) -> Result<Self, PlatformError> {
        for index in 0..u64::from(DEFAULT_CACHE_SEARCH_GRANULES) {
            let preferred = bias
                .checked_add(ARENA_BYTES)
                .and_then(|start| start.checked_add(index * 0x1_0000))
                .filter(|start| {
                    start
                        .checked_add(len as u64)
                        .is_some_and(|end| end < (1_u64 << 47))
                })
                .ok_or(PlatformError::OutOfBounds)?;
            match Self::reserve(preferred, len) {
                Err(PlatformError::OperatingSystem { code: 487, .. }) => continue,
                result => return result,
            }
        }
        Err(PlatformError::NativeDispatchFailed {
            stage: "near-cache-reservation",
        })
    }

    /// Reserve and commit RW at a preferred base near the guest image so that
    /// the planner's RIP-relative placement constraint is satisfiable.
    fn reserve(preferred: u64, len: usize) -> Result<Self, PlatformError> {
        if len < NATIVE_GUEST_PAGE as usize || !len.is_multiple_of(NATIVE_GUEST_PAGE as usize) {
            return Err(PlatformError::InvalidReservationSize);
        }
        let page_count = len / NATIVE_GUEST_PAGE as usize;
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(page_count)
            .map_err(|_| PlatformError::NativeDispatchFailed {
                stage: "cache-page-state-allocation",
            })?;
        if pages.capacity() != page_count {
            return Err(PlatformError::NativeDispatchFailed {
                stage: "cache-page-state-capacity",
            });
        }
        pages.resize(page_count, CachePage::Writable);
        // SAFETY: A preferred, page-granular base is requested; VirtualAlloc
        // either returns exactly that base or fails. No pointer is dereferenced.
        let raw = unsafe {
            VirtualAlloc(
                preferred as usize as *const c_void,
                len,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_READWRITE,
            )
        };
        let base = NonNull::new(raw).ok_or_else(|| last_error("VirtualAlloc(cache)"))?;
        if base.as_ptr() as u64 != preferred {
            // SAFETY: base is the just-returned owned reservation; release it.
            unsafe { VirtualFree(base.as_ptr(), 0, MEM_RELEASE) };
            return Err(PlatformError::OutOfBounds);
        }
        Ok(Self {
            base,
            len,
            capacity: len - TAIL_CELL,
            executable: false,
            pages,
            _not_send: PhantomData,
        })
    }

    fn address(&self) -> u64 {
        self.base.as_ptr() as u64
    }

    fn capacity(&self) -> u64 {
        self.capacity as u64
    }

    /// Re-arm to writable so a fresh block can be staged. A no-op the first time.
    fn arm_writable(&mut self) -> Result<(), PlatformError> {
        self.arm_writable_range(0, self.len)
    }

    fn page_range(
        &self,
        offset: usize,
        len: usize,
    ) -> Result<std::ops::Range<usize>, PlatformError> {
        let page = NATIVE_GUEST_PAGE as usize;
        if len == 0
            || !offset.is_multiple_of(page)
            || !len.is_multiple_of(page)
            || offset.checked_add(len).is_none_or(|end| end > self.len)
        {
            return Err(PlatformError::OutOfBounds);
        }
        Ok(offset / page..(offset + len) / page)
    }

    fn range_executable(&self, offset: usize, len: usize) -> Result<bool, PlatformError> {
        let pages = self.page_range(offset, len)?;
        Ok(self.pages[pages]
            .iter()
            .all(|page| *page == CachePage::Executable))
    }

    fn arm_writable_range(&mut self, offset: usize, len: usize) -> Result<(), PlatformError> {
        let pages = self.page_range(offset, len)?;
        if self.pages[pages.clone()]
            .iter()
            .all(|page| *page == CachePage::Writable)
        {
            return Ok(());
        }
        let mut old = 0;
        // SAFETY: The aligned, nonempty range is wholly inside this committed
        // owned reservation. Exclusive access excludes execution while its pages
        // revoke execute and become writable; no page is granted RWX.
        if unsafe {
            VirtualProtect(
                self.base.as_ptr().cast::<u8>().add(offset).cast(),
                len,
                PAGE_READWRITE,
                &mut old,
            )
        } == 0
        {
            return Err(last_error("VirtualProtect(cache RW)"));
        }
        self.pages[pages].fill(CachePage::Writable);
        self.executable = false;
        Ok(())
    }

    /// Copy checked bytes into the writable region at `offset`.
    fn write(&mut self, offset: usize, bytes: &[u8]) -> Result<(), PlatformError> {
        if offset
            .checked_add(bytes.len())
            .is_none_or(|end| end > self.len)
        {
            return Err(PlatformError::OutOfBounds);
        }
        if !bytes.is_empty() {
            let page = NATIVE_GUEST_PAGE as usize;
            if self.pages[offset / page..=(offset + bytes.len() - 1) / page]
                .iter()
                .any(|state| *state != CachePage::Writable)
            {
                return Err(PlatformError::OutOfBounds);
            }
        }
        if bytes.is_empty() {
            return Ok(());
        }
        // SAFETY: The range is checked inside the uniquely owned, committed,
        // currently writable region; no external alias into it exists.
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.base.as_ptr().cast::<u8>().add(offset),
                bytes.len(),
            );
        }
        Ok(())
    }

    /// Read bytes back from the region for attestation.
    fn read(&self, offset: usize, out: &mut [u8]) -> Result<(), PlatformError> {
        if offset
            .checked_add(out.len())
            .is_none_or(|end| end > self.len)
        {
            return Err(PlatformError::OutOfBounds);
        }
        if out.is_empty() {
            return Ok(());
        }
        // SAFETY: Checked range inside the uniquely owned committed region.
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.base.as_ptr().cast::<u8>().add(offset),
                out.as_mut_ptr(),
                out.len(),
            );
        }
        Ok(())
    }

    /// Compare bounded live cache bytes without allocating a second code copy.
    fn matches(&self, offset: usize, expected: &[u8]) -> Result<bool, PlatformError> {
        let mut scratch = [0; 128];
        for (index, chunk) in expected.chunks(scratch.len()).enumerate() {
            let at = index
                .checked_mul(scratch.len())
                .and_then(|n| offset.checked_add(n))
                .ok_or(PlatformError::OutOfBounds)?;
            self.read(at, &mut scratch[..chunk.len()])?;
            if scratch[..chunk.len()] != *chunk {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Revoke write, grant execute, and flush the instruction cache.
    fn seal_executable(&mut self) -> Result<(), PlatformError> {
        self.seal_executable_range(0, self.len)
    }

    fn seal_executable_range(&mut self, offset: usize, len: usize) -> Result<(), PlatformError> {
        let pages = self.page_range(offset, len)?;
        let mut old = 0;
        // SAFETY: The aligned range belongs wholly to the committed reservation.
        // Revoke write before granting execute. The caller holds exclusive cache
        // ownership and cannot enter the changed pages before successful flush.
        if unsafe {
            VirtualProtect(
                self.base.as_ptr().cast::<u8>().add(offset).cast(),
                len,
                PAGE_EXECUTE_READ,
                &mut old,
            )
        } == 0
        {
            return Err(last_error("VirtualProtect(cache RX)"));
        }
        // The OS protection already changed, even if the following flush fails.
        self.pages[pages.clone()].fill(CachePage::NeedsFlush);
        self.executable = false;
        // SAFETY: The pseudo handle names this process; the owned region stays
        // allocated and readable during publication.
        if unsafe {
            FlushInstructionCache(
                GetCurrentProcess(),
                self.base.as_ptr().cast::<u8>().add(offset).cast(),
                len,
            )
        } == 0
        {
            return Err(last_error("FlushInstructionCache(cache)"));
        }
        self.pages[pages].fill(CachePage::Executable);
        self.executable = self.pages.iter().all(|page| *page == CachePage::Executable);
        Ok(())
    }
}

impl Drop for CodeCache {
    fn drop(&mut self) {
        // SAFETY: base is the owned reservation; MEM_RELEASE needs size 0.
        unsafe { VirtualFree(self.base.as_ptr(), 0, MEM_RELEASE) };
    }
}

// ---------------------------------------------------------------------------
// Host feature verification
// ---------------------------------------------------------------------------

/// Whether the actual host supports a declared CPUID feature the planner
/// admitted. Unknown features are refused (return false) rather than assumed,
/// so an emission that relies on anything unmapped stops before execution.
fn host_supports(feature: CpuidFeature) -> bool {
    use CpuidFeature::*;
    match feature {
        // x86-64 long-mode baseline, always present on this target.
        INTEL8086 | INTEL186 | INTEL286 | INTEL386 | INTEL486 | X64 | CMOV | CX8 | MULTIBYTENOP => {
            true
        }
        SSE => std::is_x86_feature_detected!("sse"),
        SSE2 => std::is_x86_feature_detected!("sse2"),
        SSE3 => std::is_x86_feature_detected!("sse3"),
        SSSE3 => std::is_x86_feature_detected!("ssse3"),
        SSE4_1 => std::is_x86_feature_detected!("sse4.1"),
        SSE4_2 => std::is_x86_feature_detected!("sse4.2"),
        POPCNT => std::is_x86_feature_detected!("popcnt"),
        LZCNT => std::is_x86_feature_detected!("lzcnt"),
        BMI1 => std::is_x86_feature_detected!("bmi1"),
        BMI2 => std::is_x86_feature_detected!("bmi2"),
        ADX => std::is_x86_feature_detected!("adx"),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// The closed original program
// ---------------------------------------------------------------------------
//
// Fixed layout of the one executable program this engine accepts. All addresses
// are derived from these constants; no caller value reaches them.
const ARENA_BYTES: u64 = 0x20_0000;
#[cfg(test)]
const BIAS: u64 = 0x20_0000;
const ENTRY: usize = 0x200;
const DATA_OFF: u64 = 0x4000;
const PAGE: u64 = 0x4000;
// Two mapped pages sit between two owned, uncommitted guard pages in the
// same 64 KiB allocation. Neither guard depends on an unowned host hole.
const STACK_OFF: u64 = 0x10_4000;
const STACK_BYTES: u64 = 2 * PAGE;
const CACHE_BYTES: usize = 0x10000;
/// The value seeded as the outermost return address. Popping it is the only
/// normal completion; it is not an executable target.
const STOP: u64 = 0;
/// Data constant loaded via the RIP-relative operand; a fixed diagnostic value.
const DATA_CONSTANT: u64 = 100;
/// The verified final result.
const EXPECTED_RESULT: u64 = 42;
/// The red-zone sentinel the guest writes and the dispatcher re-checks.
const SENTINEL: u64 = 42;

/// Build the one original ELF image, the guest code and the data constant.
/// Returns the image bytes and the guest-code offsets used for assertions.
fn original_image() -> Vec<u8> {
    // Two-pass hand assembly of the closed program. Every control transfer is
    // mediated by the dispatcher, so the rel/RIP displacements only need to
    // point at the intended guest instruction boundaries.
    let base = ENTRY as u64;
    let data_va = DATA_OFF;

    // Offsets of each labelled instruction within the code (see module docs).
    let l_equal = 32_u64;
    let l_cont = 37_u64;
    let l_callee = 53_u64;
    let l_after = 47_u64;

    let mut code = Vec::new();
    // lea rax,[rdi+rsi]            ; rax = 19 + 23 = 42   (arithmetic)
    code.extend_from_slice(&[0x48, 0x8d, 0x04, 0x37]);
    // mov [rsp-16],rax             ; red-zone sentinel
    code.extend_from_slice(&[0x48, 0x89, 0x44, 0x24, 0xf0]);
    // mov rcx,[rip+disp]           ; RIP-relative data operand (load DATA_CONSTANT)
    let mov_next = base + 16;
    let disp = (data_va as i64 - mov_next as i64) as i32;
    code.extend_from_slice(&[0x48, 0x8b, 0x0d]);
    code.extend_from_slice(&disp.to_le_bytes());
    // cmp rax,42
    code.extend_from_slice(&[0x48, 0x83, 0xf8, 0x2a]);
    // je L_equal                   ; conditional transfer (mediated)
    let je_next = base + 26;
    let je_disp = (base + l_equal) as i64 - je_next as i64;
    code.extend_from_slice(&[0x0f, 0x84]);
    code.extend_from_slice(&(je_disp as i32).to_le_bytes());
    // L_fail @26: mov eax,0; ret   ; not reached when rax == 42
    code.extend_from_slice(&[0xb8, 0x00, 0x00, 0x00, 0x00]);
    code.push(0xc3);
    // L_equal @32: jmp L_cont      ; direct transfer (mediated)
    let jmp_next = base + 37;
    let jmp_disp = (base + l_cont) as i64 - jmp_next as i64;
    code.push(0xe9);
    code.extend_from_slice(&(jmp_disp as i32).to_le_bytes());
    // L_cont @37: lea r11,[rip+disp]  ; ordinary; computes callee address
    let lea_next = base + 44;
    let lea_disp = (base + l_callee) as i64 - lea_next as i64;
    code.extend_from_slice(&[0x4c, 0x8d, 0x1d]);
    code.extend_from_slice(&(lea_disp as i32).to_le_bytes());
    // call r11                     ; indirect call (mediated), checked stack effect
    code.extend_from_slice(&[0x41, 0xff, 0xd3]);
    // L_after @47: mov rax,[rsp-16]; ret  ; read sentinel back; return (mediated)
    code.extend_from_slice(&[0x48, 0x8b, 0x44, 0x24, 0xf0]);
    code.push(0xc3);
    // L_callee @53: xor edx,edx; ret
    code.extend_from_slice(&[0x31, 0xd2]);
    code.push(0xc3);

    debug_assert_eq!(code.len(), 56);
    debug_assert_eq!(l_after, 47);

    build_elf(&code)
}

/// Minimal ELF64 wrapper with an R-X code segment and an RW data segment that
/// holds [`DATA_CONSTANT`] at offset 0. Mirrors the shape the planner's own
/// fixtures decode; no external binary is read.
fn build_elf(code: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0u8; 0x4100 + 8];
    let put = |b: &mut [u8], at: usize, v: &[u8]| b[at..at + v.len()].copy_from_slice(v);
    put(&mut bytes, 0, b"\x7fELF\x02\x01\x01");
    put(&mut bytes, 16, &3u16.to_le_bytes());
    put(&mut bytes, 18, &62u16.to_le_bytes());
    put(&mut bytes, 20, &1u32.to_le_bytes());
    put(&mut bytes, 24, &(ENTRY as u64).to_le_bytes());
    put(&mut bytes, 32, &64u64.to_le_bytes());
    put(&mut bytes, 52, &64u16.to_le_bytes());
    put(&mut bytes, 54, &56u16.to_le_bytes());
    put(&mut bytes, 56, &2u16.to_le_bytes());
    let program = |b: &mut [u8], index: usize, kind: u32, flags: u32, f: [u64; 5]| {
        let at = 64 + index * 56;
        let putp = |b: &mut [u8], at: usize, v: &[u8]| b[at..at + v.len()].copy_from_slice(v);
        putp(b, at, &kind.to_le_bytes());
        putp(b, at + 4, &flags.to_le_bytes());
        for (field, value) in [(8, f[0]), (16, f[1]), (32, f[2]), (40, f[3]), (48, f[4])] {
            putp(b, at + field, &value.to_le_bytes());
        }
    };
    // PT_LOAD R-X: file offset 0, vaddr 0, covering the ELF header and code.
    program(
        &mut bytes,
        0,
        1,
        5,
        [0, 0, (ENTRY + code.len()) as u64, PAGE, PAGE],
    );
    // PT_LOAD RW-: the data segment holding the constant.
    program(&mut bytes, 1, 1, 6, [DATA_OFF, DATA_OFF, 0x100, PAGE, PAGE]);
    put(&mut bytes, ENTRY, code);
    put(&mut bytes, DATA_OFF as usize, &DATA_CONSTANT.to_le_bytes());
    bytes
}

// ---------------------------------------------------------------------------
// Guest memory staging (owned, paused, physically NX)
// ---------------------------------------------------------------------------

/// Stage the plan's segments and an owned guest stack into a fresh native
/// address space at the plan's guest VAs. Code pages are logical-X but
/// physically read-only NX; the CPU fetches only from the cache. Data/stack
/// pages are read/write. A live red-zone sentinel lives in the stack region.
fn fresh_guest_space() -> Result<NativeAddressSpace, PlatformError> {
    NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), ARENA_BYTES).map_err(native)
}

fn stage_guest(
    plan: &SymbolicLinkPlan<'_>,
    mut space: NativeAddressSpace,
) -> Result<(NativeAddressSpace, u64), PlatformError> {
    let module = &plan.modules()[0];
    for segment in &module.runtime_plan().segments {
        if segment.mem_size == 0 {
            continue;
        }
        let va = segment
            .vaddr
            .checked_add(module.load_bias())
            .ok_or(PlatformError::OutOfBounds)?;
        let size = round_up(segment.mem_size, NATIVE_GUEST_PAGE)?;
        space
            .map_at(va, size, NativePermissions::READ_WRITE)
            .map_err(native)?;
        {
            let mut paused = space.pause().map_err(native)?;
            paused.write(va, segment.file_bytes).map_err(native)?;
        }
    }
    // Complete retained writes before final permissions, as in core staging.
    for patch in plan.patches() {
        let mut paused = space.pause().map_err(native)?;
        paused
            .write(patch.address(), &patch.value().to_le_bytes())
            .map_err(native)?;
    }
    for segment in &module.runtime_plan().segments {
        if segment.mem_size != 0 && segment.flags.executable() {
            space
                .protect(
                    module.load_bias() + segment.vaddr,
                    round_up(segment.mem_size, NATIVE_GUEST_PAGE)?,
                    NativePermissions::READ_EXECUTE,
                )
                .map_err(native)?;
        }
    }
    // Owned guest stack, disjoint from the image segments.
    let stack_va = module.load_bias() + STACK_OFF;
    space
        .map_at(stack_va, STACK_BYTES, NativePermissions::READ_WRITE)
        .map_err(native)?;
    let top = stack_va + STACK_BYTES;
    // rsp points at a seeded STOP return address; SysV entry alignment (rsp % 16
    // == 8) is preserved. The red zone [rsp-128, rsp) stays owned and untouched
    // by the engine.
    let rsp = (top - 16) + 8;
    {
        let mut paused = space.pause().map_err(native)?;
        paused.write(rsp, &STOP.to_le_bytes()).map_err(native)?;
    }
    Ok((space, rsp))
}

fn round_up(value: u64, unit: u64) -> Result<u64, PlatformError> {
    value
        .checked_add(unit - 1)
        .map(|v| v / unit * unit)
        .ok_or(PlatformError::OutOfBounds)
}

fn native(_error: NativeMemoryError) -> PlatformError {
    PlatformError::NativeDispatchFailed {
        stage: "guest-memory",
    }
}

// ---------------------------------------------------------------------------
// Block publication and attestation
// ---------------------------------------------------------------------------

/// Publish one block's ordinary instructions plus the engine exit tail into the
/// cache, after verifying the paused source bytes and the host feature policy.
///
/// The ordinary prefix is attested by direct equality with the planner's
/// retained emission, and the planner's own `attest_exit` confirms the terminal
/// UD2 site. The engine's controlled native tail that replaces that UD2 has its
/// own complete attestation here: the UD2 attestation is NOT reused for it.
fn publish_block(
    cache: &mut CodeCache,
    space: &mut NativeAddressSpace,
    plan: &BlockPlan<'_, '_>,
    guest_pc: u64,
) -> Result<(), PlatformError> {
    publish_block_with_mode(cache, space, plan, guest_pc, ymm_bridge::VectorMode::Sse2)
}

fn publish_block_with_mode(
    cache: &mut CodeCache,
    space: &mut NativeAddressSpace,
    plan: &BlockPlan<'_, '_>,
    guest_pc: u64,
    mode: ymm_bridge::VectorMode,
) -> Result<(), PlatformError> {
    // The closed diagnostic wrapper above always selects the SSE2 mode.
    // A declared vector request cannot override that immutable entry/tail pair.
    if ((plan.policy().requests_native_packed_float_memory()
        || plan.policy().requests_native_streaming_stores()
        || plan.policy().requests_native_vector_moves())
        && !plan.policy().requests_native_ymm_registers())
        || ((plan.policy().requests_native_ymm_registers()
            || plan.policy().requests_native_packed_float_memory()
            || plan.policy().requests_native_streaming_stores()
            || plan.policy().requests_native_vector_moves())
            && mode.features().is_none())
    {
        return Err(PlatformError::UnsupportedHostFeature {
            feature: "native YMM context",
        });
    }
    // 1. Host CPUID/OS support must satisfy the declared emission policy for
    //    every instruction actually emitted into the cache.
    for record in plan.instructions() {
        if record.emitted_len() == 0 {
            continue;
        }
        if matches!(
            record.instruction().code(),
            pentax_codegen::Code::VEX_Vmovntdq_m128_xmm
                | pentax_codegen::Code::VEX_Vmovntdq_m256_ymm
        ) && !plan.policy().requests_native_streaming_stores()
        {
            return Err(PlatformError::UnsupportedHostFeature {
                feature: "native streaming store profile",
            });
        }
        for feature in record.instruction().cpuid_features() {
            if !mode.supports(*feature) {
                return Err(PlatformError::UnsupportedHostFeature {
                    feature: feature_name(*feature),
                });
            }
        }
    }

    // 2. Verify the actual paused source bytes against the retained relocated
    //    window before publication. Unknown/altered source must not be run.
    let end = match plan.exit().instruction() {
        Some(_) => plan.exit().next_guest_pc(),
        None => plan.exit().guest_pc(),
    };
    let consumed = usize::try_from(
        end.checked_sub(guest_pc)
            .ok_or(PlatformError::OutOfBounds)?,
    )
    .map_err(|_| PlatformError::OutOfBounds)?;
    let relocated = plan.relocated_source_bytes();
    if consumed > relocated.len() {
        return Err(PlatformError::SourceAttestationFailed);
    }
    let mut source = vec![0u8; consumed];
    {
        let paused = space.pause().map_err(native)?;
        paused.fetch_source(guest_pc, &mut source).map_err(native)?;
    }
    if source != relocated[..consumed] {
        return Err(PlatformError::SourceAttestationFailed);
    }

    // 3. The planner must have placed the block exactly at the cache base, so
    //    the verbatim ordinary copy keeps every re-encoded RIP-relative operand.
    let bytes = plan.bytes();
    if bytes.len() < 2 || !plan.attest_emission(cache.address(), bytes) {
        return Err(PlatformError::EmissionAttestationFailed);
    }
    let ud2_off = bytes.len() - 2;
    if !plan.attest_exit(cache.address() + ud2_off as u64, &bytes[ud2_off..]) {
        return Err(PlatformError::EmissionAttestationFailed);
    }
    let ordinary = &bytes[..ud2_off];

    // 4. Build the controlled native tail: an indirect jump through an in-cache
    //    cell holding the trailer address. This is the ONLY byte the engine
    //    emits beyond the planner's ordinary output; it has its own attestation.
    let cell_off = cache.capacity; // == len - TAIL_CELL
    if ud2_off + 6 > cell_off {
        return Err(PlatformError::OutOfBounds);
    }
    let trailer = mode.trailer();
    let jmp_end = cache.address() + (ud2_off + 6) as u64;
    let cell_abs = cache.address() + cell_off as u64;
    let rel =
        i32::try_from(cell_abs as i64 - jmp_end as i64).map_err(|_| PlatformError::OutOfBounds)?;
    let mut tail = [0u8; 6];
    tail[0] = 0xff; // jmp qword ptr [rip + rel32]
    tail[1] = 0x25;
    tail[2..].copy_from_slice(&rel.to_le_bytes());

    // 5. Stage into the re-armed writable cache, then attest the exact bytes.
    cache.arm_writable()?;
    cache.write(0, ordinary)?;
    cache.write(ud2_off, &tail)?;
    cache.write(cell_off, &trailer.to_le_bytes())?;

    let mut check = vec![0u8; ud2_off + 6];
    cache.read(0, &mut check)?;
    if check[..ud2_off] != *ordinary || check[ud2_off..] != tail {
        return Err(PlatformError::EmissionAttestationFailed);
    }
    let mut cell = [0u8; 8];
    cache.read(cell_off, &mut cell)?;
    if u64::from_le_bytes(cell) != trailer {
        return Err(PlatformError::EmissionAttestationFailed);
    }

    cache.seal_executable()
}

fn feature_name(feature: CpuidFeature) -> &'static str {
    use CpuidFeature::*;
    match feature {
        SSE => "sse",
        SSE2 => "sse2",
        SSE3 => "sse3",
        SSSE3 => "ssse3",
        SSE4_1 => "sse4.1",
        SSE4_2 => "sse4.2",
        POPCNT => "popcnt",
        LZCNT => "lzcnt",
        BMI1 => "bmi1",
        BMI2 => "bmi2",
        ADX => "adx",
        _ => "unsupported-feature",
    }
}

// ---------------------------------------------------------------------------
// Mediated transfer resolution
// ---------------------------------------------------------------------------

/// The precise HLE stop metadata captured when the guest reaches a SYSCALL.
/// This records state but never fakes a kernel service or a success value.
/// Fields beyond rip/rax are retained for a future reviewed HLE integration.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
struct SyscallStop {
    rip: u64,
    rax: u64,
    rcx: u64,
    rdi: u64,
    rsi: u64,
    rdx: u64,
    r10: u64,
    r8: u64,
    r9: u64,
}

/// What the dispatcher does after a block exit.
enum Next {
    /// Continue at this guest PC.
    Resume(u64),
    /// The outermost return address was popped: dispatch is complete.
    Done,
}

/// Map a decoder GPR64 register onto a context slot index.
fn gpr_slot(reg: Register) -> Option<usize> {
    use Register::*;
    Some(match reg {
        RAX => 0,
        RCX => 1,
        RDX => 2,
        RBX => 3,
        RSP => 4,
        RBP => 5,
        RSI => 6,
        RDI => 7,
        R8 => 8,
        R9 => 9,
        R10 => 10,
        R11 => 11,
        R12 => 12,
        R13 => 13,
        R14 => 14,
        R15 => 15,
        _ => return Option::None,
    })
}

/// Evaluate a condition code against the captured guest RFLAGS.
fn condition_holds(condition: ConditionCode, rflags: u64) -> Option<bool> {
    let cf = rflags & (1 << 0) != 0;
    let pf = rflags & (1 << 2) != 0;
    let zf = rflags & (1 << 6) != 0;
    let sf = rflags & (1 << 7) != 0;
    let of = rflags & (1 << 11) != 0;
    use ConditionCode::*;
    Some(match condition {
        o => of,
        no => !of,
        b => cf,
        ae => !cf,
        e => zf,
        ne => !zf,
        be => cf || zf,
        a => !cf && !zf,
        s => sf,
        ns => !sf,
        p => pf,
        np => !pf,
        l => sf != of,
        ge => sf == of,
        le => zf || (sf != of),
        g => !zf && (sf == of),
        None => return Option::None,
    })
}

/// Confirm a resolved transfer target lies inside an executable segment of the
/// primary module. A non-executable, unmapped or non-canonical target is
/// rejected: unknown data never selects an executable flow.
fn executable_target(plan: &SymbolicLinkPlan<'_>, target: u64) -> bool {
    if target >= (1 << 47) {
        return false;
    }
    let module = &plan.modules()[0];
    for segment in &module.runtime_plan().segments {
        if !segment.flags.executable() || segment.file_bytes.is_empty() {
            continue;
        }
        let Some(start) = segment.vaddr.checked_add(module.load_bias()) else {
            continue;
        };
        let end = start + segment.file_bytes.len() as u64;
        if target >= start && target < end {
            return true;
        }
    }
    false
}

/// Compute the effective address of a mediated indirect memory operand from the
/// captured guest registers, honouring an original RIP-relative target.
fn memory_effective_address(
    ctx: &GuestContext,
    operand: &pentax_codegen::MemoryOperand,
) -> Option<u64> {
    if let Some(ip_target) = operand.ip_relative_address() {
        return Some(ip_target);
    }
    let base = match operand.base() {
        Register::None => 0,
        reg => ctx.gpr[gpr_slot(reg)?],
    };
    let index = match operand.index() {
        Register::None => 0,
        reg => ctx.gpr[gpr_slot(reg)?].wrapping_mul(u64::from(operand.scale())),
    };
    Some(
        base.wrapping_add(index)
            .wrapping_add(operand.displacement()),
    )
}

/// Resolve a direct/register/memory control target to an absolute guest PC.
fn resolve_target(
    ctx: &GuestContext,
    space: &mut NativeAddressSpace,
    target: ControlTarget,
) -> Result<u64, PlatformError> {
    match target {
        ControlTarget::Direct(address) => Ok(address),
        ControlTarget::Register(reg) => {
            let slot = gpr_slot(reg).ok_or(PlatformError::NativeDispatchFailed {
                stage: "indirect-register",
            })?;
            Ok(ctx.gpr[slot])
        }
        ControlTarget::Memory(operand) => {
            let address = memory_effective_address(ctx, &operand).ok_or(
                PlatformError::NativeDispatchFailed {
                    stage: "indirect-memory",
                },
            )?;
            let mut pointer = [0u8; 8];
            let paused = space.pause().map_err(native)?;
            paused.read(address, &mut pointer).map_err(native)?;
            Ok(u64::from_le_bytes(pointer))
        }
    }
}

// ---------------------------------------------------------------------------
// The dispatcher
// ---------------------------------------------------------------------------

/// A guard that forbids a concurrent native dispatch process-wide and restores
/// the context to zero on drop so no guest or host register contents linger.
struct DispatchGuard;

impl DispatchGuard {
    fn acquire() -> Result<Self, PlatformError> {
        if DISPATCH_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(PlatformError::TrapScopeUnavailable);
        }
        Ok(Self)
    }
}

impl Drop for DispatchGuard {
    fn drop(&mut self) {
        // SAFETY: We hold the exclusive dispatch guard; no bridge call is in
        // flight here. Zeroing prevents any later leak of guest/host contents.
        unsafe { *CONTEXT.0.get() = GuestContext::zeroed() };
        DISPATCH_ACTIVE.store(false, Ordering::Release);
    }
}

/// Order actual emitted streaming writes before any subsequent Rust load.
fn complete_non_temporal_stores() {
    // SAFETY: MFENCE is part of the mandatory x86-64 SSE2 baseline. Its default
    // inline-assembly memory effects also prevent compiler reordering across
    // this boundary. Do not add nomem/readonly: subsequent Rust loads must wait
    // for the actual guest stores, including legacy non-temporal instructions.
    unsafe { std::arch::asm!("mfence", options(nostack, preserves_flags)) };
}

fn preflight_diagnostic_vector_memory(
    space: &mut NativeAddressSpace,
    plan: &BlockPlan<'_, '_>,
) -> Result<(), PlatformError> {
    if !plan.emits_non_temporal_store() && !plan.emits_vector_memory_move() {
        return Ok(());
    }
    // SAFETY: run_one_block's caller holds DispatchGuard. No bridge call is in
    // flight; only current guest address registers are observed here.
    let gpr = unsafe { (*CONTEXT.0.get()).gpr };
    let memory = space.pause().map_err(native)?;
    let mut analyzer = MemoryAccessAnalyzer::default();
    let mut written = 0_u16;
    for record in plan.instructions().iter().filter(|r| r.emitted_len() != 0) {
        let accesses =
            analyzer
                .analyze(record, &gpr)
                .map_err(|_| PlatformError::NativeDispatchFailed {
                    stage: "native-vector footprint",
                })?;
        let dependencies = accesses.register_dependencies();
        if dependencies.address & written != 0 {
            return Err(PlatformError::NativeDispatchFailed {
                stage: "native-vector address dependency",
            });
        }
        written |= dependencies.written;
        for access in accesses.accesses() {
            if matches!(
                access.kind,
                MemoryAccessKind::Read | MemoryAccessKind::ReadWrite
            ) {
                memory
                    .preflight_read(access.address, access.size)
                    .map_err(native)?;
            }
            if matches!(
                access.kind,
                MemoryAccessKind::Write | MemoryAccessKind::ReadWrite
            ) {
                memory
                    .preflight_write(access.address, access.size)
                    .map_err(native)?;
            }
        }
    }
    Ok(())
}

/// Run one block: publish it, enter the bridge, and return the captured exit.
/// On return the shared [`CONTEXT`] holds the post-block guest state.
fn run_one_block(
    cache: &mut CodeCache,
    space: &mut NativeAddressSpace,
    plan: &BlockPlan<'_, '_>,
    guest_pc: u64,
) -> Result<(), PlatformError> {
    publish_block(cache, space, plan, guest_pc)?;
    preflight_diagnostic_vector_memory(space, plan)?;
    let streaming = plan.emits_non_temporal_store();
    // SAFETY: We hold the exclusive DispatchGuard, so no other access to
    // CONTEXT races this. The cache body at CONTEXT.rip was attested above:
    // its ordinary bytes equal the planner's emission and its only added tail
    // is the engine trailer jump. The guest stack and data pages are owned,
    // live and mapped. The bridge preserves/restores host nonvolatile, control
    // and TEB state. An unexpected fault inside the block stays fatal to this
    // dedicated worker; no resumption across incomplete recovery is attempted.
    unsafe {
        let ctx = &mut *CONTEXT.0.get();
        ctx.rip = cache.address();
        pentax_block_enter();
        if streaming {
            complete_non_temporal_stores();
        }
    }
    Ok(())
}

/// Drive the closed original program across its native blocks.
fn dispatch(plan: &SymbolicLinkPlan<'_>, arena: NativeAddressSpace) -> Result<u64, PlatformError> {
    let _guard = DispatchGuard::acquire()?;
    let bias = plan.modules()[0].load_bias();
    let (mut space, rsp) = stage_guest(plan, arena)?;
    let stack_va = bias + STACK_OFF;
    let mut cache = CodeCache::reserve_near(bias, CACHE_BYTES)?;

    // Documented original-fixture initialization policy for every field.
    // SAFETY: Exclusive dispatch guard held; no bridge call in flight.
    unsafe {
        let ctx = &mut *CONTEXT.0.get();
        *ctx = GuestContext::zeroed();
        ctx.gpr[RDI] = 19;
        ctx.gpr[RSI] = 23;
        ctx.gpr[RDX] = 91; // The native callee must actually clear this value.
        ctx.gpr[RSP] = rsp;
        ctx.guest_base = stack_va + STACK_BYTES;
        ctx.guest_limit = stack_va;
    }

    let mut guest_pc = plan.primary_entry();
    let policy = TranslationPolicy::x64_sse2();
    let limits = BlockLimits::default();
    // A bounded number of block dispatches for this closed program; a runaway
    // or cyclic plan stops explicitly rather than spinning.
    let mut exits = 0u32;
    let mut observed_exits = 0u32;
    loop {
        if exits >= 64 {
            return Err(PlatformError::NativeDispatchFailed {
                stage: "block-budget",
            });
        }
        exits += 1;

        let placement = CachePlacement {
            address: cache.address(),
            capacity: cache.capacity(),
        };
        let plan_block_result = plan_block(plan, 0, guest_pc, placement, &policy, &limits);
        let block = plan_block_result.map_err(|_| PlatformError::NativeDispatchFailed {
            stage: "plan-block",
        })?;

        run_one_block(&mut cache, &mut space, &block, guest_pc)?;

        // Read back the post-block guest state and verify the live red-zone
        // sentinel across every exit (it must survive the host round trip).
        // SAFETY: exclusive guard; the bridge returned, no call in flight.
        let ctx = unsafe { *CONTEXT.0.get() };
        observed_exits += 1;
        // This address belongs to the outer frame, independent of nested CALLs.
        verify_sentinel(&mut space, rsp)?;

        let next = resolve_exit(plan, &mut space, &block, &ctx, rsp)?;
        match next {
            Next::Resume(pc) => {
                // SAFETY: exclusive guard; update resume PC only.
                guest_pc = pc;
            }
            Next::Done => {
                if observed_exits != 5 || ctx.gpr[RCX] != DATA_CONSTANT || ctx.gpr[RDX] != 0 {
                    return Err(PlatformError::NativeDispatchFailed {
                        stage: "original-program-state",
                    });
                }
                return Ok(ctx.gpr[RAX]);
            }
        }
    }
}

/// Verify the guest red-zone sentinel written by the program. The sentinel sits
/// at [outer_rsp-16]; the engine never writes the guest red zone, so a corrupted
/// sentinel means a round-trip defect and is reported, not ignored.
fn verify_sentinel(space: &mut NativeAddressSpace, outer_rsp: u64) -> Result<(), PlatformError> {
    let address = outer_rsp
        .checked_sub(16)
        .ok_or(PlatformError::OutOfBounds)?;
    let mut bytes = [0u8; 8];
    let paused = space.pause().map_err(native)?;
    paused.read(address, &mut bytes).map_err(native)?;
    let value = u64::from_le_bytes(bytes);
    if value != SENTINEL {
        return Err(PlatformError::RedZoneCorrupted { value });
    }
    Ok(())
}

/// Resolve the block's typed exit against owned guest memory.
fn resolve_exit(
    plan: &SymbolicLinkPlan<'_>,
    space: &mut NativeAddressSpace,
    block: &BlockPlan<'_, '_>,
    ctx: &GuestContext,
    outer_rsp: u64,
) -> Result<Next, PlatformError> {
    match block.exit().kind() {
        ExitKind::Fallthrough { target } => {
            check_executable(plan, target)?;
            Ok(Next::Resume(target))
        }
        ExitKind::Conditional {
            condition,
            target,
            fallthrough,
        } => {
            let taken = condition_holds(condition, ctx.rflags)
                .ok_or(PlatformError::NativeDispatchFailed { stage: "condition" })?;
            let pc = if taken { target } else { fallthrough };
            check_executable(plan, pc)?;
            Ok(Next::Resume(pc))
        }
        ExitKind::Jump { target } => {
            let pc = resolve_target(ctx, space, target)?;
            check_executable(plan, pc)?;
            Ok(Next::Resume(pc))
        }
        ExitKind::Call { target, return_pc } => {
            let pc = resolve_target(ctx, space, target)?;
            check_executable(plan, pc)?;
            check_executable(plan, return_pc)?;
            // Checked CALL stack effect: push the return address onto the owned
            // guest stack (rsp -= 8). Context already holds the post-block rsp.
            let new_rsp = ctx.gpr[RSP]
                .checked_sub(8)
                .ok_or(PlatformError::OutOfBounds)?;
            check_stack_pointer(new_rsp, plan.modules()[0].load_bias())?;
            {
                let mut paused = space.pause().map_err(native)?;
                paused
                    .write(new_rsp, &return_pc.to_le_bytes())
                    .map_err(native)?;
            }
            commit_rsp(new_rsp);
            Ok(Next::Resume(pc))
        }
        ExitKind::Return { adjust } => {
            // Checked RET stack effect: pop the return address, then raise rsp
            // by 8 + adjust. The outermost STOP marker ends dispatch normally.
            let rsp = ctx.gpr[RSP];
            let mut bytes = [0u8; 8];
            {
                let paused = space.pause().map_err(native)?;
                paused.read(rsp, &mut bytes).map_err(native)?;
            }
            let target = u64::from_le_bytes(bytes);
            let new_rsp = rsp
                .checked_add(8)
                .and_then(|v| v.checked_add(u64::from(adjust)))
                .ok_or(PlatformError::OutOfBounds)?;
            if target == STOP {
                if rsp != outer_rsp || adjust != 0 {
                    return Err(PlatformError::GuestTargetRejected { target });
                }
                check_stack_pointer(new_rsp, plan.modules()[0].load_bias())?;
                commit_rsp(new_rsp);
                return Ok(Next::Done);
            }
            check_executable(plan, target)?;
            check_stack_pointer(new_rsp, plan.modules()[0].load_bias())?;
            commit_rsp(new_rsp);
            Ok(Next::Resume(target))
        }
        ExitKind::Syscall => {
            // Unknown syscall: explicit pre-execution refusal with precise HLE
            // stop metadata. No kernel service is faked and no success value is
            // substituted. The actual guest SYSCALL was never emitted.
            let stop = SyscallStop {
                rip: block.exit().guest_pc(),
                rax: ctx.gpr[RAX],
                rcx: ctx.gpr[RCX],
                rdi: ctx.gpr[RDI],
                rsi: ctx.gpr[RSI],
                rdx: ctx.gpr[RDX],
                r10: ctx.gpr[10],
                r8: ctx.gpr[8],
                r9: ctx.gpr[9],
            };
            let _ = &stop; // Retained for a future reviewed HLE integration.
            Err(PlatformError::UnknownGuestSyscall {
                rip: stop.rip,
                number: stop.rax,
            })
        }
        ExitKind::Interrupt { .. }
        | ExitKind::GuestException { .. }
        | ExitKind::Mediated { .. } => Err(PlatformError::UnsupportedGuestExit {
            guest_pc: block.exit().guest_pc(),
        }),
        // `ExitKind` is non_exhaustive: any future variant has no native path
        // until it is reviewed, so refuse rather than guess.
        _ => Err(PlatformError::UnsupportedGuestExit {
            guest_pc: block.exit().guest_pc(),
        }),
    }
}

fn check_stack_pointer(rsp: u64, bias: u64) -> Result<(), PlatformError> {
    let start = bias + STACK_OFF;
    if (start + 128..=start + STACK_BYTES).contains(&rsp) {
        Ok(())
    } else {
        Err(PlatformError::OutOfBounds)
    }
}

/// Commit a new guest RSP into the shared context between blocks.
fn commit_rsp(rsp: u64) {
    // SAFETY: exclusive dispatch guard held; no bridge call in flight.
    unsafe {
        (*CONTEXT.0.get()).gpr[RSP] = rsp;
    }
}

fn check_executable(plan: &SymbolicLinkPlan<'_>, target: u64) -> Result<(), PlatformError> {
    if executable_target(plan, target) {
        Ok(())
    } else {
        Err(PlatformError::GuestTargetRejected { target })
    }
}

// ---------------------------------------------------------------------------
// Public closed entry
// ---------------------------------------------------------------------------

/// Execute the one closed native-block-dispatch diagnostic in this worker.
pub(crate) fn execute_native_block_dispatch() -> Result<OriginalFixtureResult, PlatformError> {
    context_probe::verify_context_roundtrip()?;
    let arena = fresh_guest_space()?;
    let bias = arena
        .reserved_ranges()
        .next()
        .ok_or(PlatformError::OutOfBounds)?
        .address;
    let image = original_image();
    let view = open_executable(&image).map_err(|_| PlatformError::NativeDispatchFailed {
        stage: "open-executable",
    })?;
    let inputs = [ModuleInput::new(view, bias)];
    let plan = plan_symbolic(&inputs, &SymbolicLimits::default()).map_err(|_| {
        PlatformError::NativeDispatchFailed {
            stage: "plan-symbolic",
        }
    })?;
    let value = dispatch(&plan, arena)?;
    if value != EXPECTED_RESULT {
        return Err(PlatformError::UnexpectedDispatchResult { value });
    }
    Ok(OriginalFixtureResult { value, traps: 0 })
}

#[cfg(test)]
#[path = "dispatch/tests.rs"]
mod tests;
