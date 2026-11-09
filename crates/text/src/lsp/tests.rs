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

/// Edits on `\r\n` text, on the last line without a terminator, across lines, and past a line's
/// end, which LSP reads as the end of the line.
#[test]
fn changes_land_on_crlf_text_and_on_the_last_line() {
    let mut text = "ab\r\ncd\r\nef".to_owned();
    let mut told = Vec::new();
    let applied = apply_each(
        &mut text,
        vec![
            // Past the end of line 1: the end of its content, before the `\r\n`.
            change((1, 9), (1, 9), "!"),
            // The last line, which has no terminator.
            change((2, 1), (2, 2), "F"),
            // Across lines, taking a terminator with it.
            change((0, 1), (1, 1), "-"),
        ],
        Encoding::Utf16,
        |edit| {
            if let Edit::Range {
                before,
                range,
                inserted,
            } = edit
            {
                told.push(format!("{range:?} {:?} -> {inserted:?}", &before[range]));
            }
        },
    );
    assert_eq!(applied, Ok(()));
    expect![[r#"
        6..6 "" -> "!"
        10..11 "f" -> "F"
        1..5 "b\r\nc" -> "-""#]]
    .assert_eq(&told.join("\n"));
    assert_eq!(text, "a-d!\r\neF");

    let mut text = "ab\ncd".to_owned();
    assert_eq!(
        apply(
            &mut text,
            vec![change((2, 0), (2, 0), "x")],
            Encoding::Utf16
        ),
        Err(OutOfRange),
        "a line past the end"
    );
    assert_eq!(
        apply(
            &mut text,
            vec![change((1, 1), (0, 1), "x")],
            Encoding::Utf16
        ),
        Err(OutOfRange),
        "a range that ends before it starts"
    );
    assert_eq!(text, "ab\ncd");
}
