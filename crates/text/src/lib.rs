//! Line starts and columns for one text, with columns in bytes, UTF-16 code units or characters.
//!
//! Columns inside the server are bytes. LSP counts UTF-16 code units unless the client offers
//! UTF-8 at `initialize`. The two agree on ASCII, so a wrong conversion passes tests written in
//! English and misplaces the cursor in a file with a Ukrainian comment. This crate does the
//! conversion in one place, so code that only names a position never sees an encoding.
//!
//! With the `lsp` feature, [`lsp`] converts protocol positions and applies the client's edits.

pub use text_size::{TextRange, TextSize};

#[cfg(feature = "lsp")]
pub mod lsp;

#[cfg(test)]
mod tests;

/// The unit a column is counted in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Encoding {
    Utf8,
    Utf16,
    Utf32,
}

impl Encoding {
    pub const ALL: [Encoding; 3] = [Encoding::Utf8, Encoding::Utf16, Encoding::Utf32];

    fn width(self, c: char) -> u32 {
        match self {
            Encoding::Utf8 => c.len_utf8() as u32,
            Encoding::Utf16 => c.len_utf16() as u32,
            Encoding::Utf32 => 1,
        }
    }
}

/// A zero-based line and column. The column is in bytes for [`LineIndex::line_col`] and
/// [`LineIndex::offset`], and in the given [`Encoding`]'s units for their `_in` forms.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

/// The start offset of each line in one text.
///
/// A line ends at `\n`, `\r\n` or a lone `\r`, the three terminators LSP counts. The index holds
/// only offsets, so it is cheap to keep one beside every text. Queries that count characters take
/// the text the index was built from. Querying with a text that has since changed gives positions
/// off by a few lines. A debug build checks the text's length against the index, and a release
/// build may panic on a slice that no longer fits.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LineIndex {
    /// Offset of the first byte of each line. Always starts with 0.
    starts: Vec<TextSize>,
    len: TextSize,
}

impl LineIndex {
    /// Indexes every line of `text`.
    ///
    /// # Panics
    ///
    /// If the text is over 4 GiB, which a [`TextSize`] cannot address.
    #[must_use]
    pub fn new(text: &str) -> LineIndex {
        LineIndex::through(text, usize::MAX)
    }

    /// An index of the lines up to `last` and the start of the line after it, which is all the
    /// positions on those lines need. `lsp::apply` uses it to index only as far as an edit
    /// reaches. A full index would scan the whole text and store every line start on every
    /// keystroke.
    pub(crate) fn through(text: &str, last: usize) -> LineIndex {
        let bytes = text.as_bytes();
        let mut starts = vec![TextSize::from(0)];
        // The `\n` of a `\r\n`, already counted with its `\r`.
        let mut paired = usize::MAX;
        for at in memchr::memchr2_iter(b'\n', b'\r', bytes) {
            if starts.len() > last.saturating_add(1) {
                break;
            }
            if at == paired {
                continue;
            }
            if bytes[at] == b'\r' && bytes.get(at + 1) == Some(&b'\n') {
                paired = at + 1;
                starts.push(size(at + 2));
            } else {
                starts.push(size(at + 1));
            }
        }
        LineIndex {
            starts,
            len: TextSize::of(text),
        }
    }

    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    /// The line and byte column of `offset`, clamping to the end of the text.
    ///
    /// It clamps instead of returning `None` because an offset past the end means the text changed
    /// underneath. Reporting the last position is better than losing the result.
    pub fn line_col(&self, offset: TextSize) -> LineCol {
        let offset = offset.min(self.len);
        // The number of line starts at or before `offset`, less one. There is always a first
        // line start, so this cannot underflow.
        let line = self.starts.partition_point(|start| *start <= offset) - 1;
        LineCol {
            line: u32::try_from(line).expect("line count should fit into u32"),
            col: (offset - self.starts[line]).into(),
        }
    }

    /// The offset of a line and byte column, or `None` if the line does not exist or the column
    /// runs past the start of the next one.
    pub fn offset(&self, position: LineCol) -> Option<TextSize> {
        let line_start = *self.starts.get(position.line as usize)?;
        let line_end = self
            .starts
            .get(position.line as usize + 1)
            .copied()
            .unwrap_or(self.len);
        // Checked, because in a release build a column near `u32::MAX` would wrap to a small
        // offset that passes the test below.
        let offset = line_start.checked_add(TextSize::from(position.col))?;
        (offset <= line_end).then_some(offset)
    }

    /// The byte range of one line without the last byte of its terminator, so the `\r` of a
    /// `\r\n` stays in. [`LineIndex::line_content`] takes the text and excludes the whole
    /// terminator.
    pub fn line_range(&self, line: u32) -> Option<TextRange> {
        let start = *self.starts.get(line as usize)?;
        let end = self
            .starts
            .get(line as usize + 1)
            // The stored start is one past the terminator's last byte, so step back over it.
            .map_or(self.len, |next| *next - TextSize::from(1));
        Some(TextRange::new(start, end.max(start)))
    }

    pub fn range_line_cols(&self, range: TextRange) -> (LineCol, LineCol) {
        (self.line_col(range.start()), self.line_col(range.end()))
    }

    /// The column a person reads: characters into the line, counted from zero.
    ///
    /// [`LineCol::col`] is a byte offset, which conversions need and people do not. Editors follow
    /// `path:line:col`, and on a line holding `'Привет, мир!'` the byte column is nine past the
    /// character a finding is about, so the jump lands there too. In phrost, byte columns put
    /// eight findings across Laravel's and Symfony's test suites off by one to twenty columns.
    pub fn character_col(&self, text: &str, offset: TextSize) -> u32 {
        let at = self.line_col(offset);
        let Some(line) = self.line_range(at.line) else {
            return at.col;
        };
        let (start, here) = (usize::from(line.start()), usize::from(offset));
        text.get(start..here)
            .and_then(|prefix| u32::try_from(prefix.chars().count()).ok())
            .unwrap_or(at.col)
    }

    /// The range of `line` without its terminator.
    pub fn line_content(&self, text: &str, line: u32) -> Option<TextRange> {
        self.check(text);
        let start = *self.starts.get(line as usize)?;
        Some(TextRange::new(start, self.content_end(text, line as usize)))
    }

    /// The position of `at`, with the column counted in `encoding`.
    ///
    /// Returns `None` past the end of the text, inside a character, or between the `\r` and `\n`
    /// of a terminator, since no position can name those places.
    pub fn line_col_in(&self, text: &str, at: TextSize, encoding: Encoding) -> Option<LineCol> {
        self.check(text);
        let at_byte = usize::from(at);
        if at > self.len || !text.is_char_boundary(at_byte) {
            return None;
        }
        let line = self.starts.partition_point(|&start| start <= at) - 1;
        let before = &text[usize::from(self.starts[line])..at_byte];
        if before.ends_with('\r') && text[at_byte..].starts_with('\n') {
            return None;
        }
        // In UTF-8, and for ASCII in every encoding, the byte count is the column. A position on
        // a long line, such as in a minified file, then costs an ASCII scan instead of a decode.
        let col = if encoding == Encoding::Utf8 || before.is_ascii() {
            before.len() as u32
        } else {
            before.chars().map(|c| encoding.width(c)).sum()
        };
        Some(LineCol {
            line: line as u32,
            col,
        })
    }

    /// The offset of `pos`, with the column counted in `encoding`.
    ///
    /// A column past the end of its line means the end of the line, as LSP specifies. Returns
    /// `None` for a line past the end of the text or a column inside a character (such as half of
    /// a surrogate pair).
    pub fn offset_in(&self, text: &str, pos: LineCol, encoding: Encoding) -> Option<TextSize> {
        self.check(text);
        let line = pos.line as usize;
        let start = usize::from(*self.starts.get(line)?);
        let end = usize::from(self.content_end(text, line));
        let content = &text[start..end];
        let wanted = pos.col as usize;
        // As in `line_col_in`, the column is the byte offset in UTF-8, and in every encoding over
        // ASCII. Only a column past non-ASCII text walks the characters.
        if encoding == Encoding::Utf8 {
            return if wanted >= content.len() {
                Some(size(end))
            } else {
                content
                    .is_char_boundary(wanted)
                    .then(|| size(start + wanted))
            };
        }
        let ascii = &content.as_bytes()[..wanted.min(content.len())];
        if ascii.is_ascii() {
            return Some(size(start + ascii.len()));
        }
        let mut col = 0;
        for (i, c) in content.char_indices() {
            if col == pos.col {
                return Some(size(start + i));
            }
            col += encoding.width(c);
            if col > pos.col {
                return None;
            }
        }
        Some(size(end))
    }

    fn content_end(&self, text: &str, line: usize) -> TextSize {
        let Some(&next) = self.starts.get(line + 1) else {
            return self.len;
        };
        let next = usize::from(next);
        let terminator = if text.as_bytes()[..next].ends_with(b"\r\n") {
            2
        } else {
            1
        };
        size(next - terminator)
    }

    /// Catches, in a debug build, an index queried with a text it was not built from.
    fn check(&self, text: &str) {
        debug_assert_eq!(
            TextSize::of(text),
            self.len,
            "a LineIndex queried with another text"
        );
    }
}

fn size(at: usize) -> TextSize {
    TextSize::try_from(at).expect("a text over 4 GiB")
}

// Runs the README's examples as doctests so they stay in sync with the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
