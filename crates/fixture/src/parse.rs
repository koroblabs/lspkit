use text_size::TextSize;

use crate::{CURSOR, File, Fixture, Marker, Markers};

const HEADER: &str = "//- ";

/// The flag this crate reads itself.
const RAW: &str = "raw";

/// Reads one fixture, with the server's list of flags that leave a file's text as written.
pub(crate) struct Reader<'v> {
    verbatim: &'v [&'v str],
}

impl<'v> Reader<'v> {
    pub(crate) fn new(verbatim: &'v [&'v str]) -> Reader<'v> {
        Reader { verbatim }
    }

    pub(crate) fn read(&self, source: &str) -> Fixture {
        let source = trim_indent(source);
        let mut sections: Vec<(&str, String)> = Vec::new();
        for line in source.split_inclusive('\n') {
            if let Some(header) = line.strip_prefix(HEADER) {
                sections.push((header.trim_end(), String::new()));
            } else if let Some((_, body)) = sections.last_mut() {
                body.push_str(line);
            } else {
                assert!(
                    line.trim().is_empty(),
                    "fixture content must start with a `//- /path` header, not {line:?}"
                );
            }
        }
        assert!(
            !sections.is_empty(),
            "fixture has no files: it needs a `//- /path` header"
        );

        let mut files: Vec<File> = Vec::with_capacity(sections.len());
        let mut markers = Markers::default();
        for (header, body) in sections {
            let file = self.file(header, &body, &mut markers);
            assert!(
                !files.iter().any(|other| other.path == file.path),
                "fixture has two files at {}",
                file.path
            );
            files.push(file);
        }
        Fixture { files, markers }
    }

    fn file(&self, header: &str, body: &str, markers: &mut Markers) -> File {
        let mut parts = header.split_whitespace();
        let path = parts
            .next()
            .unwrap_or_else(|| panic!("fixture header `//- {header}` has no path"));
        assert!(
            path.starts_with('/') && path.len() > 1,
            "fixture paths start at the root, with the slash: {path:?}"
        );

        let mut verbatim = false;
        let mut flags = Vec::new();
        for flag in parts {
            if flag == RAW {
                verbatim = true;
                continue;
            }
            verbatim |= self.verbatim.contains(&flag);
            flags.push(flag.to_owned());
        }

        let body = with_one_newline(body);
        let text = if verbatim {
            body
        } else {
            cut_markers(path, &body, markers)
        };
        File {
            path: path.to_owned(),
            flags,
            text,
            verbatim,
        }
    }
}

/// Cuts `$0` and `@name@` out of `body`, recording where each was, and resolves `\$` and `\@`.
///
/// Each offset is the length of the output so far, so it always matches the text it points into.
fn cut_markers(path: &str, body: &str, markers: &mut Markers) -> String {
    let mut text = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(at) = rest.find(['$', '@']) {
        text.push_str(&rest[..at]);
        rest = &rest[at..];
        let sigil = &rest[..1];

        // A backslash makes the sigil text, whether or not a marker would have followed: in
        // `\@@here@` the escaped `@` is text and the marker after it is still read.
        if text.ends_with('\\') {
            text.pop();
            text.push_str(sigil);
            rest = &rest[1..];
            continue;
        }

        let found = match rest.strip_prefix("$0") {
            Some(after) => Some((CURSOR, after)),
            None => named_marker(rest),
        };
        match found {
            Some((name, after)) => {
                let offset = TextSize::of(text.as_str());
                markers.insert(
                    name,
                    Marker {
                        path: path.to_owned(),
                        offset,
                    },
                );
                rest = after;
            }
            None => {
                text.push_str(sigil);
                rest = &rest[1..];
            }
        }
    }
    text.push_str(rest);
    text
}

/// `@name@` at the start of `s`: the name, and what follows the closing `@`. The name is an ASCII
/// identifier, so neither `@@` nor an e-mail address (`a@b.com`) matches: the dot ends the name
/// before a second `@`.
fn named_marker(s: &str) -> Option<(&str, &str)> {
    let body = s.strip_prefix('@')?;
    let len = body
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(body.len());
    let name = &body[..len];
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    body[len..].strip_prefix('@').map(|after| (name, after))
}

/// Removes the indentation every non-blank line shares, so a fixture can be indented to its call
/// site. A blank line may be shorter than the indent, so it is emptied rather than sliced.
fn trim_indent(source: &str) -> String {
    let indent = source
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.bytes()
                .take_while(|byte| matches!(byte, b' ' | b'\t'))
                .count()
        })
        .min()
        .unwrap_or(0);
    source
        .split_inclusive('\n')
        .map(|line| {
            if line.trim().is_empty() {
                line.trim_start_matches([' ', '\t'])
            } else {
                &line[indent..]
            }
        })
        .collect()
}

/// Drops trailing blank lines, and ends a non-empty text in exactly one newline.
fn with_one_newline(body: &str) -> String {
    let mut end = body.len();
    for line in body.split_inclusive('\n').rev() {
        if !line.trim().is_empty() {
            break;
        }
        end -= line.len();
    }
    let mut text = body[..end].to_owned();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text
}
