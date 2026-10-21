// SPDX-License-Identifier: GPL-2.0-only
// Public KytyPS5 7b7f9a70 libConvertKeycode.cpp, Git blob
// 7b3564593f884db0cd3f62c9b843e26662caab12. See NOTICE.
// This exact full-prefix pair is an explicit PentaX compatibility binding.

use super::{Registration, Service};

pub(super) const CONVERT_KEYCODE_REGISTRATIONS: [Registration; 1] = [Registration {
    service: Service::ConvertKeycodeGetImeKeyboardType,
    nid: "mUuUOWI-C+0",
    source_name: "ConvertKeycodeGetImeKeyboardType",
    arguments: 2,
    module: "libSceConvertKeycode",
    module_version: (1, 0),
    library: "libSceConvertKeycode",
    library_version: 1,
}];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{REGISTRATIONS, resolve};

    #[test]
    fn keyboard_type_appends_one_exact_binding_without_scope_fallbacks() {
        assert_eq!(REGISTRATIONS.len(), 1326);
        assert_eq!(&REGISTRATIONS[1325..], &CONVERT_KEYCODE_REGISTRATIONS);
        assert_eq!(
            Service::ConvertKeycodeGetImeKeyboardType as usize,
            Service::PadSetLightBar as usize + 1
        );
        let row = CONVERT_KEYCODE_REGISTRATIONS[0];
        assert_eq!(row.arguments, 2);
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
        for (module, version, library, library_version, nid) in [
            ("ConvertKeycode", (1, 0), row.library, 1, row.nid),
            (row.module, (1, 0), "ConvertKeycode", 1, row.nid),
            ("ConvertKeycode", (1, 0), "ConvertKeycode", 1, row.nid),
            (row.module, (1, 1), row.library, 1, row.nid),
            (row.module, (2, 0), row.library, 1, row.nid),
            (row.module, (1, 0), row.library, 2, row.nid),
            (row.module, (1, 0), row.library, 1, "vIsoJsLvvlM"),
        ] {
            assert_eq!(
                resolve(module, version, library, library_version, nid),
                None
            );
        }
    }
}
