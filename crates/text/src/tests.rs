//! Tests for byte columns, the three encodings and the line terminators.

use std::fmt::Write as _;

use expect_test::expect;

use super::{Encoding, LineCol, LineIndex, TextSize};

const PHP: &str = "<?php\nclass A {\n    public int $x;\n}\n";

fn at(text: &str, needle: &str) -> TextSize {
    TextSize::try_from(text.find(needle).expect("in the text")).unwrap()
}

#[test]
fn maps_offsets_to_lines_and_back() {
    let index = LineIndex::new(PHP);
    let x = at(PHP, "$x");
    let position = index.line_col(x);
    assert_eq!(position, LineCol { line: 2, col: 15 });
    assert_eq!(index.offset(position), Some(x));
}

#[test]
fn a_printed_column_counts_characters_where_a_converted_one_counts_bytes() {
    // `Привет` is six characters and twelve bytes, so a byte count reports a finding about the
    // `)` after it six columns too far right.
    let text = "<?php\n$x = f('Привет', $y);\n";
    let index = LineIndex::new(text);
    let y = at(text, "$y");
    assert_eq!(index.line_col(y), LineCol { line: 1, col: 23 });
    assert_eq!(index.character_col(text, y), 17);
}

#[test]
fn counts_the_line_after_a_trailing_newline() {
    let index = LineIndex::new(PHP);
    assert_eq!(index.line_count(), 5);
    assert_eq!(
        index.line_col(TextSize::of(PHP)),
        LineCol { line: 4, col: 0 }
    );
}

#[test]
fn clamps_an_offset_past_the_end_instead_of_failing() {
    let index = LineIndex::new(PHP);
    let past_end = TextSize::of(PHP) + TextSize::from(99);
    assert_eq!(index.line_col(past_end), index.line_col(TextSize::of(PHP)));
}

#[test]
fn a_line_range_excludes_its_newline() {
    let index = LineIndex::new(PHP);
    let line = index.line_range(1).expect("line 1 exists");
    assert_eq!(&PHP[line], "class A {");
    // The last line of a file ending in a newline is empty, not missing.
    assert!(index.line_range(4).expect("line 4 exists").is_empty());
    assert_eq!(index.line_range(5), None);
}

#[test]
fn rejects_a_byte_column_past_the_end_of_its_line() {
    let index = LineIndex::new("ab\ncd\n");
    assert!(index.offset(LineCol { line: 0, col: 2 }).is_some());
    assert!(index.offset(LineCol { line: 0, col: 9 }).is_none());
    assert!(index.offset(LineCol { line: 7, col: 0 }).is_none());
}

/// ASCII, a two-byte letter, an astral emoji, and all three terminators.
const MIXED: &str = "aб😀\r\nx\ry\n";

#[test]
fn every_boundary_in_every_encoding() {
    let index = LineIndex::new(MIXED);
    let mut out = String::new();
    for (at, c) in MIXED.char_indices().chain([(MIXED.len(), '∎')]) {
        let at = TextSize::try_from(at).unwrap();
        let positions: Vec<String> = Encoding::ALL
            .iter()
            .map(|&encoding| match index.line_col_in(MIXED, at, encoding) {
                Some(pos) => format!("{}:{}", pos.line, pos.col),
                None => "-".to_owned(),
            })
            .collect();
        let bytes = index.line_col(at);
        writeln!(
            out,
            "{c:?} @{} utf8/16/32 {} bytes {}:{}",
            u32::from(at),
            positions.join(" "),
            bytes.line,
            bytes.col
        )
        .unwrap();
    }
    expect![[r#"
        'a' @0 utf8/16/32 0:0 0:0 0:0 bytes 0:0
        'б' @1 utf8/16/32 0:1 0:1 0:1 bytes 0:1
        '😀' @3 utf8/16/32 0:3 0:2 0:2 bytes 0:3
        '\r' @7 utf8/16/32 0:7 0:4 0:3 bytes 0:7
        '\n' @8 utf8/16/32 - - - bytes 0:8
        'x' @9 utf8/16/32 1:0 1:0 1:0 bytes 1:0
        '\r' @10 utf8/16/32 1:1 1:1 1:1 bytes 1:1
        'y' @11 utf8/16/32 2:0 2:0 2:0 bytes 2:0
        '\n' @12 utf8/16/32 2:1 2:1 2:1 bytes 2:1
        '∎' @13 utf8/16/32 3:0 3:0 3:0 bytes 3:0
    "#]]
    .assert_eq(&out);
}

#[test]
fn every_position_round_trips() {
    for text in [
        MIXED,
        "",
        "\n",
        "\r\r\n",
        "plain",
        "Привіт 👋\nworld",
        "\r\n\r\n",
    ] {
        let index = LineIndex::new(text);
        let mut unnamed = 0;
        for at in (0..=text.len()).filter(|&i| text.is_char_boundary(i)) {
            let at = TextSize::try_from(at).unwrap();
            assert_eq!(
                index.offset(index.line_col(at)),
                Some(at),
                "{text:?} @{at:?} bytes"
            );
            for encoding in Encoding::ALL {
                match index.line_col_in(text, at, encoding) {
                    Some(pos) => assert_eq!(
                        index.offset_in(text, pos, encoding),
                        Some(at),
                        "{text:?} @{at:?} {encoding:?}"
                    ),
                    None => unnamed += 1,
                }
            }
        }
        // The only boundary no position can name is the one inside a `\r\n`.
        assert_eq!(
            unnamed,
            text.matches("\r\n").count() * Encoding::ALL.len(),
            "{text:?}"
        );
    }
}

#[test]
fn a_column_past_the_line_is_its_end_and_one_inside_a_character_is_nothing() {
    let text = "a😀b\nc";
    let index = LineIndex::new(text);
    let at = |line, col, encoding| {
        index
            .offset_in(text, LineCol { line, col }, encoding)
            .map(u32::from)
    };
    assert_eq!(
        at(0, 3, Encoding::Utf16),
        Some(5),
        "after the surrogate pair"
    );
    assert_eq!(
        at(0, 99, Encoding::Utf16),
        Some(6),
        "clamped before the terminator"
    );
    assert_eq!(
        at(1, 99, Encoding::Utf16),
        Some(8),
        "clamped to the end of the text"
    );
    assert_eq!(at(2, 0, Encoding::Utf16), None, "no such line");
    assert_eq!(
        at(0, 2, Encoding::Utf16),
        None,
        "between the halves of a surrogate pair"
    );
    assert_eq!(at(0, 2, Encoding::Utf8), None, "inside the emoji's bytes");
}

#[test]
fn line_content_excludes_the_whole_terminator_and_line_range_its_last_byte() {
    let index = LineIndex::new(MIXED);
    let lines: Vec<String> = (0..index.line_count() as u32)
        .map(|line| {
            format!(
                "{:?} / {:?}",
                &MIXED[index.line_content(MIXED, line).unwrap()],
                &MIXED[index.line_range(line).unwrap()]
            )
        })
        .collect();
    expect![[r#"
        "aб😀" / "aб😀\r"
        "x" / "x"
        "y" / "y"
        "" / """#]]
    .assert_eq(&lines.join("\n"));
}

#[test]
fn a_column_near_the_largest_is_past_its_line_not_wrapped_onto_it() {
    let index = LineIndex::new("ab\ncd\n");
    assert_eq!(
        index.offset(LineCol {
            line: 1,
            col: u32::MAX
        }),
        None
    );
    assert_eq!(
        index.offset(LineCol {
            line: 1,
            col: u32::MAX - 2
        }),
        None
    );
}

/// The ASCII and UTF-8 shortcuts agree with walking the characters, at every boundary and every
/// column of every line, in every encoding.
#[test]
fn the_shortcuts_agree_with_walking_the_characters() {
    for text in [
        MIXED,
        "plain ascii line\nand another",
        "ascii then ü then ascii\r\nПривіт 👋 world",
        "👋",
        "",
    ] {
        let index = LineIndex::new(text);
        for encoding in Encoding::ALL {
            for line in 0..index.line_count() as u32 + 1 {
                for col in 0..40 {
                    let pos = LineCol { line, col };
                    assert_eq!(
                        index.offset_in(text, pos, encoding),
                        walked_offset(&index, text, pos, encoding),
                        "{text:?} {pos:?} {encoding:?}"
                    );
                }
            }
            for at in (0..=text.len()).filter(|&i| text.is_char_boundary(i)) {
                let at = TextSize::try_from(at).unwrap();
                let walked = index.line_col_in(text, at, encoding).map(|pos| {
                    let start = usize::from(index.line_range(pos.line).unwrap().start());
                    let col = text[start..usize::from(at)]
                        .chars()
                        .map(|c| match encoding {
                            Encoding::Utf8 => c.len_utf8() as u32,
                            Encoding::Utf16 => c.len_utf16() as u32,
                            Encoding::Utf32 => 1,
                        })
                        .sum::<u32>();
                    LineCol {
                        line: pos.line,
                        col,
                    }
                });
                assert_eq!(
                    index.line_col_in(text, at, encoding),
                    walked,
                    "{text:?} @{at:?} {encoding:?}"
                );
            }
        }
    }
}

/// `offset_in` without its shortcuts, walking every character.
fn walked_offset(
    index: &LineIndex,
    text: &str,
    pos: LineCol,
    encoding: Encoding,
) -> Option<TextSize> {
    let content = index.line_content(text, pos.line)?;
    let (start, end) = (usize::from(content.start()), usize::from(content.end()));
    let mut col = 0;
    for (i, c) in text[start..end].char_indices() {
        if col == pos.col {
            return Some(TextSize::try_from(start + i).unwrap());
        }
        col += match encoding {
            Encoding::Utf8 => c.len_utf8() as u32,
            Encoding::Utf16 => c.len_utf16() as u32,
            Encoding::Utf32 => 1,
        };
        if col > pos.col {
            return None;
        }
    }
    Some(TextSize::try_from(end).unwrap())
}

/// An index cut off after a line answers every position up to it as the full index does.
/// `lsp::apply` relies on this to index only as far as its edit.
#[test]
fn an_index_through_a_line_agrees_with_the_whole_one_up_to_it() {
    for text in [
        MIXED,
        "\r\r\n\n",
        "one\ntwo\r\nthree\rfour",
        "",
        "no terminator",
    ] {
        let whole = LineIndex::new(text);
        for last in 0..whole.line_count() + 2 {
            let through = LineIndex::through(text, last);
            for line in 0..=last as u32 {
                for col in 0..12 {
                    for encoding in Encoding::ALL {
                        let pos = LineCol { line, col };
                        assert_eq!(
                            through.offset_in(text, pos, encoding),
                            whole.offset_in(text, pos, encoding),
                            "{text:?} through {last}, {pos:?} {encoding:?}"
                        );
                    }
                }
            }
        }
    }
}
