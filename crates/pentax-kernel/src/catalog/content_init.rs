// SPDX-License-Identifier: GPL-2.0-only
//! Exact content-initialization identities from pinned libs.cpp.
use super::{Registration, Service};

pub(super) const CONTENT_INIT_REGISTRATIONS: [Registration; 4] = {
    let base = Registration {
        service: Service::ContentExportInit2,
        nid: "0GnN4QCgIfs",
        source_name: "ContentExport::ContentExportInit2",
        arguments: 1,
        module: "ContentExport",
        module_version: (1, 1),
        library: "ContentExport",
        library_version: 1,
    };
    [
        base,
        Registration {
            service: Service::ContentSearchInit,
            nid: "dPj4ZtRcIWk",
            source_name: "ContentSearch::ContentSearchInit",
            module: "ContentSearch",
            module_version: (1, 0),
            library: "ContentSearch",
            ..base
        },
        Registration {
            service: Service::ContentDeleteInitialize,
            nid: "zoxb0wEChEM",
            source_name: "ContentDelete::ContentDeleteInitialize",
            module: "ContentDelete",
            library: "ContentDelete",
            ..base
        },
        Registration {
            module: "libSceContentExport",
            library: "libSceContentExport",
            ..base
        },
    ]
};

#[cfg(test)]
mod tests {
    use super::super::{REGISTRATIONS, resolve};
    use super::*;

    #[test]
    fn content_catalogue_keeps_search_minor_zero_and_only_observed_export_alias() {
        assert_eq!(REGISTRATIONS.len(), 1283);
        assert_eq!(
            Service::ContentExportInit2 as usize,
            Service::ShareTerminate as usize + 1
        );
        for (index, row) in CONTENT_INIT_REGISTRATIONS.iter().enumerate() {
            assert_eq!(REGISTRATIONS[1216 + index].service, row.service);
            assert_eq!(row.arguments, 1);
            assert_eq!(
                resolve(row.module, row.module_version, row.library, 1, row.nid),
                Some(row.service)
            );
            for version in [(2, 1), (1, 2), if index == 1 { (1, 1) } else { (1, 0) }] {
                assert_eq!(resolve(row.module, version, row.library, 1, row.nid), None);
            }
            assert_eq!(
                resolve(row.module, row.module_version, row.library, 2, row.nid),
                None
            );
        }
        for (index, prefix) in [(1, "libSceContentSearch"), (2, "libSceContentDelete")] {
            let row = CONTENT_INIT_REGISTRATIONS[index];
            assert_eq!(
                resolve(prefix, row.module_version, prefix, 1, row.nid),
                None
            );
        }
        for (module, library) in [
            ("ContentExport", "libSceContentExport"),
            ("libSceContentExport", "ContentExport"),
        ] {
            assert_eq!(resolve(module, (1, 1), library, 1, "0GnN4QCgIfs"), None);
        }
    }
}
