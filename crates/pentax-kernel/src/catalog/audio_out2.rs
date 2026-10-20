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

#[cfg(test)]
mod tests {
    use super::super::{REGISTRATIONS, resolve};
    use super::*;

    #[test]
    fn audio_out2_appends_only_four_stateless_roles_under_two_exact_identities() {
        assert_eq!(REGISTRATIONS.len(), 1291);
        assert_eq!(
            Service::AudioOut2Initialize as usize,
            Service::AcmContextCreate as usize + 1
        );
        assert_eq!(&REGISTRATIONS[1283..], &AUDIO_OUT2_REGISTRATIONS);
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
        assert_eq!(
            resolve("AudioOut", (1, 1), "AudioOut2", 1, "0x6o1VVAYSY"),
            None
        );
    }
}
