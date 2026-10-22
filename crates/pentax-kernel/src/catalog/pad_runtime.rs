// SPDX-License-Identifier: GPL-2.0-only
// Exact libScePad profile, appended without renumbering. See NOTICE.
use super::{Registration, Service};

pub(super) const PAD_RUNTIME_REGISTRATIONS: [Registration; 7] = {
    use Service::*;
    let rows = [
        (
            PadSetTiltCorrectionState,
            "vDLMoJLde8I",
            "PadSetTiltCorrectionState",
            2,
        ),
        (
            PadGetTriggerEffectState,
            "znaWI0gpuo8",
            "PadGetTriggerEffectState",
            2,
        ),
        (PadSetVibration, "yFVnOdGxvZY", "PadSetVibration", 2),
        (PadRead, "q1cHNfGycLI", "PadRead", 3),
        (
            PadGetControllerInformation,
            "gjP9-KQzoUk",
            "PadGetControllerInformation",
            2,
        ),
        (PadSetTriggerEffect, "2JgFB2n9oUM", "PadSetTriggerEffect", 2),
        (PadSetLightBar, "RR4novUEENY", "PadSetLightBar", 2),
    ];
    let mut result = [Registration {
        service: PadRead,
        nid: "",
        source_name: "",
        arguments: 0,
        module: "libScePad",
        module_version: (1, 1),
        library: "libScePad",
        library_version: 1,
    }; 7];
    let mut index = 0;
    while index < rows.len() {
        let (service, nid, source_name, arguments) = rows[index];
        result[index] = Registration {
            service,
            nid,
            source_name,
            arguments,
            ..result[index]
        };
        index += 1;
    }
    result
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{REGISTRATIONS, resolve};

    #[test]
    fn pad_runtime_appends_seven_exact_roles_without_old_alias_expansion() {
        assert_eq!(REGISTRATIONS.len(), 1399);
        assert_eq!(&REGISTRATIONS[1318..1325], &PAD_RUNTIME_REGISTRATIONS);
        assert_eq!(
            REGISTRATIONS[1317].service,
            Service::ConfiguredFlexibleMemorySize
        );
        assert_eq!(
            Service::PadSetTiltCorrectionState as usize,
            Service::ConfiguredFlexibleMemorySize as usize + 1
        );
        for row in PAD_RUNTIME_REGISTRATIONS {
            assert_eq!(
                resolve("libScePad", (1, 1), "libScePad", 1, row.nid),
                Some(row.service)
            );
            for (module, library, version, library_version) in [
                ("Pad", "Pad", (1, 1), 1),
                ("Pad", "libScePad", (1, 1), 1),
                ("libScePad", "Pad", (1, 1), 1),
                ("libScePad", "libScePad", (1, 0), 1),
                ("libScePad", "libScePad", (2, 1), 1),
                ("libScePad", "libScePad", (1, 1), 2),
            ] {
                assert_eq!(
                    resolve(module, version, library, library_version, row.nid),
                    None
                );
            }
        }
    }
}
