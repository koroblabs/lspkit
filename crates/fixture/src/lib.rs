//! A test workspace written as one string, the usual input of a language server's tests.
//!
//! ```text
//! //- /composer.json
//! {"autoload": {"psr-4": {"App\\": "src/"}}}
//!
//! //- /src/Use.php  open
//! <?php
//! function go(Repo $r) {
//!     $r->fi@member@nd($0);
//! }
//! ```
//!
//! - `//- /path flags` starts a file. Paths start at the workspace root with a leading slash, and
//!   [`Fixture::file`] panics on a path spelled without it.
//! - Flags belong to the server. [`File::flags`] returns them unread, and the server should refuse
//!   one it does not know, so a misspelled flag fails the test instead of building a different
//!   workspace. The only flag read here is `raw`, which keeps markers in that file for languages
//!   whose syntax collides with them (`$0` in shell). [`Fixture::parse_with`] adds more flags that
//!   leave the text as written.
//! - `$0` is the cursor wherever it appears, so `fi$0nd` puts it inside a word. `$` followed by a
//!   letter is never a marker. `@name@` is a named marker whose name is an ASCII identifier, so an
//!   e-mail address or a lone `@` stays text. Both kinds are cut from the text.
//! - A backslash makes either sigil text: `\$0`, `\@name@`. phrost needs this in docblocks, where
//!   `{\@inherit@at@doc}` puts a marker inside a tag.
//! - Every marker must be asked for. [`Markers::assert_all_used`] panics with the name of each one
//!   nothing asked about. This catches a misspelled name, and text that only looked like a marker
//!   and had its sigils silently cut.
//! - A marker name appears once in the whole fixture, so a question about a position is never
//!   ambiguous.
//! - The subject is the last file ([`Fixture::subject`]).
//! - Common indentation is trimmed, so a fixture can be indented to match its call site. Trailing
//!   blank lines are dropped and each file ends in exactly one newline, because the blank line
//!   between two files is layout. For the same reason a whitespace-only line is emptied; a test
//!   about such a line should pass its text directly instead of through a fixture.
//!
//! A fixture is test code, so malformed input panics with a specific message instead of returning
//! an error.

mod markers;
mod parse;

#[cfg(test)]
mod tests;

pub use self::markers::{CURSOR, Marker, Markers};
pub use text_size::TextSize;

/// A parsed fixture: its files, and the markers cut out of them.
#[derive(Debug, Clone)]
pub struct Fixture {
    files: Vec<File>,
    markers: Markers,
}

/// One file of a fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    path: String,
    flags: Vec<String>,
    text: String,
    verbatim: bool,
}

impl Fixture {
    /// Parses the format described in the crate documentation.
    pub fn parse(source: &str) -> Fixture {
        Fixture::parse_with(source, &[])
    }

    /// [`Fixture::parse`], but a file with any of `verbatim` among its flags keeps its text as
    /// written, like `raw`. This is for files whose text is read some other way, such as an
    /// annotated rendering where cutting a marker would shift every caret after it.
    pub fn parse_with(source: &str, verbatim: &[&str]) -> Fixture {
        parse::Reader::new(verbatim).read(source)
    }

    pub fn files(&self) -> &[File] {
        &self.files
    }

    /// The file at `path`, spelled as its header spells it: from the root, with the slash.
    pub fn file(&self, path: &str) -> &File {
        assert!(
            path.starts_with('/'),
            "fixture paths start at the root, with the slash: {path:?}"
        );
        self.files
            .iter()
            .find(|file| file.path == path)
            .unwrap_or_else(|| {
                let declared: Vec<&str> = self.files.iter().map(File::path).collect();
                panic!("fixture has no file at {path}; it has {declared:?}")
            })
    }

    /// The last file, which is the one a test is about.
    pub fn subject(&self) -> &File {
        self.files
            .last()
            .expect("parsing refuses a fixture with no files")
    }

    pub fn markers(&self) -> &Markers {
        &self.markers
    }

    /// The files and the markers, for a harness that keeps them apart or presents them its own way.
    pub fn into_parts(self) -> (Vec<File>, Markers) {
        (self.files, self.markers)
    }
}

impl File {
    /// The path from the root, with the leading slash, as the header wrote it.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The header's flags after the path, in order, except `raw`, which this crate handles.
    pub fn flags(&self) -> &[String] {
        &self.flags
    }

    /// The text with every marker cut and every escape resolved, unless the file is verbatim.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether the text is as written: `raw`, or a flag [`Fixture::parse_with`] named.
    pub fn is_verbatim(&self) -> bool {
        self.verbatim
    }
}

// Compiles and runs the README's examples as doctests, so they stay in step with the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
