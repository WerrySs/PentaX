// SPDX-License-Identifier: GPL-2.0-only

//! Exact pinned Font/FontFt FUNC identities. See FONT-NEXT/provenance.
//! GP arity is independent of separately captured SSE arguments.

use super::{Registration, Service};

const EXPORTS: [(Service, &str, &str, u8, bool, bool); 72] = [
    (
        Service::FontAttachDeviceCacheBuffer,
        "CUKn5pX-NVY",
        "Font::FontAttachDeviceCacheBuffer",
        3,
        false,
        true,
    ),
    (
        Service::FontCloseFont,
        "vzHs3C8lWJk",
        "Font::FontCloseFont",
        1,
        false,
        true,
    ),
    (
        Service::FontCreateRendererWithEdition,
        "WaSFJoRWXaI",
        "Font::FontCreateRendererWithEdition",
        4,
        false,
        true,
    ),
    (
        Service::FontDestroyRenderer,
        "exAxkyVLt0s",
        "Font::FontDestroyRenderer",
        1,
        false,
        true,
    ),
    (
        Service::FontBindRenderer,
        "3OdRkSjOcog",
        "Font::FontBindRenderer",
        2,
        false,
        true,
    ),
    (
        Service::FontRebindRenderer,
        "Z2cdsqJH+5k",
        "Font::FontRebindRenderer",
        1,
        false,
        false,
    ),
    (
        Service::FontUnbindRenderer,
        "1QjhKxrsOB8",
        "Font::FontUnbindRenderer",
        1,
        false,
        true,
    ),
    (
        Service::FontSetScalePixel,
        "N1EBMeGhf7E",
        "Font::FontSetScalePixel",
        1,
        false,
        true,
    ),
    (
        Service::FontSetEffectSlant,
        "TMtqoFQjjbA",
        "Font::FontSetEffectSlant",
        1,
        false,
        true,
    ),
    (
        Service::FontSetEffectWeight,
        "v0phZwa4R5o",
        "Font::FontSetEffectWeight",
        2,
        false,
        true,
    ),
    (
        Service::FontSetupRenderScalePixel,
        "6vGCkkQJOcI",
        "Font::FontSetupRenderScalePixel",
        1,
        false,
        true,
    ),
    (
        Service::FontSetupRenderEffectSlant,
        "lz9y9UFO2UU",
        "Font::FontSetupRenderEffectSlant",
        1,
        false,
        true,
    ),
    (
        Service::FontSetupRenderEffectWeight,
        "XIGorvLusDQ",
        "Font::FontSetupRenderEffectWeight",
        2,
        false,
        true,
    ),
    (
        Service::FontGetHorizontalLayout,
        "imxVx8lm+KM",
        "Font::FontGetHorizontalLayout",
        2,
        false,
        true,
    ),
    (
        Service::FontGetVerticalLayout,
        "3BrWWFU+4ts",
        "Font::FontGetVerticalLayout",
        2,
        false,
        true,
    ),
    (
        Service::FontDefineAttribute,
        "8h-SOB-asgk",
        "Font::FontDefineAttribute",
        3,
        false,
        false,
    ),
    (
        Service::FontCreateLibrary,
        "nWrfPI4Okmg",
        "Font::FontCreateLibrary",
        3,
        false,
        false,
    ),
    (
        Service::FontCreateLibraryWithEdition,
        "n590hj5Oe-k",
        "Font::FontCreateLibraryWithEdition",
        4,
        false,
        true,
    ),
    (
        Service::FontCreateString,
        "MO24vDhmS4E",
        "Font::FontCreateString",
        4,
        false,
        true,
    ),
    (
        Service::FontCreateWritingLine,
        "7rogx92EEyc",
        "Font::FontCreateWritingLine",
        4,
        false,
        true,
    ),
    (
        Service::FontDestroyString,
        "SSCaczu2aMQ",
        "Font::FontDestroyString",
        1,
        false,
        true,
    ),
    (
        Service::FontDestroyWritingLine,
        "PEjv7CVDRYs",
        "Font::FontDestroyWritingLine",
        1,
        false,
        true,
    ),
    (
        Service::FontDeleteGlyph,
        "LHDoRWVFGqk",
        "Font::FontDeleteGlyph",
        2,
        false,
        true,
    ),
    (
        Service::FontDestroyLibrary,
        "FXP359ygujs",
        "Font::FontDestroyLibrary",
        1,
        false,
        false,
    ),
    (
        Service::FontGenerateCharGlyph,
        "C-4Qw5Srlyw",
        "Font::FontGenerateCharGlyph",
        4,
        false,
        true,
    ),
    (
        Service::FontGetCharGlyphMetrics,
        "L97d+3OgMlE",
        "Font::FontGetCharGlyphMetrics",
        3,
        false,
        false,
    ),
    (
        Service::FontGetRenderCharGlyphMetrics,
        "IQtleGLL5pQ",
        "Font::FontGetRenderCharGlyphMetrics",
        3,
        false,
        true,
    ),
    (
        Service::FontGlyphDefineAttribute,
        "8-zmgsxkBek",
        "Font::FontGlyphDefineAttribute",
        3,
        false,
        true,
    ),
    (
        Service::FontMemoryInit,
        "whrS4oksXc4",
        "Font::FontMemoryInit",
        7,
        false,
        true,
    ),
    (
        Service::FontMemoryTerm,
        "h6hIgxXEiEc",
        "Font::FontMemoryTerm",
        1,
        false,
        false,
    ),
    (
        Service::FontOpenFontInstance,
        "JzCH3SCFnAU",
        "Font::FontOpenFontInstance",
        3,
        false,
        true,
    ),
    (
        Service::FontOpenFontMemory,
        "KXUpebrFk1U",
        "Font::FontOpenFontMemory",
        5,
        false,
        true,
    ),
    (
        Service::FontOpenFontSet,
        "cKYtVmeSTcw",
        "Font::FontOpenFontSet",
        5,
        false,
        true,
    ),
    (
        Service::FontRenderCharGlyphImageHorizontal,
        "3G4zhgKuxE8",
        "Font::FontRenderCharGlyphImageHorizontal",
        5,
        false,
        false,
    ),
    (
        Service::FontRenderCharGlyphImageHorizontal,
        "kAenWy1Zw5o",
        "Font::FontRenderCharGlyphImageHorizontal",
        5,
        false,
        true,
    ),
    (
        Service::FontRenderCharGlyphImageHorizontal,
        "i6UNdSig1uE",
        "Font::FontRenderCharGlyphImageHorizontal",
        5,
        false,
        false,
    ),
    (
        Service::FontRenderSurfaceInit,
        "gdUCnU0gHdI",
        "Font::FontRenderSurfaceInit",
        6,
        false,
        true,
    ),
    (
        Service::FontRenderSurfaceSetScissor,
        "vRxf4d0ulPs",
        "Font::FontRenderSurfaceSetScissor",
        5,
        false,
        false,
    ),
    (
        Service::FontStringGetTerminateCode,
        "ObkDGDBsVtw",
        "Font::FontStringGetTerminateCode",
        1,
        false,
        true,
    ),
    (
        Service::FontStringGetTerminateOrder,
        "+B-xlbiWDJ4",
        "Font::FontStringGetTerminateOrder",
        1,
        false,
        false,
    ),
    (
        Service::FontStringGetWritingForm,
        "o1vIEHeb6tw",
        "Font::FontStringGetWritingForm",
        1,
        false,
        true,
    ),
    (
        Service::FontStringRefersRenderCharacters,
        "hq5LffQjz-s",
        "Font::FontStringRefersRenderCharacters",
        4,
        false,
        true,
    ),
    (
        Service::FontStringRefersTextCharacters,
        "Avv7OApgCJk",
        "Font::FontStringRefersTextCharacters",
        2,
        false,
        true,
    ),
    (
        Service::FontTextSourceInit,
        "oaJ1BpN2FQk",
        "Font::FontTextSourceInit",
        5,
        false,
        true,
    ),
    (
        Service::FontTextSourceRewind,
        "VRFd3diReec",
        "Font::FontTextSourceRewind",
        1,
        false,
        false,
    ),
    (
        Service::FontTextSourceSetDefaultFont,
        "eCRMCSk96NU",
        "Font::FontTextSourceSetDefaultFont",
        2,
        false,
        true,
    ),
    (
        Service::FontTextSourceSetWritingForm,
        "OqQKX0h5COw",
        "Font::FontTextSourceSetWritingForm",
        2,
        false,
        true,
    ),
    (
        Service::FontCharacterGetBidiLevel,
        "6DFUkCwQLa8",
        "Font::FontCharacterGetBidiLevel",
        2,
        false,
        true,
    ),
    (
        Service::FontCharacterGetSyllableStringState,
        "coCrV6IWplE",
        "Font::FontCharacterGetSyllableStringState",
        2,
        false,
        false,
    ),
    (
        Service::FontCharacterGetTextFontCode,
        "zN3+nuA0SFQ",
        "Font::FontCharacterGetTextFontCode",
        3,
        false,
        true,
    ),
    (
        Service::FontCharacterGetTextOrder,
        "mxgmMj-Mq-o",
        "Font::FontCharacterGetTextOrder",
        2,
        false,
        true,
    ),
    (
        Service::FontCharacterLooksFormatCharacters,
        "-P6X35Rq2-E",
        "Font::FontCharacterLooksFormatCharacters",
        1,
        false,
        false,
    ),
    (
        Service::FontCharacterLooksWhiteSpace,
        "SaRlqtqaCew",
        "Font::FontCharacterLooksWhiteSpace",
        1,
        false,
        true,
    ),
    (
        Service::FontCharacterRefersTextBack,
        "6Gqlv5KdTbU",
        "Font::FontCharacterRefersTextBack",
        1,
        false,
        false,
    ),
    (
        Service::FontCharacterRefersTextNext,
        "BkjBP+YC19w",
        "Font::FontCharacterRefersTextNext",
        1,
        false,
        true,
    ),
    (
        Service::FontCharactersRefersTextCodes,
        "lVSR5ftvNag",
        "Font::FontCharactersRefersTextCodes",
        3,
        false,
        false,
    ),
    (
        Service::FontTextCodesStepBack,
        "IPoYwwlMx-g",
        "Font::FontTextCodesStepBack",
        1,
        false,
        false,
    ),
    (
        Service::FontTextCodesStepNext,
        "olSmXY+XP1E",
        "Font::FontTextCodesStepNext",
        1,
        false,
        false,
    ),
    (
        Service::FontWritingGetRenderMetrics,
        "fljdejMcG1c",
        "Font::FontWritingGetRenderMetrics",
        2,
        false,
        true,
    ),
    (
        Service::FontWritingInit,
        "fD5rqhEXKYQ",
        "Font::FontWritingInit",
        3,
        false,
        true,
    ),
    (
        Service::FontWritingLineClear,
        "1+DgKL0haWQ",
        "Font::FontWritingLineClear",
        1,
        false,
        true,
    ),
    (
        Service::FontWritingLineGetOrderingSpace,
        "JQKWIsS9joE",
        "Font::FontWritingLineGetOrderingSpace",
        5,
        false,
        true,
    ),
    (
        Service::FontWritingLineGetRenderMetrics,
        "nlU2VnfpqTM",
        "Font::FontWritingLineGetRenderMetrics",
        2,
        false,
        true,
    ),
    (
        Service::FontWritingLineRefersRenderStep,
        "+FYcYefsVX0",
        "Font::FontWritingLineRefersRenderStep",
        1,
        false,
        true,
    ),
    (
        Service::FontWritingLineWritesOrder,
        "wyKFUOWdu3Q",
        "Font::FontWritingLineWritesOrder",
        4,
        false,
        true,
    ),
    (
        Service::FontWritingRefersRenderStep,
        "W-2WOXEHGck",
        "Font::FontWritingRefersRenderStep",
        1,
        false,
        true,
    ),
    (
        Service::FontWritingRefersRenderStepCharacter,
        "f4Onl7efPEY",
        "Font::FontWritingRefersRenderStepCharacter",
        2,
        false,
        true,
    ),
    (
        Service::FontWritingSetMaskInvisible,
        "BbCZjJizU4A",
        "Font::FontWritingSetMaskInvisible",
        2,
        false,
        false,
    ),
    (
        Service::FontSupportExternalFonts,
        "mz2iTY0MK4A",
        "Font::FontSupportExternalFonts",
        3,
        false,
        true,
    ),
    (
        Service::FontSupportSystemFonts,
        "SsRbbCiWoGw",
        "Font::FontSupportSystemFonts",
        1,
        false,
        true,
    ),
    (
        Service::FontSelectLibraryFt,
        "oM+XCzVG3oM",
        "FontFt::FontSelectLibraryFt",
        1,
        true,
        true,
    ),
    (
        Service::FontSelectRendererFt,
        "Xx974EW-QFY",
        "FontFt::FontSelectRendererFt",
        1,
        true,
        true,
    ),
];

pub(super) const FONT_REGISTRATIONS: [Registration; 126] = {
    let base = Registration {
        service: Service::FontAttachDeviceCacheBuffer,
        nid: "CUKn5pX-NVY",
        source_name: "Font::FontAttachDeviceCacheBuffer",
        arguments: 3,
        module: "Font",
        module_version: (1, 1),
        library: "Font",
        library_version: 1,
    };
    let mut entries = [base; 126];
    let mut index = 0;
    while index < EXPORTS.len() {
        let (service, nid, source_name, arguments, ft, _) = EXPORTS[index];
        let scope = if ft { "FontFt" } else { "Font" };
        entries[index] = Registration {
            service,
            nid,
            source_name,
            arguments,
            module: scope,
            library: scope,
            ..base
        };
        index += 1;
    }
    let mut output = EXPORTS.len();
    index = 0;
    while index < EXPORTS.len() {
        let (_, _, _, _, ft, observed) = EXPORTS[index];
        if observed {
            let scope = if ft { "libSceFontFt" } else { "libSceFont" };
            entries[output] = Registration {
                module: scope,
                library: scope,
                ..entries[index]
            };
            output += 1;
        }
        index += 1;
    }
    assert!(output == entries.len());
    entries
};

#[cfg(test)]
mod tests {
    use super::super::{REGISTRATIONS, resolve};
    use super::*;

    const OBSERVED: [usize; 54] = [
        0, 1, 2, 3, 4, 6, 7, 8, 9, 10, 11, 12, 13, 14, 17, 18, 19, 20, 21, 22, 24, 26, 27, 28, 30,
        31, 32, 34, 36, 38, 40, 41, 42, 43, 45, 46, 47, 49, 50, 52, 54, 58, 59, 60, 61, 62, 63, 64,
        65, 66, 68, 69, 70, 71,
    ];

    #[test]
    fn font_catalogue_preserves_gp_classes_and_exact_per_nid_aliases() {
        assert_eq!(REGISTRATIONS.len(), 1281);
        assert_eq!(
            Service::FontAttachDeviceCacheBuffer as usize,
            Service::EntitlementAccessGetEntitlementKey as usize + 1
        );
        assert_eq!(
            REGISTRATIONS[1056].service,
            Service::EntitlementAccessGetAddcontEntitlementInfoList
        );
        for (nid, service, gp) in [
            ("whrS4oksXc4", Service::FontMemoryInit, 7),
            ("N1EBMeGhf7E", Service::FontSetScalePixel, 1),
            ("v0phZwa4R5o", Service::FontSetEffectWeight, 2),
            ("gdUCnU0gHdI", Service::FontRenderSurfaceInit, 6),
            (
                "kAenWy1Zw5o",
                Service::FontRenderCharGlyphImageHorizontal,
                5,
            ),
            ("oM+XCzVG3oM", Service::FontSelectLibraryFt, 1),
            ("Xx974EW-QFY", Service::FontSelectRendererFt, 1),
        ] {
            let row = FONT_REGISTRATIONS
                .iter()
                .find(|row| row.nid == nid)
                .unwrap();
            assert_eq!((row.service, row.arguments), (service, gp));
        }
        for (index, row) in FONT_REGISTRATIONS[..72].iter().enumerate() {
            assert_eq!(REGISTRATIONS[1057 + index], *row);
            assert_eq!((row.module_version, row.library_version), ((1, 1), 1));
            let scope = if index >= 70 { "FontFt" } else { "Font" };
            let prefixed = if index >= 70 {
                "libSceFontFt"
            } else {
                "libSceFont"
            };
            assert_eq!((row.module, row.library), (scope, scope));
            assert_eq!(resolve(scope, (1, 1), scope, 1, row.nid), Some(row.service));
            let observed = OBSERVED.iter().position(|candidate| *candidate == index);
            assert_eq!(
                resolve(prefixed, (1, 1), prefixed, 1, row.nid),
                observed.map(|_| row.service)
            );
            if let Some(alias) = observed {
                assert_eq!(
                    FONT_REGISTRATIONS[72 + alias],
                    Registration {
                        module: prefixed,
                        library: prefixed,
                        ..*row
                    }
                );
            }
        }
        for nid in ["3G4zhgKuxE8", "kAenWy1Zw5o", "i6UNdSig1uE"] {
            assert_eq!(
                resolve("Font", (1, 1), "Font", 1, nid),
                Some(Service::FontRenderCharGlyphImageHorizontal)
            );
            assert_eq!(
                resolve("libSceFont", (1, 1), "libSceFont", 1, nid),
                (nid == "kAenWy1Zw5o").then_some(Service::FontRenderCharGlyphImageHorizontal)
            );
        }
    }

    #[test]
    fn font_catalogue_refuses_mixed_names_and_wrong_versions() {
        for row in &FONT_REGISTRATIONS[..72] {
            let prefixed = if row.module == "FontFt" {
                "libSceFontFt"
            } else {
                "libSceFont"
            };
            for (module, library) in [(row.module, prefixed), (prefixed, row.library)] {
                assert_eq!(resolve(module, (1, 1), library, 1, row.nid), None);
            }
            for (module, library) in [(row.module, row.library), (prefixed, prefixed)] {
                for (module_version, library_version) in [((2, 1), 1), ((1, 2), 1), ((1, 1), 2)] {
                    assert_eq!(
                        resolve(module, module_version, library, library_version, row.nid),
                        None
                    );
                }
            }
            assert_eq!(resolve("Font", (1, 1), "FontFt", 1, row.nid), None);
        }
    }
}
