//! Tracks which closed files a project-wide pass reported problems for, so the next pass clears
//! only those.
//!
//! A client shows the last findings it was sent for a file until it gets new ones, so a closed file
//! whose problems were fixed must be sent an empty list. Sending one for every clean file would
//! mean thirty thousand empty notifications per session on a thirty-thousand-file project.

use std::{borrow::Borrow, collections::HashSet, hash::Hash};

/// The closed files last published with problems.
#[derive(Debug)]
pub struct Reported<K> {
    files: HashSet<K>,
}

impl<K> Default for Reported<K> {
    fn default() -> Self {
        Reported {
            files: HashSet::new(),
        }
    }
}

impl<K: Eq + Hash> Reported<K> {
    /// Records whether `file` has problems now, and returns whether its findings should be sent:
    /// when it has some, or when it had some that the client still shows. The key is borrowed, so
    /// a clean file (most of a pass) costs no allocation.
    pub fn note<Q>(&mut self, file: &Q, any: bool) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ToOwned<Owned = K> + ?Sized,
    {
        if !any {
            return self.files.remove(file);
        }
        if !self.files.contains(file) {
            self.files.insert(file.to_owned());
        }
        true
    }

    /// Stops tracking `file` because it is open now and its findings come from the buffer. A later
    /// pass must not clear it as a closed file it did not see.
    pub fn forget<Q>(&mut self, file: &Q)
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.files.remove(file);
    }

    /// Removes and returns the files reported earlier that are not in `seen` (deleted, or no longer
    /// checked), so they can be sent an empty list. Call it only after a pass over the whole
    /// project: a pass that stopped part-way says nothing about the files it did not reach.
    pub fn unseen(&mut self, seen: &HashSet<K>) -> Vec<K> {
        let (gone, kept) = std::mem::take(&mut self.files)
            .into_iter()
            .partition(|file| !seen.contains(file));
        self.files = kept;
        gone.into_iter().collect()
    }

    pub fn contains<Q>(&self, file: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.files.contains(file)
    }
}
