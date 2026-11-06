//! kitserver's language checks. This crate knows nothing about the protocol.
//!
//! It takes text and returns findings, with ranges as byte offsets into that text. The server
//! converts them to the client's positions and the command line prints them. Because no protocol
//! type is used here, the editor, `kitserver check` and `kitserver perf` all run the same check and
//! get the same results.
//!
//! The check below (trailing whitespace) is the template's example. Replace it with the language's
//! own.

use lspkit_text::{TextRange, TextSize};

#[cfg(test)]
mod tests;

/// A problem found in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Byte range in the checked text.
    pub range: TextRange,
    /// A stable name that users can turn off and tests can assert on.
    pub code: &'static str,
    pub message: String,
}

/// Every finding in `text`.
pub fn check(text: &str) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        let kept = content.trim_end_matches([' ', '\t']);
        if kept.len() < content.len() {
            found.push(Finding {
                range: TextRange::new(offset(start + kept.len()), offset(start + content.len())),
                code: "trailing-whitespace",
                message: "whitespace at the end of a line".to_owned(),
            });
        }
        start += line.len();
    }
    found
}

fn offset(at: usize) -> TextSize {
    TextSize::try_from(at).expect("a text under 4 GiB")
}
