use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};

use text_size::TextSize;

/// The name `$0` is recorded under.
pub const CURSOR: &str = "0";

/// Where a marker was, in the text it was cut from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    pub path: String,
    pub offset: TextSize,
}

/// The markers cut from a fixture, and which of them a test has asked about.
///
/// A marker nobody asks about is usually a typo, or text that looked like a marker and silently
/// lost its sigils. Either way the test no longer tests the code it shows.
#[derive(Debug, Default)]
pub struct Markers {
    by_name: BTreeMap<String, Marker>,
    used: RefCell<BTreeSet<String>>,
}

impl Markers {
    /// The marker called `name`, recorded as asked about.
    pub fn position(&self, name: &str) -> &Marker {
        self.used.borrow_mut().insert(name.to_owned());
        self.by_name.get(name).unwrap_or_else(|| {
            panic!(
                "fixture has no marker `{name}`; it declares {}",
                self.declared_names()
            )
        })
    }

    /// The `$0` cursor.
    pub fn cursor(&self) -> &Marker {
        self.position(CURSOR)
    }

    /// Every marker, by name, without recording any as asked about.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Marker)> {
        self.by_name
            .iter()
            .map(|(name, marker)| (name.as_str(), marker))
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// The declared names, quoted, for a message: "`a`, `b`", or `<none>` when there are none.
    pub fn declared_names(&self) -> String {
        if self.by_name.is_empty() {
            return "<none>".to_owned();
        }
        self.by_name
            .keys()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Panics with the name of every marker nothing asked about. Call it after the test's last
    /// question.
    pub fn assert_all_used(&self) {
        let used = self.used.borrow();
        let unused: Vec<String> = self
            .by_name
            .keys()
            .filter(|name| !used.contains(*name))
            .map(|name| format!("`{name}`"))
            .collect();
        assert!(
            unused.is_empty(),
            "fixture declares unused marker(s): {}. Ask about them, or put a backslash before the \
             sigil if it is text: `\\$0`, `\\@name@`",
            unused.join(", ")
        );
    }

    /// The same markers with every path rewritten, for a harness that spells paths its own way.
    pub fn map_paths(self, rewrite: impl Fn(&str) -> String) -> Markers {
        Markers {
            by_name: self
                .by_name
                .into_iter()
                .map(|(name, marker)| {
                    let path = rewrite(&marker.path);
                    (name, Marker { path, ..marker })
                })
                .collect(),
            used: self.used,
        }
    }

    pub(crate) fn insert(&mut self, name: &str, marker: Marker) {
        let previous = self.by_name.insert(name.to_owned(), marker);
        assert!(
            previous.is_none(),
            "fixture has two markers named `{name}`; a question about one position cannot pick \
             between them"
        );
    }
}

impl Clone for Markers {
    /// Clones the markers but not the record of what was asked about, so each clone tracks its
    /// own. A harness that runs one fixture twice (open and closed, cold and warm) must not let the
    /// first run satisfy the second run's check.
    fn clone(&self) -> Self {
        Markers {
            by_name: self.by_name.clone(),
            used: RefCell::default(),
        }
    }
}
