// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded owned references and atomic guest output.
// Adapted from pinned src/libs/libSysmodule.cpp; see NOTICE.

use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestWrite, Limits, Service};

const UNLOADED: u64 = 0x805a_1001;

/// Source-listed application modules, not system/firmware search paths.
/// Table adapted from KytyPS5 pull1266, revision1125a0013eea7bdf54cda852c515ae0c45ef7e06.
pub const fn game_module_path(id: u16) -> Option<&'static str> {
    Some(match id {
        0x0038 => "/app0/sce_module/libSceFace.prx",
        0x0039 => "/app0/sce_module/libSceSmart.prx",
        0x0086 => "/app0/sce_module/libSceS3DConversion.prx",
        0x0093 => "/app0/sce_module/libSceHand.prx",
        0x00cb => "/app0/sce_module/libSceHeadTracker.prx",
        0x00d8 => "/app0/sce_module/libSceFaceTracker.prx",
        0x00d9 => "/app0/sce_module/libSceHandTracker.prx",
        0x00e8 => "/app0/sce_module/libSceAudioLatencyEstimation.prx",
        0x00f7 => "/app0/sce_module/libSceNpToolkit.prx",
        0x00fe => "/app0/sce_module/libSceJobManager.prx",
        0x0102 => "/app0/sce_module/libSceNpToolkit2.prx",
        0x0115 => "/app0/sce_module/libSceNpCppWebApi.prx",
        0x0135 => "/app0/sce_module/libSceFontGsm.prx",
        _ => return None,
    })
}

pub(crate) fn load_output(service: Service, args: CallArgs) -> Result<Option<u64>, Failure> {
    match service {
        Service::SysmoduleLoad => Ok(None),
        Service::SysmoduleLoadInternal => {
            if args.0[1..4].iter().any(|value| *value as u32 != 0) || args.0[4] == 0 {
                return Err(Failure::Unsupported("system module load arguments"));
            }
            Ok(Some(args.0[4]))
        }
        _ => Err(Failure::Unsupported("system module load service")),
    }
}

/// The pinned HLE module activation table, independent of loaded guest images.
/// A record does not provide missing exports or authorize firmware execution.
pub(crate) struct ModuleState {
    entries: Vec<(u16, u32)>,
}

impl ModuleState {
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn new(limits: Limits) -> Result<Self, Failure> {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        Ok(Self { entries })
    }

    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        self.invoke_inner(service, args, guest, budget, false)
    }

    pub(crate) fn complete_load(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        load_output(service, args)?;
        self.invoke_inner(service, args, guest, budget, true)
    }

    fn invoke_inner(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        precharged_output: bool,
    ) -> Result<u64, Failure> {
        use Service::*;
        budget.work(self.entries.len() as u64 + 1)?;
        // The source parameter is uint16_t; unused upper register bits do not
        // form a second module identity.
        let id = args.0[0] as u16;
        let index = self.entries.iter().position(|entry| entry.0 == id);
        match service {
            SysmoduleIsLoaded => Ok(if index.is_some() { 0 } else { UNLOADED }),
            SysmoduleUnload => {
                let Some(index) = index else {
                    return Ok(UNLOADED);
                };
                if self.entries[index].1 == 1 {
                    self.entries.swap_remove(index);
                    budget.removed_handle();
                } else {
                    self.entries[index].1 -= 1;
                }
                Ok(0)
            }
            SysmoduleLoad | SysmoduleLoadInternal => {
                let output = load_output(service, args)?;
                let references = if let Some(index) = index {
                    self.entries[index]
                        .1
                        .checked_add(1)
                        .ok_or(Failure::Limit("system module references"))?
                } else {
                    budget.can_add_handle()?;
                    1
                };
                // Preallocated metadata and all limits are admitted before the
                // output transaction; publication below cannot allocate/fail.
                if let Some(address) = output {
                    let writes = [GuestWrite {
                        address,
                        bytes: &0_i32.to_le_bytes(),
                    }];
                    if precharged_output {
                        // The opaque admission reserved this exact four-byte
                        // output. Still apply it transactionally before state.
                        guest
                            .apply(crate::MemoryEffect::None, &writes)
                            .map_err(Failure::Guest)?;
                    } else {
                        budget.write(guest, &writes)?;
                    }
                }
                if let Some(index) = index {
                    self.entries[index].1 = references;
                } else {
                    self.entries.push((id, references));
                    budget.added_handle();
                }
                Ok(0)
            }
            _ => Err(Failure::Unsupported("system module service")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct NoGuest;
    impl GuestAccess for NoGuest {
        fn read(&self, _: u64, _: &mut [u8]) -> Result<(), crate::GuestError> {
            panic!("reference overflow must not read guest backing")
        }
        fn apply(
            &mut self,
            _: crate::MemoryEffect,
            _: &[GuestWrite<'_>],
        ) -> Result<(), crate::GuestError> {
            panic!("reference overflow must not publish guest output")
        }
    }

    #[test]
    fn sysmodule_reference_overflow_keeps_record_and_output_unchanged() {
        let mut state = ModuleState {
            entries: vec![(37, u32::MAX)],
        };
        let mut budget = Budget::new(Limits::default()).unwrap();
        for service in [Service::SysmoduleLoad, Service::SysmoduleLoadInternal] {
            assert_eq!(
                state.invoke(
                    service,
                    CallArgs([37, 0, 0, 0, 16, 0]),
                    &mut NoGuest,
                    &mut budget
                ),
                Err(Failure::Limit("system module references"))
            );
            assert_eq!(state.entries, [(37, u32::MAX)]);
        }
    }
}
