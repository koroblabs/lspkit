//! Every filesystem read on the path a request takes, with the reason it is there.
//!
//! In a language server, a fallback is usually a read: a lookup finds nothing, and the code goes to
//! disk to make up for it. Every read must be listed with its purpose, and an unlisted one fails.
//! A read that is the source of a fact belongs in [`Reads::allowed`]. A read that covers for a
//! missing index answer is a fallback; fix the index, or answer nothing.
//!
//! The check is a plain text scan. It skips test sources (a `tests/` directory or a `tests.rs`
//! file) and anything after `//` on a line, so comments can mention a read.

use crate::workspace::SourceFile;

/// The usual value of [`Reads::patterns`]: calls that reach the disk. Building a `PathBuf` does
/// not match, since a check that flagged it would be too noisy to keep. Asking whether a path
/// exists does match, because `if path.exists()` is the easiest way to write a fallback without
/// noticing.
pub const DEFAULT_PATTERNS: &[&str] = &[
    "fs::read",
    "fs::write",
    "fs::metadata",
    "fs::read_dir",
    "fs::read_to_string",
    "fs::File",
    "File::open",
    "File::create",
    "OpenOptions",
    "Mmap",
    "WalkBuilder::new",
    ".is_file()",
    ".is_dir()",
    ".exists()",
    ".try_exists()",
];

/// One file permitted to read, and what for.
#[derive(Debug, Clone, Copy)]
pub struct Allowed {
    /// From the workspace root.
    pub file: &'static str,
    pub why: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct Reads<'a> {
    pub patterns: &'a [&'a str],
    /// Path prefixes, from the root, that no request passes through, and why: a command line, test
    /// support, the checks themselves.
    pub unpoliced: &'a [(&'a str, &'a str)],
    pub allowed: &'a [Allowed],
    /// The fewest policed files a run must see. Fewer means the scan is looking in the wrong place,
    /// and an empty result would prove nothing.
    pub min_policed: usize,
}

impl Reads<'_> {
    pub fn is_test(path: &str) -> bool {
        path.contains("/tests/") || path.ends_with("/tests.rs")
    }

    pub fn violations(&self, files: &[SourceFile]) -> Vec<String> {
        let mut out = Vec::new();
        let mut policed = 0;
        for file in files {
            if Self::is_test(&file.path) || self.is_unpoliced(&file.path) {
                continue;
            }
            policed += 1;
            if self.allowed.iter().any(|allowed| allowed.file == file.path) {
                continue;
            }
            for (line, read) in self.reads(&file.text) {
                out.push(format!(
                    "{}:{line}: `{read}` reaches the filesystem on the request path. If it is where \
                     a fact comes from, add the file to the allowed reads with what it is for; if it \
                     makes up for a missing answer, it is a fallback",
                    file.path
                ));
            }
        }
        if policed < self.min_policed {
            out.push(format!(
                "only {policed} files were policed, fewer than {}, so the scan proves nothing",
                self.min_policed
            ));
        }

        for allowed in self.allowed {
            if allowed.why.trim().is_empty() {
                out.push(format!(
                    "{} is allowed to read without a reason",
                    allowed.file
                ));
            }
            match files.iter().find(|file| file.path == allowed.file) {
                None => out.push(format!(
                    "{} is allowed to read, and was not scanned or does not exist",
                    allowed.file
                )),
                Some(file) if self.reads(&file.text).is_empty() => out.push(format!(
                    "{} is allowed to read and no longer does: remove the exemption, or the next \
                     person to touch the file inherits it",
                    allowed.file
                )),
                Some(_) => {}
            }
        }
        for (prefix, _) in self.unpoliced {
            if !files.iter().any(|file| file.path.starts_with(prefix)) {
                out.push(format!("{prefix} is unpoliced and matches no file"));
            }
        }
        out
    }

    fn is_unpoliced(&self, path: &str) -> bool {
        self.unpoliced
            .iter()
            .any(|(prefix, _)| path.starts_with(prefix))
    }

    /// The line numbers that read, with the pattern each one matched.
    fn reads(&self, text: &str) -> Vec<(usize, &str)> {
        text.lines()
            .enumerate()
            .filter_map(|(i, line)| {
                let code = Self::code_of(line);
                self.patterns
                    .iter()
                    .find(|read| Self::mentions(code, read))
                    .map(|read| (i + 1, *read))
            })
            .collect()
    }

    /// `line` up to the first `//` outside a `"..."` string, so `"http://x"` does not hide a read
    /// after it. Only unescaped double quotes are tracked, so a `'"'` char literal or a raw string
    /// containing a quote can confuse it. That costs at most one missed or spurious read on the
    /// line.
    fn code_of(line: &str) -> &str {
        let bytes = line.as_bytes();
        let (mut in_string, mut i) = (false, 0);
        while i < bytes.len() {
            match bytes[i] {
                b'\\' if in_string => i += 1,
                b'"' => in_string = !in_string,
                b'/' if !in_string && bytes.get(i + 1) == Some(&b'/') => return &line[..i],
                _ => {}
            }
            i += 1;
        }
        line
    }

    /// Whether `code` calls `read`, rather than merely containing its letters.
    ///
    /// A pattern that starts like a name needs a word boundary before it: `MyFile::open` contains
    /// `File::open`. A pattern that starts with punctuation must not get one, because the character
    /// before `.exists()` is the receiver's last letter, as in `path.exists()`. phrost's copy of
    /// this check applied the boundary to both and so missed `path.exists()`, the most common form.
    fn mentions(code: &str, read: &str) -> bool {
        let starts_like_a_name = read.starts_with(|c: char| c.is_alphanumeric() || c == '_');
        code.match_indices(read).any(|(at, _)| {
            !starts_like_a_name
                || code[..at]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !c.is_alphanumeric() && c != '_')
        })
    }
}
