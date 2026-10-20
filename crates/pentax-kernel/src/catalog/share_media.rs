// SPDX-License-Identifier: GPL-2.0-only
//! Appended Share exports; the existing FeaturePermit row retains its site.
use super::{Registration, Service};

const EXPORTS: [(Service, &str, &str, u8); 14] = [
    (
        Service::ShareCaptureVideoClip,
        "4jt8pMDudgk",
        "Share::ShareCaptureVideoClip",
        2,
    ),
    (
        Service::ShareFeatureProhibit,
        "5wjxESwX68I",
        "Share::ShareFeatureProhibit",
        1,
    ),
    (
        Service::ShareSetContentParam,
        "7QZtURYnXG4",
        "Share::ShareSetContentParam",
        1,
    ),
    (
        Service::ShareOpenMenuForContent,
        "8qAJ0Jd58-Q",
        "Share::ShareOpenMenuForContent",
        1,
    ),
    (
        Service::ShareCaptureVideoClipExtended,
        "AcDNpEpoT9U",
        "Share::ShareCaptureVideoClipExtended",
        2,
    ),
    (
        Service::ShareCaptureScreenshot,
        "ErH6tKS7fzE",
        "Share::ShareCaptureScreenshot",
        2,
    ),
    (
        Service::ShareCaptureScreenshotExtended,
        "GQTObcITIXI",
        "Share::ShareCaptureScreenshotExtended",
        2,
    ),
    (
        Service::ShareUnregisterContentEventCallback,
        "KnsfHKmZqFA",
        "Share::ShareUnregisterContentEventCallback",
        1,
    ),
    (
        Service::ShareSetContentParamForApplicationTitle,
        "ORspsWDXPps",
        "Share::ShareSetContentParamForApplicationTitle",
        1,
    ),
    (
        Service::ShareGetCurrentStatus,
        "QNop2YAtIDE",
        "Share::ShareGetCurrentStatus",
        2,
    ),
    (
        Service::ShareRegisterContentEventCallback,
        "Sygnk9dr5WQ",
        "Share::ShareRegisterContentEventCallback",
        2,
    ),
    (
        Service::ShareSetScreenshotOverlayImage,
        "T64o-315wbg",
        "Share::ShareSetScreenshotOverlayImage",
        4,
    ),
    (
        Service::ShareGetRunningStatus,
        "crFxyW3HdK0",
        "Share::ShareGetRunningStatus",
        1,
    ),
    (
        Service::ShareSetCaptureSource,
        "kCurUZVFqcI",
        "Share::ShareSetCaptureSource",
        2,
    ),
];

pub(super) const SHARE_MEDIA_REGISTRATIONS: [Registration; 18] = {
    let base = Registration {
        service: Service::ShareCaptureVideoClip,
        nid: "4jt8pMDudgk",
        source_name: "Share::ShareCaptureVideoClip",
        arguments: 2,
        module: "Share",
        module_version: (1, 1),
        library: "Share",
        library_version: 1,
    };
    let mut rows = [base; 18];
    let mut index = 0;
    while index < EXPORTS.len() {
        let (service, nid, source_name, arguments) = EXPORTS[index];
        rows[index] = Registration {
            service,
            nid,
            source_name,
            arguments,
            ..base
        };
        index += 1;
    }
    let aliases = [5, 6, 7, 10];
    index = 0;
    while index < aliases.len() {
        rows[14 + index] = Registration {
            module: "libSceShare",
            library: "libSceShare",
            ..rows[aliases[index]]
        };
        index += 1;
    }
    rows
};

#[cfg(test)]
mod tests {
    use super::super::{REGISTRATIONS, resolve};
    use super::*;
    #[test]
    fn share_media_exact_append_and_winning_permit_keep_unique_identities() {
        assert_eq!(REGISTRATIONS.len(), 1283);
        assert_eq!(
            Service::ShareCaptureVideoClip as usize,
            Service::ContentDeleteInitialize as usize + 1
        );
        let permit = &REGISTRATIONS[1212];
        assert_eq!(
            (
                permit.service,
                permit.nid,
                permit.source_name,
                permit.arguments
            ),
            (
                Service::ShareFeaturePermit,
                "YBiIdcDPrxs",
                "Share::ShareFeaturePermit",
                1
            )
        );
        assert_eq!(
            REGISTRATIONS.iter().filter(|r| r.nid == permit.nid).count(),
            1
        );
        for (index, (service, nid, name, arguments)) in EXPORTS.into_iter().enumerate() {
            let row = &REGISTRATIONS[1220 + index];
            assert_eq!(
                (row.service, row.nid, row.source_name, row.arguments),
                (service, nid, name, arguments)
            );
            assert_eq!(resolve("Share", (1, 1), "Share", 1, nid), Some(service));
            assert_eq!(
                resolve("libSceShare", (1, 1), "libSceShare", 1, nid),
                if [5, 6, 7, 10].contains(&index) {
                    Some(service)
                } else {
                    None
                }
            );
            for (module, library) in [("Share", "libSceShare"), ("libSceShare", "Share")] {
                assert_eq!(resolve(module, (1, 1), library, 1, nid), None);
            }
            for scope in ["Share", "libSceShare"] {
                for (version, lib) in [((2, 1), 1), ((1, 0), 1), ((1, 1), 2)] {
                    assert_eq!(resolve(scope, version, scope, lib, nid), None);
                }
            }
        }
        for (offset, index) in [5, 6, 7, 10].into_iter().enumerate() {
            assert_eq!(
                REGISTRATIONS[1234 + offset].service,
                REGISTRATIONS[1220 + index].service
            );
        }
    }
}
