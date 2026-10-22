// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: explicit source library error return widths.
// Adapted from pinned kernel headers and errno.h; see NOTICE.

//! Exact direct-library error domains; not POSIX wrappers or raw syscalls.

use crate::{CallArgs, Failure, GuestAccess, Kernel, Service};

enum ErrorDomain {
    KernelInt32,
    KernelInt64,
}

fn error_domain(service: Service) -> Option<ErrorDomain> {
    use Service::*;
    match service {
        // fileSystem.h: these return int64_t and promote negative source ints.
        Read | Write | Pread | Pwrite | Lseek => Some(ErrorDomain::KernelInt64),
        // fileSystem.h, memory.h, semaphore.h, eventFlag.h and pthread.h:
        // these exact registered functions return int, not a POSIX -1/errno.
        CreateEqueue
        | DeleteEqueue
        | WaitEqueue
        | GetEventFilter
        | GetEventError
        | AddUserEvent
        | AddUserEventEdge
        | TriggerUserEvent
        | DeleteUserEvent
        | AddTimerEvent
        | AddHRTimerEvent
        | DeleteHRTimerEvent
        | AmprConstruct
        | AmprAprConstruct
        | AmprAprDestruct
        | AmprDestruct
        | AmprSetBuffer
        | AmprReset
        | AmprReadFile
        // Source KernelErrorU64 explicitly zero-extends its int32 error bits.
        | AmprMeasureReadFile
        | Open
        | Close
        | Fstat
        | Ftruncate
        | Fsync
        | Mkdir
        | Rmdir
        | Rename
        | Unlink
        | Stat
        | CheckReachability
        | Getdirentries
        | Getdents
        | AllocateDirect
        | AllocateMainDirect
        | ConfiguredFlexibleMemorySize
        | ReleaseDirect
        | MapDirect
        | MapNamedDirect
        | MapFlexible
        | MapNamedFlexible
        | Munmap
        | Mprotect
        | QueryProtection
        | SetVirtualRangeName
        | ClearVirtualRangeName
        | CreateSema
        | DeleteSema
        | WaitSema
        | PollSema
        | SignalSema
        | CancelSema
        | CreateEventFlag
        | DeleteEventFlag
        | WaitEventFlag
        | PollEventFlag
        | SetEventFlag
        | ClearEventFlag
        | CancelEventFlag
        | ClockGettime
        | ClockGetres
        | Gettimeofday
        | Gettimezone
        | ConvertUtcToLocaltime
        | ConvertLocaltimeToUtc
        | Usleep
        | Nanosleep
        | PthreadSemInit
        | PthreadSemDestroy
        | PthreadSemWait
        | PthreadSemTryWait
        | PthreadSemTimedWait
        | PthreadSemPost
        | PthreadSemGetValue
        | MutexAttrInit
        | MutexAttrDestroy
        | MutexAttrSetType
        | MutexAttrSetProtocol
        | MutexInit
        | MutexDestroy
        | MutexLock
        | MutexTryLock
        | MutexTimedLock
        | MutexUnlock
        | KeyCreate
        | KeyDelete
        | SetSpecific
        | CondAttrInit
        | CondAttrDestroy
        | CondAttrSetClock
        | CondInit
        | CondDestroy
        | CondSignal
        | CondBroadcast
        | RwlockInit
        | RwlockDestroy
        | RwlockReadLock
        | RwlockWriteLock
        | RwlockTryReadLock
        | RwlockTryWriteLock
        | RwlockTimedReadLock
        | RwlockTimedWriteLock
        | RwlockUnlock
        | RwlockAttrInit
        | RwlockAttrDestroy
        | RwlockAttrSetType
        | ThreadAttrDestroy
        | ThreadAttrGet
        | ThreadAttrGetAffinity
        | ThreadAttrInit
        | ThreadAttrGetDetachState
        | ThreadAttrGetStack
        | ThreadAttrGetStackAddr
        | ThreadAttrGetStackSize
        | ThreadAttrGetGuardSize
        | ThreadAttrGetSoloSched
        | ThreadAttrSetStack
        | ThreadAttrSetStackAddr
        | ThreadAttrSetStackSize
        | ThreadAttrSetDetachState
        | ThreadAttrSetGuardSize
        | ThreadAttrSetInheritSched
        | ThreadAttrSetSchedParam
        | ThreadAttrSetSchedPolicy
        | ThreadAttrSetSoloSched
        | ThreadAttrSetAffinity
        | ThreadAttrGetSchedParam
        | ThreadAttrGetSchedPolicy => Some(ErrorDomain::KernelInt32),
        PthreadSetaffinity | PthreadGetaffinity | PthreadGetprio | PthreadSetprio => {
            Some(ErrorDomain::KernelInt32)
        }
        // Pointer/unsigned/thread/lifecycle domains have no blanket conversion.
        // Dlsym already implements its observed null/miss output transactions;
        // its adapter failures must not become invented lookup results.
        _ => None,
    }
}
impl Kernel {
    /// Invoke the pinned library-call ABI, retaining one normal service attempt.
    /// Only supported source errno outcomes for explicitly audited functions
    /// become register results. Int32 errors are zero-extended; int64 errors
    /// sign-extend the source negative int. Success values are unchanged.
    /// Unsupported behavior, policy limits, allocation and backing failures
    /// remain typed stops. This does not set POSIX errno or raw-syscall carry.
    /// [`Self::invoke`] remains the raw typed service API.
    pub fn invoke_library(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        self.invoke_library_with_stack(service, args, None, guest)
    }

    /// The same audited result conversion with an optional captured callee RSP.
    /// A failed stack read remains a typed stop, never a synthetic source errno.
    pub fn invoke_library_with_stack(
        &mut self,
        service: Service,
        args: CallArgs,
        stack_pointer: Option<u64>,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        self.invoke_library_with_vectors(service, args, stack_pointer, None, guest)
    }

    /// Preserve captured SysV vector registers alongside the original GP and
    /// stack arguments. An absent capture is never an invented zero vector.
    pub fn invoke_library_with_vectors(
        &mut self,
        service: Service,
        args: CallArgs,
        stack_pointer: Option<u64>,
        vector_args: Option<[[u8; 16]; 8]>,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        match (
            self.invoke_with_vectors(service, args, stack_pointer, vector_args, guest),
            error_domain(service),
        ) {
            (Err(Failure::Errno(errno)), Some(ErrorDomain::KernelInt32)) => {
                Ok(u64::from(errno.kernel_code() as u32))
            }
            (Err(Failure::Errno(errno)), Some(ErrorDomain::KernelInt64)) => {
                Ok(i64::from(errno.kernel_code()) as u64)
            }
            (result, _) => result,
        }
    }
}
