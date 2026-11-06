//! Finds a project's files for a language server.
//!
//! - Files that git ignores are left out, through `.gitignore`, `.ignore` and
//!   `.git/info/exclude`. A `.gitignore` counts without a `.git`, so a project unpacked from a
//!   tarball reads the same as a checkout.
//! - The user's global git excludes are never read. They differ between machines, and a project's
//!   files must not.
//! - The walk runs on every thread. In phrost, the directory reads and `stat`s of a tree of thirty
//!   thousand files took 0.3 s of every start on one thread.
//! - [`Walk::files`] sorts by the paths' bytes, so the order does not depend on which thread
//!   reached a file first.
//!
//! The builder holds what each server decides: whether hidden directories count as code, which
//! directories are never entered, and which files are wanted.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
};

use ignore::{WalkBuilder, WalkState};

pub use ignore::overrides::{Override, OverrideBuilder};

#[cfg(test)]
mod tests;

/// A file or directory the walk reached, passed to [`Walk::prune`].
#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    pub path: &'a Path,
    pub is_dir: bool,
}

type Judge = Arc<dyn Fn(Entry<'_>) -> bool + Send + Sync>;
type Wanted = Arc<dyn Fn(&Path) -> bool + Send + Sync>;

/// A walk over one or more roots. Configure it, then call [`Walk::files`].
#[derive(Clone)]
pub struct Walk {
    roots: Vec<PathBuf>,
    ignore_files: bool,
    skip_hidden: bool,
    overrides: Option<Override>,
    prune: Option<Judge>,
    keep: Option<Wanted>,
}

impl std::fmt::Debug for Walk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Walk")
            .field("roots", &self.roots)
            .field("ignore_files", &self.ignore_files)
            .field("skip_hidden", &self.skip_hidden)
            .finish_non_exhaustive()
    }
}

impl Walk {
    /// A walk of every file under `roots` (a root may itself be a file). By default it respects
    /// ignore files and skips hidden entries.
    #[must_use]
    pub fn new<P: Into<PathBuf>>(roots: impl IntoIterator<Item = P>) -> Walk {
        Walk {
            roots: roots.into_iter().map(Into::into).collect(),
            ignore_files: true,
            skip_hidden: true,
            overrides: None,
            prune: None,
            keep: None,
        }
    }

    /// Whether `.gitignore`, `.ignore` and `.git/info/exclude` are read. Turn it off to walk a
    /// directory whose files are all wanted regardless, such as a dependency directory the
    /// project's `.gitignore` lists.
    #[must_use]
    pub fn ignore_files(mut self, read: bool) -> Walk {
        self.ignore_files = read;
        self
    }

    /// Whether hidden entries are skipped. phrost walks them, because some projects keep source in
    /// a dot-prefixed directory and skipping it silently loses its declarations.
    #[must_use]
    pub fn skip_hidden(mut self, skip: bool) -> Walk {
        self.skip_hidden = skip;
        self
    }

    /// Globs from the project's own settings. Files they exclude are left out like ignored files.
    #[must_use]
    pub fn overrides(mut self, overrides: Override) -> Walk {
        self.overrides = Some(overrides);
        self
    }

    /// Skips every entry for which `prune` returns `true`, and never enters such a directory,
    /// whatever the ignore files say. Use it for directories nobody writes by hand, such as `.git`,
    /// `node_modules` or a tool's cache.
    #[must_use]
    pub fn prune(mut self, prune: impl Fn(Entry<'_>) -> bool + Send + Sync + 'static) -> Walk {
        self.prune = Some(Arc::new(prune));
        self
    }

    /// Returns only the files for which `keep` returns `true`, such as a server's extensions.
    #[must_use]
    pub fn keep(mut self, keep: impl Fn(&Path) -> bool + Send + Sync + 'static) -> Walk {
        self.keep = Some(Arc::new(keep));
        self
    }

    /// Every file the walk reaches, sorted by the paths' bytes. Each file appears once even when
    /// roots overlap (`[a, a/b]`, or a root twice).
    ///
    /// The walk is best-effort: unreadable directories and missing roots are skipped silently.
    /// Symlinks are not followed, so a symlinked file or directory is not listed.
    #[must_use]
    pub fn files(&self) -> Vec<PathBuf> {
        let mut files = self.files_unordered();
        // Sort by bytes instead of `Path`'s component-wise order. On phrost's 29 000 files this
        // takes 10 ms of every start, against 50 ms.
        files.sort_unstable_by(|a, b| a.as_os_str().cmp(b.as_os_str()));
        files.dedup();
        files
    }

    /// Every file the walk reaches, in the order the threads reached them.
    ///
    /// Use it when the caller puts the files in an ordered map anyway, to avoid sorting twice.
    /// phrost's start does, and the sort was 10 ms of its 180. Overlapping roots (`[a, a/b]`, or a
    /// root twice) repeat the files they share, unlike [`Walk::files`]. Unreadable directories,
    /// missing roots and symlinks are handled as in [`Walk::files`].
    #[must_use]
    pub fn files_unordered(&self) -> Vec<PathBuf> {
        let Some((first, rest)) = self.roots.split_first() else {
            return Vec::new();
        };
        let mut builder = WalkBuilder::new(first);
        for root in rest {
            builder.add(root);
        }
        builder
            .standard_filters(self.ignore_files)
            // Skip the user's global excludes, so a project's files are the same on every machine.
            .git_global(false)
            // Honor a `.gitignore` even without a `.git` beside it.
            .require_git(false)
            // Read ancestors' ignore files only inside a git repository, where git reads them too
            // (and `ignore` stops at the repository's root). Outside one, a `~/.gitignore` of `*`,
            // common in dotfile repos, would empty every project beneath it.
            .parents(self.ignore_files && self.roots.iter().all(|root| inside_a_repository(root)))
            .hidden(self.skip_hidden);
        if let Some(overrides) = &self.overrides {
            builder.overrides(overrides.clone());
        }
        if let Some(prune) = self.prune.clone() {
            builder.filter_entry(move |entry| {
                let is_dir = entry.file_type().is_some_and(|kind| kind.is_dir());
                !prune(Entry {
                    path: entry.path(),
                    is_dir,
                })
            });
        }

        // Walk on every thread. The work is directory reads and `stat`s, which the kernel serves
        // concurrently, and `files` sorts away the order they arrive in.
        let (send, receive) = mpsc::channel::<PathBuf>();
        let keep = self.keep.clone();
        builder.build_parallel().run(|| {
            let send = send.clone();
            let keep = keep.clone();
            Box::new(move |entry| {
                let Ok(entry) = entry else {
                    return WalkState::Continue;
                };
                if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                    return WalkState::Continue;
                }
                let path = entry.into_path();
                if keep.as_ref().is_none_or(|keep| keep(&path)) && send.send(path).is_err() {
                    return WalkState::Quit;
                }
                WalkState::Continue
            })
        });
        drop(send);
        receive.into_iter().collect()
    }
}

/// Whether `root` or one of its ancestors holds a `.git` entry: a directory, or the file a
/// worktree or a submodule has instead.
fn inside_a_repository(root: &Path) -> bool {
    let root = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
    root.ancestors()
        .any(|dir| dir.join(".git").symlink_metadata().is_ok())
}

// Runs the README's examples as doctests so they stay in sync with the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
