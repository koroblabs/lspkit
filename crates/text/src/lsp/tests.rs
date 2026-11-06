use expect_test::expect;
use ls_types::{
    ClientCapabilities, GeneralClientCapabilities, Position, PositionEncodingKind, Range,
    TextDocumentContentChangeEvent,
};

use super::{Edit, OutOfRange, apply, apply_each, negotiate, offset, position};
use crate::{Encoding, LineIndex, TextSize};

#[test]
fn utf8_when_offered_and_utf16_otherwise() {
    let offering = |kinds: Option<Vec<PositionEncodingKind>>| ClientCapabilities {
        general: Some(GeneralClientCapabilities {
            position_encodings: kinds,
            ..GeneralClientCapabilities::default()
        }),
        ..ClientCapabilities::default()
    };
    let chosen = [
        offering(Some(vec![
            PositionEncodingKind::UTF16,
            PositionEncodingKind::UTF8,
        ])),
        offering(Some(vec![PositionEncodingKind::UTF32])),
        offering(None),
        ClientCapabilities::default(),
    ]
    .iter()
    .map(|capabilities| format!("{:?}", negotiate(capabilities).0))
    .collect::<Vec<_>>();
    assert_eq!(chosen, ["Utf8", "Utf16", "Utf16", "Utf16"]);
}

#[test]
fn a_position_after_multibyte_text_in_each_encoding() {
    let text = "Привіт 👋 recieve\n";
    let index = LineIndex::new(text);
    let at = TextSize::try_from(text.find("recieve").unwrap()).unwrap();
    let positions: Vec<String> = Encoding::ALL
        .iter()
        .map(|&encoding| {
            let p = position(&index, text, at, encoding).unwrap();
            assert_eq!(offset(&index, text, p, encoding), Some(at));
            format!("{encoding:?} {}:{}", p.line, p.character)
        })
        .collect();
    expect![[r#"
        Utf8 0:18
        Utf16 0:10
        Utf32 0:9"#]]
    .assert_eq(&positions.join("\n"));
}

fn change(start: (u32, u32), end: (u32, u32), text: &str) -> TextDocumentContentChangeEvent {
    TextDocumentContentChangeEvent {
        range: Some(Range::new(
            Position::new(start.0, start.1),
            Position::new(end.0, end.1),
        )),
        range_length: None,
        text: text.to_owned(),
    }
}

#[test]
fn changes_land_in_order_where_the_client_said() {
    let mut text = "Привіт: teh value.\n".to_owned();
    // UTF-8 columns: `Привіт: ` is 14 bytes, so `teh` is 14..17, and after it is replaced the
    // `.` is at 23.
    let applied = apply(
        &mut text,
        vec![
            change((0, 14), (0, 17), "the"),
            change((0, 23), (0, 23), " we recieve"),
        ],
        Encoding::Utf8,
    );
    assert_eq!(applied, Ok(()));
    assert_eq!(text, "Привіт: the value we recieve.\n");

    let mut text = "ab\n".to_owned();
    let refused = apply(
        &mut text,
        vec![
            change((0, 0), (0, 1), "x"),
            change((5, 0), (5, 1), "y"),
            change((0, 0), (0, 0), "z"),
        ],
        Encoding::Utf16,
    );
    assert_eq!(refused, Err(OutOfRange));
    assert_eq!(
        text, "xb\n",
        "the changes before the bad one, and none after it"
    );
}

#[test]
fn each_change_is_told_in_bytes_of_the_text_before_it() {
    let mut text = "Привіт: teh value.\n".to_owned();
    let mut told = Vec::new();
    let applied = apply_each(
        &mut text,
        vec![
            change((0, 14), (0, 17), "the"),
            change((0, 23), (0, 23), " we recieve"),
            TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "new".to_owned(),
            },
            change((3, 0), (3, 1), "refused"),
        ],
        Encoding::Utf8,
        |edit| {
            told.push(match edit {
                Edit::Range {
                    before,
                    range,
                    inserted,
                } => format!("{range:?} {:?} -> {inserted:?}", &before[range]),
                Edit::Whole => "whole".to_owned(),
            })
        },
    );
    assert_eq!(applied, Err(OutOfRange));
    expect![[r#"
        14..17 "teh" -> "the"
        23..23 "" -> " we recieve"
        whole"#]]
    .assert_eq(&told.join("\n"));
    assert_eq!(text, "new");
}
