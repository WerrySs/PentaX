// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SharpEmu Emulator Project
// Public Gen5 Nid/ExportName/LibraryName attributes adapted from
// sharpemu/sharpemu b0fa905fa10682107813dd4c53c48a18005859d6,
// src/SharpEmu.Libs/Network/Http2Exports.cs, lines 202-205, 280-283, 293-296.
// Modified 2026 for PentaX: three static library-bound diagnostic references.
// No implementation, module identity, version, provider or ABI is inferred.

pub(super) const COUNT: usize = REFERENCES.len();

const REFERENCES: &[(&str, &str, &str)] = &[
    ("libSceHttp2", "09tk+kIA1Ns", "sceHttp2SetMinSslVersion"),
    ("libSceHttp2", "N4UfjvWJsMw", "sceHttp2CreateCookieBox"),
    ("libSceHttp2", "jrVHsKCXA0g", "sceHttp2SetCookieBox"),
];

pub(super) fn lookup(library: &[u8], nid: &[u8]) -> Option<&'static str> {
    REFERENCES
        .iter()
        .find_map(|&(known_library, known_nid, name)| {
            (known_library.as_bytes() == library && known_nid.as_bytes() == nid).then_some(name)
        })
}
