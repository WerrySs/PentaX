// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c)2021 InoriRus
// Modified 2026 for PentaX: bounded guest writes and an explicit system-name profile.
// Adapted from WerrySs/KytyPS5-Performance@209dffe9032d72749257ce18680d506b891284de,
// src/libs/libSystemService.cpp, Git blob 690b813be6ceac9c707e06c5fff6ec3c70d13acb.
// See NOTICE for the retained upstream license and provenance.

use crate::{CallArgs, Failure, GuestAccess, GuestWrite, Kernel, Service};

const PARAMETER_ERROR: u64 = 0x80a1_0003;
// Deliberate branding adaptation of the source's fixed "Kyty" system name.
// This profile has no host name lookup or configurable/allocated string.
const SYSTEM_NAME: &[u8; 7] = b"PentaX\0";

impl Kernel {
    pub(crate) fn invoke_system_notice(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        self.budget.work(1)?;
        match service {
            Service::SystemHideSplashScreen
            | Service::SystemDisableNoticeScreenSkipFlagAutoSet
            | Service::SystemSetNoticeScreenSkipFlag
            | Service::SystemPowerTick
            | Service::SystemReportAbnormalTermination => {
                // Source no-ops. In particular Set/Disable do not mutate the
                // configured flag, and the abnormal-info pointer is opaque.
                Ok(0)
            }
            Service::SystemGetNoticeScreenSkipFlag => {
                let output = args.0[0];
                if output == 0 {
                    return Ok(PARAMETER_ERROR);
                }
                self.budget.write(
                    guest,
                    &[GuestWrite {
                        address: output,
                        bytes: &[u8::from(self.system.skip_notice_screen)],
                    }],
                )?;
                Ok(0)
            }
            Service::SystemParamGetString => {
                let [id, output, capacity, ..] = args.0;
                if output == 0 || capacity == 0 {
                    return Ok(PARAMETER_ERROR);
                }
                if id as u32 as i32 != 6 {
                    return Err(Failure::Unsupported("system string parameter"));
                }
                if capacity < SYSTEM_NAME.len() as u64 {
                    return Ok(PARAMETER_ERROR);
                }
                self.budget.write(
                    guest,
                    &[GuestWrite {
                        address: output,
                        bytes: SYSTEM_NAME,
                    }],
                )?;
                Ok(0)
            }
            _ => Err(Failure::Unsupported("system notice service")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{ARG, PAGE, TestGuest, kernel};
    use crate::{GuestError, Limits, MemoryEffect, SystemConfig};

    struct NoGuest;

    impl GuestAccess for NoGuest {
        fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
            panic!("opaque argument must not be read")
        }

        fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
            panic!("no guest publication is permitted")
        }
    }

    struct WriteOnly(TestGuest);

    impl GuestAccess for WriteOnly {
        fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
            panic!("output must not require reading its previous payload")
        }

        fn apply(
            &mut self,
            effect: MemoryEffect,
            writes: &[GuestWrite<'_>],
        ) -> Result<(), GuestError> {
            self.0.apply(effect, writes)
        }
    }

    fn call(
        owner: &mut Kernel,
        guest: &mut impl GuestAccess,
        library: bool,
        service: Service,
        args: [u64; 3],
    ) -> Result<u64, Failure> {
        let args = CallArgs([args[0], args[1], args[2], 0, 0, 0]);
        if library {
            owner.invoke_library(service, args, guest)
        } else {
            owner.invoke(service, args, guest)
        }
    }

    #[test]
    fn notice_flag_uses_default_or_owned_config_and_writes_exactly_one_byte_without_reads() {
        for library in [false, true] {
            let mut owner = kernel(Limits::default());
            assert!(!owner.system.skip_notice_screen);
            let mut guest = WriteOnly(TestGuest::new());
            for value in [false, true] {
                owner
                    .configure_system(SystemConfig {
                        skip_notice_screen: value,
                        ..Default::default()
                    })
                    .unwrap();
                guest.0.put(PAGE - 2, &[0xa5; 2]);
                assert_eq!(
                    call(
                        &mut owner,
                        &mut guest,
                        library,
                        Service::SystemGetNoticeScreenSkipFlag,
                        [ARG + PAGE - 1, 0, 0],
                    ),
                    Ok(0)
                );
                assert_eq!(guest.0.get(ARG + PAGE - 2, 2), [0xa5, u8::from(value)]);
                assert_eq!(owner.system.skip_notice_screen, value);
            }
            assert_eq!(owner.usage().bytes_read, 0);
            assert_eq!(owner.usage().bytes_written, 2);
            assert_eq!(owner.usage().handles_live, 0);
        }
    }

    #[test]
    fn source_noops_keep_configuration_and_opaque_arguments_without_guest_or_host_effects() {
        let services = [
            Service::SystemHideSplashScreen,
            Service::SystemDisableNoticeScreenSkipFlagAutoSet,
            Service::SystemSetNoticeScreenSkipFlag,
            Service::SystemPowerTick,
            Service::SystemReportAbnormalTermination,
        ];
        let mut owner = kernel(Limits::default());
        let mut guest = NoGuest;
        for value in [false, true] {
            let config = SystemConfig {
                skip_notice_screen: value,
                ..Default::default()
            };
            owner.configure_system(config).unwrap();
            for library in [false, true] {
                for service in services {
                    assert_eq!(
                        call(&mut owner, &mut guest, library, service, [u64::MAX; 3]),
                        Ok(0)
                    );
                    assert_eq!(owner.system, config);
                }
            }
        }
        assert_eq!(owner.usage().attempts, 20);
        assert_eq!(owner.usage().bytes_read, 0);
        assert_eq!(owner.usage().bytes_written, 0);
        assert_eq!(owner.usage().handles_live, 0);
        assert_eq!(owner.usage().mapped_bytes, 0);
        let mut exhausted = kernel(Limits {
            max_work: 0,
            ..Default::default()
        });
        assert!(matches!(
            call(
                &mut exhausted,
                &mut guest,
                false,
                Service::SystemPowerTick,
                [0; 3],
            ),
            Err(Failure::Limit(_))
        ));
    }

    #[test]
    fn string_source_error_order_and_integer_widths_match_raw_and_library_domains() {
        for library in [false, true] {
            let mut owner = kernel(Limits::default());
            let mut opaque = NoGuest;
            assert_eq!(
                call(
                    &mut owner,
                    &mut opaque,
                    library,
                    Service::SystemGetNoticeScreenSkipFlag,
                    [0; 3],
                ),
                Ok(PARAMETER_ERROR)
            );
            for args in [[u64::MAX, 0, u64::MAX], [u64::MAX, u64::MAX, 0]] {
                assert_eq!(
                    call(
                        &mut owner,
                        &mut opaque,
                        library,
                        Service::SystemParamGetString,
                        args,
                    ),
                    Ok(PARAMETER_ERROR)
                );
            }
            for id in [5, u64::MAX, 0x1_0000_0005] {
                assert_eq!(
                    call(
                        &mut owner,
                        &mut opaque,
                        library,
                        Service::SystemParamGetString,
                        [id, u64::MAX, 1],
                    ),
                    Err(Failure::Unsupported("system string parameter"))
                );
            }
            for size in [1, 6] {
                assert_eq!(
                    call(
                        &mut owner,
                        &mut opaque,
                        library,
                        Service::SystemParamGetString,
                        [6, u64::MAX, size],
                    ),
                    Ok(PARAMETER_ERROR)
                );
            }
            let mut guest = WriteOnly(TestGuest::new());
            guest.0.put(0, &[0xa5; 16]);
            assert_eq!(
                call(
                    &mut owner,
                    &mut guest,
                    library,
                    Service::SystemParamGetString,
                    [0xffff_ffff_0000_0006, ARG + 2, 0x1_0000_0001],
                ),
                Ok(0)
            );
            assert_eq!(guest.0.get(ARG, 2), [0xa5; 2]);
            assert_eq!(guest.0.get(ARG + 2, 7), SYSTEM_NAME.as_slice());
            assert_eq!(guest.0.get(ARG + 9, 7), [0xa5; 7]);
            assert_eq!(owner.usage().bytes_read, 0);
            assert_eq!(owner.usage().bytes_written, 7);
        }
    }

    #[test]
    fn publication_failure_and_byte_budget_are_atomic_and_retry_keeps_config() {
        let mut owner = kernel(Limits::default());
        owner
            .configure_system(SystemConfig {
                skip_notice_screen: true,
                ..Default::default()
            })
            .unwrap();
        let mut guest = TestGuest::new();
        guest.put(PAGE - 8, &[0xa5; 8]);
        assert!(matches!(
            call(
                &mut owner,
                &mut guest,
                true,
                Service::SystemParamGetString,
                [6, ARG + PAGE - 6, u64::MAX],
            ),
            Err(Failure::Guest(_))
        ));
        assert_eq!(guest.get(ARG + PAGE - 8, 8), [0xa5; 8]);
        guest.refuse = true;
        assert_eq!(
            call(
                &mut owner,
                &mut guest,
                false,
                Service::SystemGetNoticeScreenSkipFlag,
                [ARG + PAGE - 8, 0, 0],
            ),
            Err(Failure::Guest(GuestError::WriteDenied))
        );
        assert_eq!(guest.get(ARG + PAGE - 8, 8), [0xa5; 8]);
        guest.refuse = false;
        assert_eq!(
            call(
                &mut owner,
                &mut guest,
                true,
                Service::SystemParamGetString,
                [6, ARG + PAGE - 7, 7],
            ),
            Ok(0)
        );
        assert_eq!(guest.get(ARG + PAGE - 8, 1), [0xa5]);
        assert_eq!(guest.get(ARG + PAGE - 7, 7), SYSTEM_NAME.as_slice());
        assert!(owner.system.skip_notice_screen);
        assert_eq!(owner.usage().bytes_written, 15);
        assert_eq!(owner.usage().bytes_read, 0);
        assert_eq!(owner.usage().handles_live, 0);
        let before = guest.get(ARG + PAGE - 8, 8);
        let mut limited = kernel(Limits {
            max_bytes: 6,
            ..Default::default()
        });
        assert_eq!(
            call(
                &mut limited,
                &mut guest,
                false,
                Service::SystemParamGetString,
                [6, ARG + PAGE - 7, 7],
            ),
            Err(Failure::Limit("guest bytes"))
        );
        assert_eq!(guest.get(ARG + PAGE - 8, 8), before);
    }
}
