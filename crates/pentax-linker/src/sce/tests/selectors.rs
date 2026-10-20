// SPDX-License-Identifier: GPL-2.0-or-later

use super::NID;
use crate::{
    MetadataName, NidToken, QualifiedField, QualifiedNameError, QualifiedNid, SELECTOR_ALPHABET,
    SelectorId, TokenError,
};

fn field(field: QualifiedField, error: TokenError) -> Result<QualifiedNid, QualifiedNameError> {
    Err(QualifiedNameError::Field { field, error })
}

#[test]
fn selector_boundaries_encode_canonically() {
    for (value, text) in [
        (0, "A"),
        (1, "B"),
        (25, "Z"),
        (26, "a"),
        (52, "0"),
        (62, "+"),
        (63, "-"),
        (64, "BA"),
        (4095, "--"),
        (4096, "BAA"),
        (65535, "P--"),
    ] {
        let id = SelectorId::new(value);
        assert_eq!(id.encode().as_bytes(), text.as_bytes(), "{value}");
        assert_eq!(id.to_string(), text);
        assert_eq!(SelectorId::parse(text.as_bytes()), Ok(id));
    }
}

#[test]
fn every_selector_round_trips() {
    for value in 0..=u16::MAX {
        let id = SelectorId::new(value);
        let text = id.encode();
        assert_eq!(SelectorId::parse(text.as_bytes()), Ok(id));
    }
}

#[test]
fn non_canonical_selectors_are_rejected() {
    for text in ["AA", "AB", "AAA", "A-", "ABA"] {
        assert_eq!(
            SelectorId::parse(text.as_bytes()),
            Err(TokenError::LeadingZero),
            "{text}"
        );
    }
    assert_eq!(SelectorId::parse(b"QAA"), Err(TokenError::Overflow));
    assert_eq!(SelectorId::parse(b"---"), Err(TokenError::Overflow));
    assert_eq!(
        SelectorId::parse(b"BAAA"),
        Err(TokenError::InvalidLength { length: 4 })
    );
    assert_eq!(SelectorId::parse(b""), Err(TokenError::Empty));
    for byte in [b'/', b'=', b' ', b'\t', b'\n', b'#', b'_', 0, 0xff] {
        assert_eq!(
            SelectorId::parse(&[b'B', byte]),
            Err(TokenError::InvalidByte { position: 1, byte })
        );
    }
}

#[test]
fn nid_tokens_require_eleven_digits_and_zero_padding() {
    let valid_finals = b"AEIMQUYcgkosw048";
    for &last in SELECTOR_ALPHABET {
        let mut token = *b"pentaxNid0A";
        token[10] = last;
        let parsed = NidToken::parse(&token);
        if valid_finals.contains(&last) {
            assert_eq!(parsed.map(|token| *token.as_bytes()), Ok(token));
        } else {
            assert_eq!(parsed, Err(TokenError::NonzeroPadding { byte: last }));
        }
    }
    assert_eq!(
        NidToken::parse(b"pentaxNid0"),
        Err(TokenError::InvalidLength { length: 10 })
    );
    assert_eq!(
        NidToken::parse(b"pentaxNid0AA"),
        Err(TokenError::InvalidLength { length: 12 })
    );
    assert_eq!(NidToken::parse(b""), Err(TokenError::Empty));
    assert_eq!(
        NidToken::parse(b"pen/axNid0A"),
        Err(TokenError::InvalidByte {
            position: 3,
            byte: b'/'
        })
    );
    assert_eq!(
        NidToken::parse(b"pentaxNid0="),
        Err(TokenError::InvalidByte {
            position: 10,
            byte: b'='
        })
    );
    let token = NidToken::parse(NID).unwrap();
    assert_eq!(token.to_string(), "3T1rzimi0Ew");
    assert_eq!(format!("{token:?}"), "NidToken(3T1rzimi0Ew)");
}

#[test]
fn qualified_names_parse_and_encode_canonically() {
    let name = QualifiedNid::parse(b"3T1rzimi0Ew#BA#P--").unwrap();
    assert_eq!(name.nid().as_bytes(), NID);
    assert_eq!(name.library(), SelectorId::new(64));
    assert_eq!(name.module(), SelectorId::new(65535));
    assert_eq!(name.to_bytes(), b"3T1rzimi0Ew#BA#P--");
    assert_eq!(name.to_string(), "3T1rzimi0Ew#BA#P--");
    let built = QualifiedNid::new(
        NidToken::parse(NID).unwrap(),
        SelectorId::new(0),
        SelectorId::new(4096),
    );
    assert_eq!(QualifiedNid::parse(&built.to_bytes()), Ok(built));
    assert_eq!(built.to_bytes(), b"3T1rzimi0Ew#A#BAA");
}

#[test]
fn malformed_qualified_names_are_structured_errors() {
    let delimiters = |count| Err(QualifiedNameError::DelimiterCount { count });
    // Ordinary names are not guessed at or truncated.
    assert_eq!(QualifiedNid::parse(b"puts"), delimiters(0));
    assert_eq!(QualifiedNid::parse(NID), delimiters(0));
    assert_eq!(QualifiedNid::parse(b"3T1rzimi0Ew#A"), delimiters(1));
    assert_eq!(QualifiedNid::parse(b"3T1rzimi0Ew#A#B#C"), delimiters(3));
    assert_eq!(QualifiedNid::parse(b"3T1rzimi0Ew#A#B#"), delimiters(3));
    assert_eq!(
        QualifiedNid::parse(b"##"),
        field(QualifiedField::Nid, TokenError::Empty)
    );
    assert_eq!(
        QualifiedNid::parse(b"3T1rzimi0Ew##B"),
        field(QualifiedField::Library, TokenError::Empty)
    );
    assert_eq!(
        QualifiedNid::parse(b"3T1rzimi0Ew#A#"),
        field(QualifiedField::Module, TokenError::Empty)
    );
    assert_eq!(
        QualifiedNid::parse(b"3T1rzimi0Ex#A#B"),
        field(
            QualifiedField::Nid,
            TokenError::NonzeroPadding { byte: b'x' }
        )
    );
    assert_eq!(
        QualifiedNid::parse(b" 3T1rzimi0Ew#A#B"),
        field(
            QualifiedField::Nid,
            TokenError::InvalidLength { length: 12 }
        )
    );
    assert_eq!(
        QualifiedNid::parse(b"3T1rzimi0E #A#B"),
        field(
            QualifiedField::Nid,
            TokenError::InvalidByte {
                position: 10,
                byte: b' '
            }
        )
    );
    assert_eq!(
        QualifiedNid::parse(b"3T1rzimi0Ew#A=#B"),
        field(
            QualifiedField::Library,
            TokenError::InvalidByte {
                position: 1,
                byte: b'='
            }
        )
    );
    assert_eq!(
        QualifiedNid::parse(b"3T1rzimi0Ew#A#B\n"),
        field(
            QualifiedField::Module,
            TokenError::InvalidByte {
                position: 1,
                byte: b'\n'
            }
        )
    );
    assert_eq!(
        QualifiedNid::parse(b"3T1rzimi0Ew#AB#B"),
        field(QualifiedField::Library, TokenError::LeadingZero)
    );
    assert_eq!(
        QualifiedNid::parse(b"3T1rzimi0Ew#A#QAA"),
        field(QualifiedField::Module, TokenError::Overflow)
    );
    assert_eq!(
        QualifiedNid::parse(b"3T1rzimi0Ew#A#BAAA"),
        field(
            QualifiedField::Module,
            TokenError::InvalidLength { length: 4 }
        )
    );
}

#[test]
fn diagnostics_escape_raw_bytes() {
    let error = TokenError::InvalidByte {
        position: 0,
        byte: 0xff,
    };
    assert_eq!(
        error.to_string(),
        r"byte b'\xff' at position 0 is outside the selector alphabet"
    );
    let qualified = QualifiedNameError::Field {
        field: QualifiedField::Module,
        error,
    };
    assert!(
        qualified
            .to_string()
            .starts_with("qualified module selector: ")
    );
    let name = MetadataName::new(b"lib\xff\n");
    assert_eq!(format!("{name:?}"), r#"b"lib\xff\n""#);
    assert_eq!(name.to_string(), r"lib\xff\n");
}
