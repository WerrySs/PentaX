// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn base_hardware_queries_preserve_source_alias_order_exact_words_and_fault_atomicity() {
    let mut k = kernel(Default::default());
    let mut g = TestGuest::new();
    assert_eq!(
        call(&mut k, &mut g, Service::IsTrinityMode, &[u64::MAX; 6]),
        Ok(0)
    );
    assert_eq!(g.effects, 0);
    for (mode, submode) in [(0, 0), (0, 2), (2, 0), (0, 4), (4, 0), (0, 32)] {
        g.put(0, &[0xa5; 40]);
        let mut expected = [0xa5; 40];
        expected[mode..mode + 4].copy_from_slice(&2_u32.to_le_bytes());
        expected[submode..submode + 4].fill(0);
        assert_eq!(
            call(
                &mut k,
                &mut g,
                Service::GetOperationMode,
                &[ARG + mode as u64, ARG + submode as u64]
            ),
            Ok(0)
        );
        assert_eq!(g.get(ARG, 40), expected);
    }
    for args in [[ARG, 0], [0, ARG], [ARG, u64::MAX], [ARG, ARG + PAGE - 2]] {
        g.put(0, &[0xa5; 8]);
        assert!(call(&mut k, &mut g, Service::GetOperationMode, &args).is_err());
        assert_eq!(g.get(ARG, 8), [0xa5; 8]);
    }
    let mut limited = kernel(Limits {
        max_work: 0,
        ..Default::default()
    });
    assert!(call(&mut limited, &mut g, Service::IsTrinityMode, &[]).is_err());
    assert!(
        call(
            &mut limited,
            &mut g,
            Service::GetOperationMode,
            &[ARG, ARG + 4]
        )
        .is_err()
    );
    assert_eq!(g.get(ARG, 8), [0xa5; 8]);
}
