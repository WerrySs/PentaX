// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned attribute snapshots and delayed publication.
// Adapted from pinned kernel/pthread.cpp; see NOTICE.

use crate::{CallArgs, Errno, Failure, GuestAccess, GuestWrite, Kernel, ThreadAttributes};

/// Creation request data. Only the runtime can admit and schedule the actual
/// entry, independent TLS and stack before publishing a successful result.
pub struct AdmittedPthreadCreate {
    output: u64,
    entry: u64,
    argument: u64,
    attributes: ThreadAttributes,
    name: String,
}
impl std::fmt::Debug for AdmittedPthreadCreate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmittedPthreadCreate")
            .finish_non_exhaustive()
    }
}
impl AdmittedPthreadCreate {
    pub const fn output(&self) -> u64 {
        self.output
    }
    pub const fn entry(&self) -> u64 {
        self.entry
    }
    pub const fn argument(&self) -> u64 {
        self.argument
    }
    pub const fn attributes(&self) -> ThreadAttributes {
        self.attributes
    }
    pub fn name(&self) -> &str {
        &self.name
    }
}
#[derive(Debug)]
pub enum PthreadCreateAction {
    Returned(u64),
    Create(AdmittedPthreadCreate),
}

impl Kernel {
    /// Consume one library attempt and snapshot guest configuration without
    /// writing the output or manufacturing a running pthread handle.
    pub fn admit_pthread_create(
        &mut self,
        service: crate::Service,
        args: CallArgs,
        guest: &impl GuestAccess,
    ) -> Result<PthreadCreateAction, Failure> {
        use crate::Service::*;
        if !matches!(
            service,
            PthreadCreate | PosixPthreadCreate | PosixPthreadCreateName
        ) {
            return Err(Failure::Unsupported("thread creation service"));
        }
        self.budget.attempt()?;
        guest.preflight_pthread()?;
        self.budget.work(16)?;
        let [output, attribute, entry, argument, mut name, _] = args.0;
        if service == PosixPthreadCreate {
            name = 0;
        }
        if output == 0 {
            return Ok(PthreadCreateAction::Returned(if service == PthreadCreate {
                Errno::Invalid.kernel_code() as u32 as u64
            } else {
                Errno::Invalid as u64
            }));
        }
        let attributes = self
            .pthread_attr
            .creation_values(attribute, guest, &mut self.budget)?;
        let name = if name == 0 {
            String::new()
        } else {
            self.budget.name(guest, name)?
        };
        Ok(PthreadCreateAction::Create(AdmittedPthreadCreate {
            output,
            entry,
            argument,
            attributes,
            name,
        }))
    }

    /// Publish an already registered runtime-owned handle after the runtime has
    /// admitted its executable context and ensured its run-queue insertion.
    /// The entry frame and handle are one atomic output; neither is written on
    /// failure, including invalid or overlapping caller output spans.
    pub fn publish_pthread_create(
        &mut self,
        request: &AdmittedPthreadCreate,
        context: u64,
        pointer: u64,
        entry_frame: GuestWrite<'_>,
        guest: &mut impl GuestAccess,
    ) -> Result<(), Failure> {
        guest.preflight_pthread()?;
        self.threads
            .require_registered(context, pointer, &mut self.budget)?;
        self.budget.write(
            guest,
            &[
                entry_frame,
                GuestWrite {
                    address: request.output,
                    bytes: &pointer.to_le_bytes(),
                },
            ],
        )
    }
}
