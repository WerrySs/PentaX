// SPDX-License-Identifier: GPL-2.0-only
// Public AnyPS5@80565c59424190cd0f660f8097945443a0017c42 two-argument ABI;
// Export.cpp Git blob 7a4c11d33eabba6985932131bf1ddeec3466ad91. See NOTICE.
// This single qualified tuple is a PentaX compatibility binding, not an
// assertion that either public reference registers this exact namespace.

use super::{Registration, Service};

pub(super) const USER_PRIVACY_REGISTRATIONS: [Registration; 1] = [Registration {
    service: Service::UserGetPlatformPrivacyWs1,
    nid: "D-CzAxQL0XI",
    source_name: "sceUserServiceGetPlatformPrivacyWs1",
    arguments: 2,
    module: "libSceUserService",
    module_version: (1, 1),
    library: "libSceUserServicePlatformPrivacyWs1",
    library_version: 1,
}];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{REGISTRATIONS, resolve};

    #[test]
    fn privacy_ws1_appends_one_exact_two_argument_qualified_binding() {
        assert_eq!(REGISTRATIONS.len(), 1326);
        assert_eq!(&REGISTRATIONS[1316..1317], &USER_PRIVACY_REGISTRATIONS);
        assert_eq!(
            Service::UserGetPlatformPrivacyWs1 as usize,
            Service::AudioOut2PortSetAttributes as usize + 1
        );
        let row = USER_PRIVACY_REGISTRATIONS[0];
        assert_eq!(
            (row.arguments, row.source_name),
            (2, "sceUserServiceGetPlatformPrivacyWs1")
        );
        assert_eq!(
            resolve(
                row.module,
                row.module_version,
                row.library,
                row.library_version,
                row.nid
            ),
            Some(row.service)
        );
        for (module, library, major, minor, version, nid) in [
            ("UserService", "UserService", 1, 1, 1, "D-CzAxQL0XI"),
            (
                "libSceUserService",
                "libSceUserService",
                1,
                1,
                1,
                "D-CzAxQL0XI",
            ),
            (
                "UserService",
                "libSceUserServicePlatformPrivacyWs1",
                1,
                1,
                1,
                "D-CzAxQL0XI",
            ),
            (
                "libSceUserService",
                "UserServicePlatformPrivacyWs1",
                1,
                1,
                1,
                "D-CzAxQL0XI",
            ),
            (
                "libSceUserService",
                "libSceUserServicePlatformPrivacyWs1",
                1,
                0,
                1,
                "D-CzAxQL0XI",
            ),
            (
                "libSceUserService",
                "libSceUserServicePlatformPrivacyWs1",
                2,
                1,
                1,
                "D-CzAxQL0XI",
            ),
            (
                "libSceUserService",
                "libSceUserServicePlatformPrivacyWs1",
                1,
                1,
                2,
                "D-CzAxQL0XI",
            ),
            (
                "libSceUserService",
                "libSceUserServicePlatformPrivacyWs1",
                1,
                1,
                1,
                "Iem2arLgE+I",
            ),
        ] {
            assert_eq!(resolve(module, (major, minor), library, version, nid), None);
        }
    }
}
