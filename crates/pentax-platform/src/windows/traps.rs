// SPDX-License-Identifier: GPL-2.0-or-later

//! One synthetic service at one sealed, attested original-fixture site.
//!
//! VEH is not a syscall filter. The only executable form contains UD2 instead
//! of SYSCALL. The callback never runs general HLE or trusts a caller's code.
//! Windows callback restrictions:
//! <https://learn.microsoft.com/en-us/windows/win32/api/winnt/nc-winnt-pvectored_exception_handler>
//! <https://learn.microsoft.com/en-us/windows/win32/api/errhandlingapi/nf-errhandlingapi-removevectoredexceptionhandler>

use crate::{
    GUEST_PAGE_SIZE, GuestReservation, OriginalFixtureResult, PlatformError, SYNTHETIC_SUM_SERVICE,
    TRAPPED_SUM_OFFSET, TRAPPED_SUM_ORIGINAL, prepare_trapped_sum,
};
use std::{cell::Cell, ffi::c_void, marker::PhantomData, ptr::NonNull, rc::Rc};
use windows_sys::Win32::Foundation::EXCEPTION_ILLEGAL_INSTRUCTION;
use windows_sys::Win32::System::Diagnostics::Debug::{
    AddVectoredExceptionHandler, CONTEXT, CONTEXT_CONTROL_AMD64, CONTEXT_INTEGER_AMD64,
    EXCEPTION_CONTINUE_EXECUTION, EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS, EXCEPTION_RECORD,
    RemoveVectoredExceptionHandler,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Armed,
    Handling,
    Complete,
}

#[derive(Clone, Copy)]
struct ActiveTrap {
    address: u64,
    phase: Phase,
    traps: u32,
}

std::thread_local! {
    // Const initialization and a Copy, non-Drop value use Windows native TLS.
    // No heap object, destructor, borrowed scope pointer or lazy initializer is
    // needed in the callback. TLS itself makes registration thread-specific.
    static ACTIVE_TRAP: Cell<Option<ActiveTrap>> = const { Cell::new(None) };
}

/// Owns a process-wide callback registration and this thread's copied scope.
/// The function lives in the worker executable; unregister failure cannot leave
/// a callback borrowing this object, its stack, or the released native mapping.
struct TrapScope {
    handle: Option<NonNull<c_void>>,
    address: u64,
    armed: bool,
    // A scope must be cleared on its originating thread, even if its fields
    // change later to types that would otherwise permit migration.
    _same_thread: PhantomData<Rc<()>>,
}

impl TrapScope {
    fn install(address: u64) -> Result<Self, PlatformError> {
        let armed = ACTIVE_TRAP
            .try_with(|slot| {
                if slot.get().is_some() {
                    return false;
                }
                slot.set(Some(ActiveTrap {
                    address,
                    phase: Phase::Armed,
                    traps: 0,
                }));
                true
            })
            .unwrap_or(false);
        if !armed {
            return Err(PlatformError::TrapScopeUnavailable);
        }
        let mut scope = Self {
            handle: None,
            address,
            armed: true,
            _same_thread: PhantomData,
        };
        // SAFETY: The callback has the Windows ABI, a process-lifetime code
        // address and no captured or borrowed registration state. Its scope is
        // already initialized on this thread before Windows can invoke it.
        let handle = unsafe { AddVectoredExceptionHandler(1, Some(fixture_trap_handler)) };
        scope.handle = NonNull::new(handle);
        if scope.handle.is_none() {
            // Drop clears the copied TLS scope even when registration failed.
            return Err(PlatformError::TrapHandlerRegistrationFailed);
        }
        Ok(scope)
    }

    fn traps(&self) -> u32 {
        ACTIVE_TRAP
            .try_with(|slot| {
                slot.get()
                    .filter(|state| state.address == self.address)
                    .map_or(0, |state| state.traps)
            })
            .unwrap_or(0)
    }

    fn close(mut self) -> Result<(), PlatformError> {
        self.close_with(remove_handler)
    }

    fn close_with(
        &mut self,
        remove: impl FnOnce(NonNull<c_void>) -> bool,
    ) -> Result<(), PlatformError> {
        // Inactivate first. A failed removal may leave only the static callback,
        // which sees None and continues the search without using old addresses.
        if self.armed {
            let _ = ACTIVE_TRAP.try_with(|slot| slot.set(None));
            self.armed = false;
        }
        if let Some(handle) = self.handle.take()
            && !remove(handle)
        {
            return Err(PlatformError::TrapHandlerRemovalFailed);
        }
        Ok(())
    }
}

impl Drop for TrapScope {
    fn drop(&mut self) {
        let _ = self.close_with(remove_handler);
    }
}

fn remove_handler(handle: NonNull<c_void>) -> bool {
    // SAFETY: The handle came from this scope's successful registration. It is
    // consumed once after TLS was cleared. No callback borrows the scope.
    unsafe { RemoveVectoredExceptionHandler(handle.as_ptr()) != 0 }
}

unsafe extern "system" fn fixture_trap_handler(exception: *mut EXCEPTION_POINTERS) -> i32 {
    ACTIVE_TRAP
        .try_with(|slot| {
            let Some(mut state) = slot.get() else {
                return EXCEPTION_CONTINUE_SEARCH;
            };
            if state.phase != Phase::Armed {
                return EXCEPTION_CONTINUE_SEARCH;
            }
            // SAFETY: Windows supplies valid exception/context records during
            // the callback. Tests supply the same live, nonaliasing structures.
            // Null records are rejected before any dereference.
            let Some(pointers) = (unsafe { exception.as_ref() }) else {
                return EXCEPTION_CONTINUE_SEARCH;
            };
            // SAFETY: The same Windows/test record-lifetime contract applies.
            let Some(record) = (unsafe { pointers.ExceptionRecord.as_ref() }) else {
                return EXCEPTION_CONTINUE_SEARCH;
            };
            // SAFETY: Windows grants the handler this mutable saved context;
            // it does not alias record or another Rust reference in this call.
            let Some(context) = (unsafe { pointers.ContextRecord.as_mut() }) else {
                return EXCEPTION_CONTINUE_SEARCH;
            };
            state.phase = Phase::Handling;
            slot.set(Some(state));
            let handled = apply_registered_trap(state.address, record, context);
            if handled {
                state.phase = Phase::Complete;
                state.traps = 1;
            } else {
                state.phase = Phase::Armed;
            }
            slot.set(Some(state));
            if handled {
                EXCEPTION_CONTINUE_EXECUTION
            } else {
                EXCEPTION_CONTINUE_SEARCH
            }
        })
        .unwrap_or(EXCEPTION_CONTINUE_SEARCH)
}

/// No allocation, synchronization, I/O, indexing, panic or host-service call.
/// All rejection checks precede mutation of the saved guest context.
fn apply_registered_trap(address: u64, record: &EXCEPTION_RECORD, context: &mut CONTEXT) -> bool {
    let required = CONTEXT_CONTROL_AMD64 | CONTEXT_INTEGER_AMD64;
    if record.ExceptionCode != EXCEPTION_ILLEGAL_INSTRUCTION
        || record.ExceptionFlags != 0
        || record.ExceptionAddress as usize as u64 != address
        || context.ContextFlags & required != required
        || context.Rip != address
        || context.Rax != SYNTHETIC_SUM_SERVICE
        || context.Rdi != 19
        || context.Rsi != 23
    {
        return false;
    }
    let Some(continuation) = address.checked_add(2) else {
        return false;
    };
    let Some(value) = context.Rdi.checked_add(context.Rsi) else {
        return false;
    };
    // The synthetic service explicitly models the architectural RCX/R11
    // clobbers. Other registers and saved flags remain unchanged. This is not
    // a claim about PS5 syscall argument, errno or return-flag conventions.
    context.Rax = value;
    context.Rcx = continuation;
    context.R11 = u64::from(context.EFlags);
    context.Rip = continuation;
    true
}

pub(super) fn execute() -> Result<OriginalFixtureResult, PlatformError> {
    // Attest and replace the only SYSCALL in an ordinary owned byte array before
    // an executable mapping can exist. Caller-provided bytes are never accepted.
    let prepared = prepare_trapped_sum(&TRAPPED_SUM_ORIGINAL)?;
    let mut staging = GuestReservation::reserve(GUEST_PAGE_SIZE)?.commit_writable()?;
    staging.write(0, &prepared)?;
    let executable = staging.seal_executable()?;
    let address = executable
        .address()
        .checked_add(TRAPPED_SUM_OFFSET as u64)
        .ok_or(PlatformError::OutOfBounds)?;
    let scope = TrapScope::install(address)?;
    // SAFETY: The exact original leaf was attested and its SYSCALL replaced by
    // UD2 before RX. Its registered thread/site/service scope remains alive.
    // It uses the current valid Windows stack without a red zone or guest TLS,
    // changes only SysV volatile registers, and returns through its original RET.
    let value = unsafe { executable.region.call_original_fixture() };
    let traps = scope.traps();
    scope.close()?;
    if value != 42 || traps != 1 {
        return Err(PlatformError::UnexpectedFixtureTrapResult { value, traps });
    }
    Ok(OriginalFixtureResult { value, traps })
}

#[cfg(test)]
#[path = "traps/tests.rs"]
mod tests;
