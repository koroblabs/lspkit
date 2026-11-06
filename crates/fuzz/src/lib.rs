//! Seeded random edits, and a sweep that checks `incremental == from scratch` after each one.
//!
//! Hand-written incremental tests cover only the edits someone thought of. An invalidation bug
//! they miss shows up later as a stale answer that cannot be traced back to the edit that caused
//! it. The server supplies the palette (what an edit pastes in) and what is compared; the edits,
//! the sweep and the shrinking live here.
//!
//! orthra's mutation tests shaped three choices:
//!
//! - Random edits alone miss the worst bugs. Three mutants broke incremental state while every
//!   random fixture passed: an edit that changes the length shifts every range after it, so a
//!   stale cache entry no longer matches and the bug stays hidden. Each was caught by a same-length
//!   edit (`go(` becoming `// `, ` brown` becoming `xbrown`). [`Edits`] makes those on purpose, and
//!   a [`Script`] adds the edits a test knows are risky.
//! - The sweep compares after every edit, so the failing edit is the one reported.
//! - Small fixtures never reach a cache that engages only on big inputs. Lower its threshold in
//!   test builds, or the sweep never exercises the incremental path. orthra cuts its query at 4 KB
//!   in release and at 16 bytes in tests.
//!
//! Runs are deterministic. Every failure names the seed, the edit and the text, and the failing
//! edits are shrunk to the fewest that still fail.

mod shrink;

use std::{
    collections::BTreeMap,
    fmt::{self, Display},
    ops::Range,
};

pub use text_size::{TextRange, TextSize};

#[cfg(test)]
mod tests;

/// A seeded xorshift64, written here rather than taken as a dependency so that a seed's sequence
/// never changes under a test. The quality of the randomness does not decide whether a sweep finds
/// anything.
#[derive(Debug, Clone)]
pub struct Random(u64);

impl Random {
    pub fn new(seed: u64) -> Random {
        // Spread small seeds apart, and never zero: xorshift is stuck there.
        Random(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) | 1)
    }

    /// The next 64 bits of the sequence.
    pub fn number(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A number in `0..limit`; 0 when `limit` is 0.
    pub fn below(&mut self, limit: usize) -> usize {
        match limit {
            0 => 0,
            limit => usize::try_from(self.number() % limit as u64).unwrap_or(0),
        }
    }
}

/// One edit, as a client sends it: the bytes `range` of the current text become `insert`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub range: TextRange,
    pub insert: String,
}

impl Edit {
    pub fn new(range: TextRange, insert: impl Into<String>) -> Edit {
        Edit {
            range,
            insert: insert.into(),
        }
    }

    /// An edit that replaces the first `old` in `text` with `new`.
    ///
    /// # Panics
    ///
    /// When `old` is not in `text`.
    #[must_use]
    pub fn replace(text: &str, old: &str, new: &str) -> Edit {
        let start = text
            .find(old)
            .unwrap_or_else(|| panic!("{old:?} is not in the text"));
        Edit::new(TextRange::at(size(start), TextSize::of(old)), new)
    }

    /// Applies the edit to `text`.
    ///
    /// # Panics
    ///
    /// When the edit does not fit `text`: out of bounds or off a character boundary. Check
    /// [`Edit::fits`] first.
    pub fn apply(&self, text: &mut String) {
        text.replace_range(Range::<usize>::from(self.range), &self.insert);
    }

    /// Whether it can apply to `text`: inside it, and on character boundaries.
    #[must_use]
    pub fn fits(&self, text: &str) -> bool {
        let (start, end) = (
            usize::from(self.range.start()),
            usize::from(self.range.end()),
        );
        end <= text.len() && text.is_char_boundary(start) && text.is_char_boundary(end)
    }
}

impl Display for Edit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (start, end) = (u32::from(self.range.start()), u32::from(self.range.end()));
        write!(f, "{start}..{end} -> {:?}", self.insert)
    }
}

/// Random edits, each pasting something from the server's palette.
///
/// Edits fall on character boundaries, since no editor sends one that splits a UTF-8 sequence. They
/// are insertions, deletions, replacements and same-length replacements. A same-length replacement
/// leaves every later range where it was, so a stale cache entry still matches and its wrong answer
/// shows.
#[derive(Debug, Clone)]
pub struct Edits<'p> {
    random: Random,
    palette: &'p [&'p str],
    span: usize,
}

impl<'p> Edits<'p> {
    /// Edits from `seed`, pasting from `palette`. A good palette holds what makes and breaks the
    /// structure the server keeps, rather than text that looks like its language.
    ///
    /// # Panics
    ///
    /// When `palette` is empty.
    pub fn new(seed: u64, palette: &'p [&'p str]) -> Edits<'p> {
        assert!(!palette.is_empty(), "a palette pastes something");
        Edits {
            random: Random::new(seed),
            palette,
            span: 12,
        }
    }

    /// The most characters an edit removes. 12 unless set.
    #[must_use]
    pub fn span(mut self, characters: usize) -> Edits<'p> {
        self.span = characters;
        self
    }

    /// The next edit, valid against `text`.
    pub fn next(&mut self, text: &str) -> Edit {
        let starts: Vec<usize> = (0..=text.len())
            .filter(|&at| text.is_char_boundary(at))
            .collect();
        let from = self.random.below(starts.len());
        let paste = self.palette[self.random.below(self.palette.len())];
        match self.random.below(4) {
            0 => Edit::new(TextRange::empty(size(starts[from])), paste),
            1 => {
                let to = (from
                    .saturating_add(1)
                    .saturating_add(self.random.below(self.span.max(1))))
                .min(starts.len() - 1);
                Edit::new(TextRange::new(size(starts[from]), size(starts[to])), "")
            }
            2 => {
                let to = (from.saturating_add(self.random.below(self.span.saturating_add(1))))
                    .min(starts.len() - 1);
                Edit::new(TextRange::new(size(starts[from]), size(starts[to])), paste)
            }
            _ => Self::same_length(text, starts[from], paste),
        }
    }

    /// Replaces as many bytes as the paste has if that ends on a character boundary, and inserts
    /// otherwise.
    fn same_length(text: &str, at: usize, paste: &str) -> Edit {
        let width = paste.len();
        if width > 0 && at + width <= text.len() && text.is_char_boundary(at + width) {
            return Edit::new(TextRange::at(size(at), size(width)), paste);
        }
        Edit::new(TextRange::empty(size(at)), paste)
    }

    /// `count` edits, each valid against the text the ones before it produced.
    pub fn sequence(&mut self, text: &str, count: usize) -> Vec<Edit> {
        let mut text = text.to_owned();
        (0..count)
            .map(|_| {
                let edit = self.next(&text);
                edit.apply(&mut text);
                edit
            })
            .collect()
    }
}

/// A project's texts by path, as a sweep keeps them.
pub type Files = BTreeMap<String, String>;

/// One edit to one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub path: String,
    pub edit: Edit,
}

impl Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.edit)
    }
}

/// Edits written by a test, each against the text the previous ones left. Use it for the edits a
/// test knows are risky, beside the random ones.
#[derive(Debug, Clone)]
pub struct Script {
    texts: Files,
    steps: Vec<Step>,
}

impl Script {
    pub fn new(files: &Files) -> Script {
        Script {
            texts: files.clone(),
            steps: Vec::new(),
        }
    }

    /// The first `old` in the file at `path`, as it is now, becomes `new`.
    #[must_use]
    pub fn replace(self, path: &str, old: &str, new: &str) -> Script {
        let edit = Edit::replace(self.text(path), old, new);
        self.edit(path, edit)
    }

    #[must_use]
    pub fn edit(mut self, path: &str, edit: Edit) -> Script {
        let text = self
            .texts
            .get_mut(path)
            .unwrap_or_else(|| panic!("the script has no file {path}"));
        assert!(edit.fits(text), "{edit} does not fit {path}");
        edit.apply(text);
        self.steps.push(Step {
            path: path.to_owned(),
            edit,
        });
        self
    }

    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    fn text(&self, path: &str) -> &str {
        self.texts
            .get(path)
            .unwrap_or_else(|| panic!("the script has no file {path}"))
    }
}

/// The four server functions a sweep drives.
///
/// `open` builds the live state from the texts. `edited` tells it that the file at a path got an
/// edit and now reads as the given text. `answer` asks the live state, and `scratch` asks a fresh
/// build of the same texts.
pub struct Subject<O, E, A, S> {
    pub open: O,
    pub edited: E,
    pub answer: A,
    pub scratch: S,
}

impl<O, E, A, S> fmt::Debug for Subject<O, E, A, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Subject").finish_non_exhaustive()
    }
}

impl<L, R, O, E, A, S> Subject<O, E, A, S>
where
    R: PartialEq + Display,
    O: FnMut(&Files) -> L,
    E: FnMut(&mut L, &str, &Edit, &str),
    A: FnMut(&mut L) -> R,
    S: FnMut(&Files) -> R,
{
    /// A [`Script`]'s edits, compared after each one. Panics like [`Sweep::run`] on a difference.
    pub fn script(&mut self, files: &Files, script: &Script) {
        if let Some(failure) = self.replay(files, script.steps()) {
            self.report("a script", files, script.steps(), &failure);
        }
    }

    /// Replays the steps from a fresh open, comparing after each, and returns the first
    /// disagreement. A step that no longer fits its text, which shrinking can cause, ends the
    /// replay as a pass.
    pub(crate) fn replay(&mut self, files: &Files, steps: &[Step]) -> Option<Failure> {
        let mut texts = files.clone();
        let mut live = (self.open)(&texts);
        // Ask once before any edit, as an editor does for a file it just opened. A cache that is
        // never filled cannot go stale, so without this the sweep would pass a broken server.
        (self.answer)(&mut live);
        for (number, step) in steps.iter().enumerate() {
            let text = texts.get_mut(&step.path)?;
            if !step.edit.fits(text) {
                return None;
            }
            step.edit.apply(text);
            let now = text.clone();
            (self.edited)(&mut live, &step.path, &step.edit, &now);
            let (incremental, fresh) = ((self.answer)(&mut live), (self.scratch)(&texts));
            if incremental != fresh {
                return Some(Failure {
                    step: number,
                    difference: first_difference(&fresh.to_string(), &incremental.to_string()),
                    text: now,
                });
            }
        }
        None
    }

    fn report(&mut self, run: &str, files: &Files, steps: &[Step], failure: &Failure) -> ! {
        let failing = &steps[..=failure.step];
        let shrunk = shrink::fewest(files, failing, self);
        let list = |steps: &[Step]| {
            steps
                .iter()
                .map(|step| format!("  {step}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let step = &steps[failure.step];
        panic!(
            "{run}, edit {} ({step}): the live state and a fresh build disagree\n{}\n\n\
             shrunk to {} of {} edits:\n{}\n\n{} is now:\n{}",
            failure.step,
            failure.difference,
            shrunk.len(),
            failing.len(),
            list(&shrunk),
            step.path,
            failure.text
        );
    }
}

/// `incremental == from scratch` under random edits. For every seed, `edits` random edits to random
/// files, compared after each one. A failure panics with the steps and with the fewest of them that
/// still fail.
#[derive(Debug, Clone)]
pub struct Sweep<'a> {
    pub seeds: &'a [u64],
    pub edits: usize,
    pub palette: &'a [&'a str],
    /// The most characters one edit removes.
    pub span: usize,
}

impl Sweep<'_> {
    pub fn run<L, R, O, E, A, S>(&self, files: &Files, subject: &mut Subject<O, E, A, S>)
    where
        R: PartialEq + Display,
        O: FnMut(&Files) -> L,
        E: FnMut(&mut L, &str, &Edit, &str),
        A: FnMut(&mut L) -> R,
        S: FnMut(&Files) -> R,
    {
        assert!(!files.is_empty(), "a sweep needs a file to edit");
        for &seed in self.seeds {
            let steps = self.steps(seed, files);
            if let Some(failure) = subject.replay(files, &steps) {
                subject.report(&format!("seed {seed}"), files, &steps, &failure);
            }
        }
    }

    /// The random edits for `seed`, each valid against the texts the ones before it left.
    pub fn steps(&self, seed: u64, files: &Files) -> Vec<Step> {
        let mut texts = files.clone();
        let paths: Vec<String> = texts.keys().cloned().collect();
        let mut chooser = Random::new(seed ^ 0x9E37_79B9_7F4A_7C15);
        let mut edits = Edits::new(seed, self.palette).span(self.span);
        (0..self.edits)
            .map(|_| {
                let path = paths[chooser.below(paths.len())].clone();
                let text = texts.get_mut(&path).expect("every path has a text");
                let edit = edits.next(text);
                edit.apply(text);
                Step { path, edit }
            })
            .collect()
    }
}

/// Where a replay first went wrong.
#[derive(Debug, Clone)]
pub(crate) struct Failure {
    pub(crate) step: usize,
    pub(crate) difference: String,
    pub(crate) text: String,
}

/// The first line that differs between two renderings, from each.
///
/// Values can differ by `PartialEq` while their renderings agree line by line (`lines()` ignores a
/// trailing newline). Then there is no line to show, and the message says so.
fn first_difference(fresh: &str, incremental: &str) -> String {
    let (mut fresh_lines, mut live_lines) = (fresh.lines(), incremental.lines());
    let mut line = 1;
    loop {
        match (fresh_lines.next(), live_lines.next()) {
            (Some(a), Some(b)) if a == b => line += 1,
            (None, None) if fresh.ends_with('\n') != incremental.ends_with('\n') => {
                return "the renderings are identical line by line; they differ only in a trailing newline"
                    .to_owned();
            }
            (None, None) => {
                return "the renderings are identical line by line; the values differ only by `PartialEq`"
                    .to_owned();
            }
            (a, b) => {
                return format!(
                    "first difference at line {line}:\n  from scratch: {}\n  incremental:  {}",
                    a.unwrap_or("<end>"),
                    b.unwrap_or("<end>")
                );
            }
        }
    }
}

fn size(at: usize) -> TextSize {
    TextSize::try_from(at).expect("a text under 4 GiB")
}

// Compiles and runs the README's examples as doctests, so they stay in step with the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
