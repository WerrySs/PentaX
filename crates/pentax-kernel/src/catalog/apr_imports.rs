// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: explicit APR resolver/query registrations.
// Adapted from pinned libAmpr.cpp; see NOTICE.

use super::{BASE_REGISTRATIONS, Registration, Service};

// Appended after all existing registrations: previously issued provider symbol
// ordinals remain unchanged. No wildcard scopes or unobserved SDK aliases.
pub(super) const APR_IMPORT_REGISTRATIONS: [Registration; 9] = entries();

const fn entries() -> [Registration; 9] {
    let source = [
        (
            Service::AprResolveFilepathsToIds,
            "WT-5NKy42fw",
            "Apr::ResolveFilepathsToIds",
            4,
        ),
        (
            Service::AprGetFileStat,
            "ApkYaHb8Sek",
            "Apr::GetFileStat",
            2,
        ),
        (
            Service::AprGetFileSize,
            "WvEu7yl3Ivg",
            "Apr::GetFileSize",
            2,
        ),
        (
            Service::AprResolveFilepathsWithPrefixToIds,
            "i3HWvW35jao",
            "Apr::ResolveFilepathsWithPrefixToIds",
            5,
        ),
        (
            Service::AprResolveFilepathsWithPrefixToIdsAndFileSizes,
            "w5fcCG+t31g",
            "Apr::ResolveFilepathsWithPrefixToIdsAndFileSizes",
            6,
        ),
        (
            Service::AprResolveFilepathsToIdsForEach,
            "eYAh2vlCY-U",
            "Apr::ResolveFilepathsToIdsForEach",
            4,
        ),
        (
            Service::AprResolveFilepathsToIdsAndFileSizesForEach,
            "QzB4O+bJQyA",
            "Apr::ResolveFilepathsToIdsAndFileSizesForEach",
            5,
        ),
        (
            Service::AprResolveFilepathsWithPrefixToIdsForEach,
            "VB-BtuIW8Xc",
            "Apr::ResolveFilepathsWithPrefixToIdsForEach",
            5,
        ),
        (
            Service::AprResolveFilepathsWithPrefixToIdsAndFileSizesForEach,
            "C+Khtbbx2g8",
            "Apr::ResolveFilepathsWithPrefixToIdsAndFileSizesForEach",
            6,
        ),
    ];
    let mut result = [BASE_REGISTRATIONS[0]; 9];
    let mut index = 0;
    while index < source.len() {
        let (service, nid, source_name, arguments) = source[index];
        result[index] = Registration {
            service,
            nid,
            source_name,
            arguments,
            ..BASE_REGISTRATIONS[0]
        };
        index += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nine_apr_imports_have_exact_scopes_arities_and_appended_provider_ordinals() {
        let expected = [
            ("WT-5NKy42fw", Service::AprResolveFilepathsToIds, 4),
            ("ApkYaHb8Sek", Service::AprGetFileStat, 2),
            ("WvEu7yl3Ivg", Service::AprGetFileSize, 2),
            (
                "i3HWvW35jao",
                Service::AprResolveFilepathsWithPrefixToIds,
                5,
            ),
            (
                "w5fcCG+t31g",
                Service::AprResolveFilepathsWithPrefixToIdsAndFileSizes,
                6,
            ),
            ("eYAh2vlCY-U", Service::AprResolveFilepathsToIdsForEach, 4),
            (
                "QzB4O+bJQyA",
                Service::AprResolveFilepathsToIdsAndFileSizesForEach,
                5,
            ),
            (
                "VB-BtuIW8Xc",
                Service::AprResolveFilepathsWithPrefixToIdsForEach,
                5,
            ),
            (
                "C+Khtbbx2g8",
                Service::AprResolveFilepathsWithPrefixToIdsAndFileSizesForEach,
                6,
            ),
        ];
        for (offset, (nid, service, arity)) in expected.into_iter().enumerate() {
            let entry = &super::super::REGISTRATIONS[1381 + offset];
            assert_eq!(
                (entry.nid, entry.service, entry.arguments),
                (nid, service, arity)
            );
            assert_eq!(
                (
                    entry.module,
                    entry.module_version,
                    entry.library,
                    entry.library_version
                ),
                ("libkernel", (1, 1), "libkernel", 1)
            );
            assert_eq!(
                crate::resolve("libkernel", (1, 1), "libkernel", 1, nid),
                Some(service)
            );
            assert_eq!(
                crate::resolve("libkernel", (1, 1), "libScePosix", 1, nid),
                None
            );
        }
        assert_eq!(super::super::REGISTRATIONS.len(), 1402);
    }
}
