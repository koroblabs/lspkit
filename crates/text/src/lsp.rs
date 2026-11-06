//! Protocol positions, ranges and edits, in the encoding agreed at `initialize`.

use ls_types::{
    ClientCapabilities, Position, PositionEncodingKind, Range, TextDocumentContentChangeEvent,
};

use crate::{Encoding, LineCol, LineIndex, TextRange, TextSize};

#[cfg(test)]
mod tests;

/// Picks the position encoding: UTF-8 if the client offers it, since server columns are bytes and
/// nothing then needs converting, else UTF-16, the protocol's default.
pub fn negotiate(capabilities: &ClientCapabilities) -> (Encoding, PositionEncodingKind) {
    let offered = capabilities
        .general
        .as_ref()
        .and_then(|general| general.position_encodings.as_ref());
    if offered.is_some_and(|kinds| kinds.contains(&PositionEncodingKind::UTF8)) {
        (Encoding::Utf8, PositionEncodingKind::UTF8)
    } else {
        (Encoding::Utf16, PositionEncodingKind::UTF16)
    }
}

/// The protocol's position of `at`, or `None` where no position can name it.
pub fn position(
    index: &LineIndex,
    text: &str,
    at: TextSize,
    encoding: Encoding,
) -> Option<Position> {
    let pos = index.line_col_in(text, at, encoding)?;
    Some(Position::new(pos.line, pos.col))
}

pub fn range(index: &LineIndex, text: &str, range: TextRange, encoding: Encoding) -> Option<Range> {
    Some(Range::new(
        position(index, text, range.start(), encoding)?,
        position(index, text, range.end(), encoding)?,
    ))
}

/// The offset of a protocol position, or `None` when the text has no such position.
pub fn offset(
    index: &LineIndex,
    text: &str,
    position: Position,
    encoding: Encoding,
) -> Option<TextSize> {
    let pos = LineCol {
        line: position.line,
        col: position.character,
    };
    index.offset_in(text, pos, encoding)
}

/// A change named a range that is not in the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfRange;

/// Applies a client's changes to `text` in order. Each change replaces either the whole text or a
/// range counted in `encoding`.
///
/// A change outside the text is a client bug and returns [`OutOfRange`]. The changes before it
/// stay applied, and the rest are skipped.
pub fn apply(
    text: &mut String,
    changes: Vec<TextDocumentContentChangeEvent>,
    encoding: Encoding,
) -> Result<(), OutOfRange> {
    apply_each(text, changes, encoding, |_| {})
}

/// What one change is about to do to a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit<'a> {
    /// The bytes in `range` of `before`, the text before the change, are replaced by `inserted`.
    Range {
        before: &'a str,
        range: TextRange,
        inserted: &'a str,
    },
    /// The text is replaced whole.
    Whole,
}

/// [`apply`], calling `each` with every change, in bytes, just before it is applied. State kept
/// beside the text, such as a syntax tree, can be edited from these in the same order and stays
/// in step with the text. `each` is not called for a refused change.
pub fn apply_each(
    text: &mut String,
    changes: Vec<TextDocumentContentChangeEvent>,
    encoding: Encoding,
    mut each: impl FnMut(Edit<'_>),
) -> Result<(), OutOfRange> {
    for change in changes {
        let Some(range) = change.range else {
            each(Edit::Whole);
            *text = change.text;
            continue;
        };
        let last = range.start.line.max(range.end.line) as usize;
        let index = LineIndex::through(text, last);
        let start = offset(&index, text, range.start, encoding).ok_or(OutOfRange)?;
        let end = offset(&index, text, range.end, encoding).ok_or(OutOfRange)?;
        if start > end {
            return Err(OutOfRange);
        }
        each(Edit::Range {
            before: text,
            range: TextRange::new(start, end),
            inserted: &change.text,
        });
        text.replace_range(usize::from(start)..usize::from(end), &change.text);
    }
    Ok(())
}
