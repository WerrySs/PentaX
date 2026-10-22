// SPDX-License-Identifier: GPL-2.0-only
// Public KytyPS5@f186b35706ddbc3faf5513f326e2335d224802f7 libAudio.cpp,
// blob2069a82b636778b652d4d6f936a57c3e8dd040ed: AudioOut1.1/AudioOut2v1.

use super::{Registration, Service};

const EXPORTS: [(Service, &str, &str, u8); 4] = [
    (
        Service::AudioOut2Initialize,
        "g2tViFIohHE",
        "AudioOut2::AudioOut2Initialize",
        0,
    ),
    (
        Service::AudioOut2ContextResetParam,
        "t5YrizufpQc",
        "AudioOut2::AudioOut2ContextResetParam",
        1,
    ),
    (
        Service::AudioOut2ContextQueryMemory,
        "pDmme7Bgm6E",
        "AudioOut2::AudioOut2ContextQueryMemory",
        2,
    ),
    (
        Service::AudioOut2GetSpeakerInfo,
        "DImz2Ft9E2g",
        "AudioOut2::AudioOut2GetSpeakerInfo",
        2,
    ),
];
pub(super) const AUDIO_OUT2_REGISTRATIONS: [Registration; 8] = {
    let base = Registration {
        service: Service::AudioOut2Initialize,
        nid: "g2tViFIohHE",
        source_name: "AudioOut2::AudioOut2Initialize",
        arguments: 0,
        module: "AudioOut",
        module_version: (1, 1),
        library: "AudioOut2",
        library_version: 1,
    };
    let mut rows = [base; 8];
    let mut i = 0;
    while i < EXPORTS.len() {
        let (service, nid, source_name, arguments) = EXPORTS[i];
        rows[i] = Registration {
            service,
            nid,
            source_name,
            arguments,
            ..base
        };
        rows[i + 4] = Registration {
            module: "libSceAudioOut",
            library: "libSceAudioOut2",
            ..rows[i]
        };
        i += 1;
    }
    rows
};

const LIFECYCLE: [(Service, &str, &str, u8); 11] = [
    (
        Service::AudioOut2ContextCreate,
        "0x6o1VVAYSY",
        "AudioOut2::AudioOut2ContextCreate",
        4,
    ),
    (
        Service::AudioOut2ContextDestroy,
        "on6ZH7Abo10",
        "AudioOut2::AudioOut2ContextDestroy",
        1,
    ),
    (
        Service::AudioOut2ContextAdvance,
        "PE2zHMqLSHs",
        "AudioOut2::AudioOut2ContextAdvance",
        1,
    ),
    (
        Service::AudioOut2ContextPush,
        "aII9h5nli9U",
        "AudioOut2::AudioOut2ContextPush",
        2,
    ),
    (
        Service::AudioOut2ContextGetQueueLevel,
        "R7d0F1g2qsU",
        "AudioOut2::AudioOut2ContextGetQueueLevel",
        3,
    ),
    (
        Service::AudioOut2UserCreate,
        "xywYcRB7nbQ",
        "AudioOut2::AudioOut2UserCreate",
        2,
    ),
    (
        Service::AudioOut2UserDestroy,
        "IaZXJ9M79uo",
        "AudioOut2::AudioOut2UserDestroy",
        1,
    ),
    (
        Service::AudioOut2PortCreate,
        "JK2wamZPzwM",
        "AudioOut2::AudioOut2PortCreate",
        3,
    ),
    (
        Service::AudioOut2PortDestroy,
        "cd+Rtw+D1x8",
        "AudioOut2::AudioOut2PortDestroy",
        1,
    ),
    (
        Service::AudioOut2PortGetState,
        "gatEUKG+Ea4",
        "AudioOut2::AudioOut2PortGetState",
        2,
    ),
    (
        Service::AudioOut2PortSetAttributes,
        "8XTArSPyWHk",
        "AudioOut2::AudioOut2PortSetAttributes",
        3,
    ),
];
pub(super) const AUDIO_OUT2_LIFECYCLE_REGISTRATIONS: [Registration; 22] = {
    let mut rows = [AUDIO_OUT2_REGISTRATIONS[0]; 22];
    let mut i = 0;
    while i < LIFECYCLE.len() {
        let (service, nid, source_name, arguments) = LIFECYCLE[i];
        rows[i] = Registration {
            service,
            nid,
            source_name,
            arguments,
            ..AUDIO_OUT2_REGISTRATIONS[0]
        };
        rows[i + 11] = Registration {
            module: "libSceAudioOut",
            library: "libSceAudioOut2",
            ..rows[i]
        };
        i += 1;
    }
    rows
};

#[cfg(test)]
mod tests {
    use super::super::{REGISTRATIONS, resolve};
    use super::*;

    #[test]
    fn audio_out2_appends_only_four_stateless_roles_under_two_exact_identities() {
        assert_eq!(REGISTRATIONS.len(), 1381);
        assert_eq!(
            Service::AudioOut2Initialize as usize,
            Service::AcmContextCreate as usize + 1
        );
        assert_eq!(&REGISTRATIONS[1283..1291], &AUDIO_OUT2_REGISTRATIONS);
        for (scope, module, library) in [
            (0, "AudioOut", "AudioOut2"),
            (4, "libSceAudioOut", "libSceAudioOut2"),
        ] {
            for (i, (service, nid, label, gp)) in EXPORTS.into_iter().enumerate() {
                let row = REGISTRATIONS[1283 + scope + i];
                assert_eq!(
                    (row.service, row.nid, row.source_name, row.arguments),
                    (service, nid, label, gp)
                );
                assert_eq!(
                    (
                        row.module,
                        row.module_version,
                        row.library,
                        row.library_version
                    ),
                    (module, (1, 1), library, 1)
                );
                assert_eq!(resolve(module, (1, 1), library, 1, nid), Some(service));
            }
        }
        for (_, nid, _, _) in EXPORTS {
            for (module, library, major, minor, version) in [
                ("AudioOut", "libSceAudioOut2", 1, 1, 1),
                ("libSceAudioOut", "AudioOut2", 1, 1, 1),
                ("AudioOut2", "AudioOut2", 1, 1, 1),
                ("AudioOut", "AudioOut2", 1, 0, 1),
                ("AudioOut", "AudioOut2", 2, 1, 1),
                ("AudioOut", "AudioOut2", 1, 1, 2),
            ] {
                assert_eq!(resolve(module, (major, minor), library, version, nid), None);
            }
        }
    }

    #[test]
    fn headless_lifecycle_appends_only_eleven_proven_roles_and_keeps_old_ordinals() {
        assert_eq!(
            &REGISTRATIONS[1291..1313],
            &AUDIO_OUT2_LIFECYCLE_REGISTRATIONS
        );
        assert_eq!(
            Service::AudioOut2ContextCreate as usize,
            Service::AudioOut2GetSpeakerInfo as usize + 1
        );
        for (i, (service, nid, label, gp)) in LIFECYCLE.into_iter().enumerate() {
            for (offset, module, library) in [
                (0, "AudioOut", "AudioOut2"),
                (11, "libSceAudioOut", "libSceAudioOut2"),
            ] {
                let r = REGISTRATIONS[1291 + offset + i];
                assert_eq!(
                    (r.service, r.nid, r.source_name, r.arguments),
                    (service, nid, label, gp)
                );
                assert_eq!(resolve(module, (1, 1), library, 1, nid), Some(service));
                assert_eq!(resolve(module, (1, 0), library, 1, nid), None);
                assert_eq!(resolve(module, (1, 1), library, 2, nid), None);
            }
            assert_eq!(resolve("AudioOut", (1, 1), "libSceAudioOut2", 1, nid), None);
            assert_eq!(resolve("libSceAudioOut", (1, 1), "AudioOut2", 1, nid), None);
        }
        assert_eq!(
            resolve("AudioOut", (1, 1), "AudioOut2", 1, "AudioOut2ContextCreate"),
            None
        );
    }
}
