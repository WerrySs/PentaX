// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: detach actual registered guest pthreads.
// Adapted from pinned kernel/pthread.cpp PthreadDetach; see NOTICE.
use super::*;
use pentax_kernel::{Errno, Failure, GuestAccess, Service, pthread_detach_error};

impl ThreadRuntime {
    fn detach(
        &mut self,
        kernel: &mut Kernel,
        service: Service,
        args: CallArgs,
        guest: &impl GuestAccess,
    ) -> Result<u64, Failure> {
        let index = self
            .records
            .iter()
            .position(|row| row.layout.pthread_pointer == args.0[0]);
        if let Some(index) = index {
            let row = &self.records[index];
            if !row.published || row.joined {
                return pthread_detach_error(service, Errno::NoSuchResource);
            }
            // A successful Join admission already owns the target. Do not
            // invalidate that parked continuation or discard its true result.
            if row.join_claim.is_some() {
                return pthread_detach_error(service, Errno::Invalid);
            }
        }
        // Kernel registration also covers the actual initial/current thread,
        // which has no created-thread record. Numeric unregistered handles
        // cannot detach anything. All fallible work precedes both state writes.
        let value = kernel.complete_pthread_detach(service, args.0[0], guest)?;
        if value == 0 {
            if let Some(index) = index {
                self.records[index].attributes.detached = true;
            }
        }
        Ok(value)
    }
}

pub(in super::super) fn begin(
    kernel: &mut Kernel,
    memory: &mut KernelMemory<'_, '_, '_>,
    runtime: &mut ThreadRuntime,
    service: Service,
    args: CallArgs,
) -> Result<NativeServiceAction, KernelStartupError> {
    if let Some(value) = kernel
        .admit_pthread_detach(service, args, memory)
        .map_err(KernelStartupError::Kernel)?
    {
        return Ok(NativeServiceAction::Returned(value));
    }
    let maximum = memory
        .lookup_limits
        .ok_or(invalid("pthread detach work"))?
        .max_work;
    charge(
        memory
            .lookup_work
            .as_deref_mut()
            .ok_or(invalid("pthread detach work"))?,
        runtime.records.len() as u64 + 1,
        maximum,
    )?;
    runtime
        .detach(kernel, service, args, memory)
        .map(NativeServiceAction::Returned)
        .map_err(KernelStartupError::Kernel)
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests;
