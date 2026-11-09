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

    /// How many units `bytes`, whole characters of UTF-8, span in this encoding.
    ///
    /// Counted from the bytes without decoding a character: every byte but a continuation byte
    /// begins a character, and in UTF-16 a four-byte character (a lead byte of `0xF0` or more) is a
    /// surrogate pair, so two units. The loops are plain enough for the compiler to vectorize,
    /// where decoding each character branches on it.
    fn units(self, bytes: &[u8]) -> u32 {
        // A line is part of a text a `TextSize` addresses, so its length fits.
        match self {
            Encoding::Utf8 => bytes.len() as u32,
            Encoding::Utf32 => bytes.iter().filter(|&&byte| !is_continuation(byte)).count() as u32,
            Encoding::Utf16 => bytes
                .iter()
                .map(|&byte| u32::from(!is_continuation(byte)) + u32::from(byte >= 0xF0))
                .sum(),
        }
    }
}

/// Whether `byte` continues a multi-byte character (`10xxxxxx`), so that no column begins at it.
const fn is_continuation(byte: u8) -> bool {
    byte & 0xC0 == 0x80
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
        let mut starts = vec![TextSize::from(0)];
        each_line_start(text, |start| {
            starts.push(size(start));
            true
        });
        // Grown by doubling while scanning, so up to half of it would be spare. An index lives
        // beside its text for as long as the text is open, and the one copy this costs is cheaper
        // than keeping the slack for that long.
        starts.shrink_to_fit();
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
            .map_or(at.col, |prefix| Encoding::Utf32.units(prefix.as_bytes()))
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
        // a long line, such as in a minified file, then costs an ASCII scan instead of a count.
        let col = if encoding == Encoding::Utf8 || before.is_ascii() {
            before.len() as u32
        } else {
            encoding.units(before.as_bytes())
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
        let at = column_offset(&text[start..end], pos.col, encoding)?;
        Some(size(start + at))
    }

    fn content_end(&self, text: &str, line: usize) -> TextSize {
        let next = self.starts.get(line + 1).map(|&next| usize::from(next));
        size(content_end(text, next))
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

/// The byte offset in `content`, one line without its terminator, of column `col` counted in
/// `encoding`: the line's end for a column past it, as LSP specifies, and `None` inside a
/// character.
pub(crate) fn column_offset(content: &str, col: u32, encoding: Encoding) -> Option<usize> {
    let wanted = col as usize;
    // In UTF-8 the column is the byte offset.
    if encoding == Encoding::Utf8 {
        return if wanted >= content.len() {
            Some(content.len())
        } else {
            content.is_char_boundary(wanted).then_some(wanted)
        };
    }
    // In every encoding it is over ASCII too, so only a column past non-ASCII text counts units,
    // and from the first non-ASCII byte. `is_ascii` first: it is vectorized, where finding the
    // position is a byte at a time, and a column deep in a minified line is 500 KB of ASCII.
    let bytes = content.as_bytes();
    let prefix = &bytes[..wanted.min(bytes.len())];
    if prefix.is_ascii() {
        return Some(prefix.len());
    }
    let first = prefix
        .iter()
        .position(|byte| !byte.is_ascii())
        .unwrap_or(prefix.len());
    let mut units = first as u32;
    let mut at = first;
    // Whole chunks that end at or before the column are counted at once, the vectorized way
    // `units` counts, and only the chunk the column falls in is walked. A chunk boundary inside a
    // character is fine: a lead byte is counted in its own chunk, and the walk below steps over a
    // continuation byte without counting it.
    const CHUNK: usize = 64;
    for chunk in bytes[first..].as_chunks::<CHUNK>().0 {
        let in_chunk = encoding.units(chunk);
        if units + in_chunk > col {
            break;
        }
        units += in_chunk;
        at += CHUNK;
    }
    // Each lead byte begins one unit, or two in UTF-16 for a four-byte character, and a
    // continuation byte begins none, so no character is decoded.
    for (at, &byte) in bytes.iter().enumerate().skip(at) {
        if is_continuation(byte) {
            continue;
        }
        if units == col {
            return Some(at);
        }
        units += 1 + u32::from(encoding == Encoding::Utf16 && byte >= 0xF0);
        if units > col {
            return None;
        }
    }
    Some(bytes.len())
}

/// Where the content of the line starting after `next_start`'s predecessor ends: before the
/// terminator whose end `next_start` is, or at the text's end for the last line.
fn content_end(text: &str, next_start: Option<usize>) -> usize {
    match next_start {
        None => text.len(),
        Some(next) => {
            next - if text.as_bytes()[..next].ends_with(b"\r\n") {
                2
            } else {
                1
            }
        }
    }
}

/// The content ranges (without the terminator) of lines `a` and `b`, in that order, from one scan
/// of `text` that stores no line start, or `None` when either line does not exist.
///
/// `lsp::apply` converts an edit's two positions with this. It used to build an index through the
/// edit's line on every change, which was the one allocation on that path, a `Vec` as long as the
/// edit was far down the file.
#[cfg(any(test, feature = "lsp"))]
pub(crate) fn content_bounds(text: &str, a: u32, b: u32) -> Option<[(usize, usize); 2]> {
    let (low, high) = (a.min(b) as usize, a.max(b) as usize);
    // Where lines `low`, `low + 1`, `high` and `high + 1` start, for those that exist.
    let mut starts = [None; 4];
    let mut line = 0;
    if low == 0 {
        starts[0] = Some(0);
    }
    if high == 0 {
        starts[2] = Some(0);
    }
    each_line_start(text, |start| {
        line += 1;
        if line == low {
            starts[0] = Some(start);
        }
        if line == low + 1 {
            starts[1] = Some(start);
        }
        if line == high {
            starts[2] = Some(start);
        }
        if line == high + 1 {
            starts[3] = Some(start);
            return false;
        }
        true
    });
    let low_bounds = (starts[0]?, content_end(text, starts[1]));
    let high_bounds = (starts[2]?, content_end(text, starts[3]));
    Some(if a <= b {
        [low_bounds, high_bounds]
    } else {
        [high_bounds, low_bounds]
    })
}

/// Calls `each` with the start of every line after the first, in order (the offset after each
/// `\n`, `\r\n` or lone `\r`), until it returns `false` or the text ends.
///
/// A closure rather than an iterator: the scan is one search for either byte, with the pairing of
/// `\r\n` as a few comparisons per line, and keeping that state in an iterator struct made the
/// index of a 1 MB file 27 % slower (310 µs against 244) than this loop, whose state stays in
/// registers. Two merged searches for one byte each were tried too, and a search for `\r` in text
/// that has none reads the whole text before yielding, which an edit near the top must not pay.
fn each_line_start(text: &str, mut each: impl FnMut(usize) -> bool) {
    let bytes = text.as_bytes();
    // The `\n` of a `\r\n`, already counted with its `\r`.
    let mut paired = usize::MAX;
    for at in memchr::memchr2_iter(b'\n', b'\r', bytes) {
        if at == paired {
            continue;
        }
        let start = if bytes[at] == b'\r' && bytes.get(at + 1) == Some(&b'\n') {
            paired = at + 1;
            at + 2
        } else {
            at + 1
        };
        if !each(start) {
            return;
        }
    }
}

pub(crate) fn size(at: usize) -> TextSize {
    TextSize::try_from(at).expect("a text over 4 GiB")
}

// Runs the README's examples as doctests so they stay in sync with the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
