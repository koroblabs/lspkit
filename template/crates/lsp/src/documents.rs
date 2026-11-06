//! The documents open in the editor: their text and version, updated by the client's edits.

use std::collections::HashMap;

use lspkit_runtime::protocol::ls_types::{TextDocumentContentChangeEvent, Uri};
use lspkit_text::Encoding;

#[derive(Debug)]
pub(crate) struct Document {
    pub(crate) text: String,
    pub(crate) version: i32,
}

#[derive(Debug, Default)]
pub(crate) struct Documents {
    open: HashMap<Uri, Document>,
}

/// Why a change was not applied.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    NotOpen,
    /// The change is older than the held version. The client and server disagree on the order of
    /// edits, and applying it would produce the wrong text.
    Stale {
        held: i32,
    },
    /// A range outside the text. The changes before it stay applied.
    OutOfRange,
}

impl Documents {
    pub(crate) fn open(&mut self, uri: Uri, document: Document) {
        self.open.insert(uri, document);
    }

    pub(crate) fn close(&mut self, uri: &Uri) -> Option<Document> {
        self.open.remove(uri)
    }

    pub(crate) fn get(&self, uri: &Uri) -> Option<&Document> {
        self.open.get(uri)
    }

    /// Applies the client's changes in order. Each one replaces the whole text or one range.
    pub(crate) fn change(
        &mut self,
        uri: &Uri,
        version: i32,
        changes: Vec<TextDocumentContentChangeEvent>,
        encoding: Encoding,
    ) -> Result<(), Refused> {
        let document = self.open.get_mut(uri).ok_or(Refused::NotOpen)?;
        if version < document.version {
            return Err(Refused::Stale {
                held: document.version,
            });
        }
        document.version = version;
        lspkit_text::lsp::apply(&mut document.text, changes, encoding)
            .map_err(|_| Refused::OutOfRange)
    }
}
