//! A project's findings counted by code, against a baseline whose counts may fall but not rise.
//!
//! A strict check is only usable if its existing findings can be accepted and new ones cannot slip
//! in. The baseline records the accepted count per code. phrost's and orthra's `check --ratchet`
//! agreed on the rules below and differed in the file format, so [`Baseline::parse`] reads both.
//!
//! - Counts are compared per code, because ten fixed `unknown-word` findings and ten new
//!   `mixed-script` ones leave the total unchanged.
//! - A code the baseline does not name counts as a rise from zero, so enabling a new check shows
//!   up as a diff someone reviews.
//! - A fall is reported, and the server decides what happens to the file
//!   ([`Comparison::outcome`]). orthra writes the lower count back at once, so a later rise to the
//!   old number cannot pass. phrost waits for `--accept`, because its baselines live in other
//!   people's checkouts and a check must not write there unasked.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    path::Path,
};

mod error;
#[cfg(test)]
mod tests;

pub use self::error::Error;

/// Findings by code: `unknown-word` -> 768.
pub type Counts = BTreeMap<String, usize>;

/// The finding counts a project has accepted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Baseline {
    /// How many files the counts were taken over. It lets a reader of the file tell a rise from a
    /// bigger sweep; [`Baseline::compare`] ignores it.
    pub files: Option<usize>,
    pub counts: Counts,
}

impl Baseline {
    pub fn new(files: Option<usize>, counts: Counts) -> Baseline {
        Baseline { files, counts }
    }

    /// Parses a baseline: `files = N` and a `[counts]` table, or codes and counts at the top level
    /// (the older form orthra wrote).
    pub fn parse(text: &str) -> Result<Baseline, Error> {
        Baseline::read(text).map_err(|reason| Error::Malformed { path: None, reason })
    }

    fn read(text: &str) -> Result<Baseline, String> {
        let table: toml::Table = text
            .parse()
            .map_err(|err: toml::de::Error| err.to_string())?;
        let files = match table.get("files") {
            Some(files) => Some(count("files", files)?),
            None => None,
        };
        let counts = match table.get("counts") {
            Some(counts) => counts
                .as_table()
                .ok_or("[counts] is not a table")?
                .iter()
                .map(|(code, n)| Ok((code.clone(), count(code, n)?)))
                .collect::<Result<_, String>>()?,
            None => table
                .iter()
                .filter(|(key, _)| *key != "files")
                .map(|(code, n)| Ok((code.clone(), count(code, n)?)))
                .collect::<Result<_, String>>()?,
        };
        Ok(Baseline { files, counts })
    }

    /// The baseline at `path`, `Ok(None)` when there is none.
    pub fn load(path: &Path) -> Result<Option<Baseline>, Error> {
        match std::fs::read_to_string(path) {
            Ok(text) => Baseline::read(&text)
                .map(Some)
                .map_err(|reason| Error::Malformed {
                    path: Some(path.to_owned()),
                    reason,
                }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(Error::Io {
                path: path.to_owned(),
                source,
            }),
        }
    }

    /// The file's text. `command` is the command that rewrites it, such as
    /// `phrost check --ratchet --accept`.
    pub fn render(&self, command: &str) -> String {
        // The command sits in a `#` comment, where a newline would start a line of TOML.
        let command = command.replace(['\n', '\r'], " ");
        let mut text = format!(
            "# What this project has agreed to be told about itself, by code.\n\
             #\n\
             # Written by `{command}`. A count may fall; one that rises fails the check, and so does\n\
             # a code nothing here mentions -- both are decisions rather than accidents, and this file\n\
             # is the diff somebody reads.\n\n"
        );
        if let Some(files) = self.files {
            let _ = writeln!(text, "files = {files}\n");
        }
        text.push_str("[counts]\n");
        for (code, count) in &self.counts {
            let _ = writeln!(text, "{} = {count}", toml_key(code));
        }
        text
    }

    /// Writes the file whole or not at all: to a temporary file beside it, then renamed over it. A
    /// crash mid-write cannot leave an empty baseline that the next run would read as accepting
    /// nothing.
    pub fn store(&self, path: &Path, command: &str) -> Result<(), Error> {
        let fail = |source: std::io::Error| Error::Io {
            path: path.to_owned(),
            source,
        };
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = std::path::PathBuf::from(temporary);
        std::fs::write(&temporary, self.render(command)).map_err(fail)?;
        std::fs::rename(&temporary, path).map_err(|err| {
            let _ = std::fs::remove_file(&temporary);
            fail(err)
        })
    }

    /// Compares `found` with the accepted counts.
    pub fn compare(&self, found: &Counts) -> Comparison {
        let codes: BTreeSet<&String> = self.counts.keys().chain(found.keys()).collect();
        let moved = codes
            .into_iter()
            .filter_map(|code| {
                let was = self.counts.get(code).copied();
                let now = found.get(code).copied().unwrap_or(0);
                // A code found zero times that nothing agreed to is not a change at all.
                let unchanged = was == Some(now) || (was.is_none() && now == 0);
                (!unchanged).then(|| Moved {
                    code: code.clone(),
                    was,
                    now,
                })
            })
            .collect();
        Comparison { moved }
    }
}

/// `code` as a single-line TOML basic string, since a key cannot span lines. Rust's `{:?}` would
/// write a combining character as `\u{301}`, which is not valid TOML.
fn toml_key(code: &str) -> String {
    let mut out = String::with_capacity(code.len() + 2);
    out.push('"');
    for c in code.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn count(key: &str, value: &toml::Value) -> Result<usize, String> {
    value
        .as_integer()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| format!("{key}: not a count"))
}

/// Every code whose count is not what was agreed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    pub moved: Vec<Moved>,
}

/// One code that moved. `was` is `None` for a code the baseline does not name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub code: String,
    pub was: Option<usize>,
    pub now: usize,
}

impl Moved {
    pub fn rose(&self) -> bool {
        self.now > self.was.unwrap_or(0)
    }
}

/// What a comparison comes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing moved.
    Held,
    /// Some code fell and none rose: the baseline can be lowered to what was found.
    Fell,
    /// Some code rose, or appeared: the check fails.
    Rose,
}

impl Comparison {
    pub fn outcome(&self) -> Outcome {
        if self.moved.iter().any(Moved::rose) {
            Outcome::Rose
        } else if self.moved.is_empty() {
            Outcome::Held
        } else {
            Outcome::Fell
        }
    }

    /// One line per code that moved, rises first: `  possibly-null    12 -> 14  ROSE`.
    pub fn report(&self) -> String {
        let mut moved: Vec<&Moved> = self.moved.iter().collect();
        moved.sort_by_key(|moved| (!moved.rose(), moved.code.clone()));
        let mut out = String::new();
        for moved in moved {
            let was = moved
                .was
                .map_or_else(|| "-".to_owned(), |was| was.to_string());
            let word = match (moved.was, moved.rose()) {
                (None, _) => "NEW",
                (Some(_), true) => "ROSE",
                (Some(_), false) => "fell",
            };
            let _ = writeln!(out, "  {:<32}{was:>8} -> {}  {word}", moved.code, moved.now);
        }
        out
    }
}

// Compiles and runs the README's examples as doctests, so they stay in step with the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
