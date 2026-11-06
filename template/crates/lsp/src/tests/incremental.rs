//! A buffer edited in pieces with UTF-16 ranges, as an editor does, must hold the same text and
//! findings as that text opened whole. Runs `lspkit_fuzz`'s sweep with characters whose lengths
//! in bytes, UTF-16 units and code points differ.

use lspkit_fuzz::{Edit, Files, Script, Subject, Sweep};
use lspkit_runtime::protocol::ls_types::{TextDocumentContentChangeEvent, Uri};
use lspkit_text::{Encoding, LineIndex};

use crate::documents::{Document, Documents};

const PATH: &str = "/a.txt";

fn uri() -> Uri {
    "file:///a.txt".parse().expect("a URI")
}

/// The text, and what the check finds in it.
fn rendered(text: &str) -> String {
    let mut out = format!("{text:?}\n");
    for finding in kitserver_check::check(text) {
        out.push_str(&format!("{:?} {}\n", finding.range, finding.code));
    }
    out
}

fn open(files: &Files) -> (Documents, i32) {
    let mut documents = Documents::default();
    let text = files[PATH].clone();
    documents.open(uri(), Document { text, version: 0 });
    (documents, 0)
}

/// Sends the edit as a client would: a UTF-16 range into the text before the edit.
fn send((documents, version): &mut (Documents, i32), _: &str, edit: &Edit, _: &str) {
    let before = documents
        .get(&uri())
        .expect("the buffer is open")
        .text
        .clone();
    let index = LineIndex::new(&before);
    let range = lspkit_text::lsp::range(&index, &before, edit.range, Encoding::Utf16)
        .expect("the client can name every range on a character boundary");
    *version += 1;
    let change = TextDocumentContentChangeEvent {
        range: Some(range),
        range_length: None,
        text: edit.insert.clone(),
    };
    documents
        .change(&uri(), *version, vec![change], Encoding::Utf16)
        .expect("the change applies");
}

#[test]
fn edits_sent_in_pieces_hold_what_the_whole_text_holds() {
    let files: Files = [(
        PATH.to_owned(),
        "héllo  wörld 👋\nsecond\t\nthird\n".to_owned(),
    )]
    .into();
    let mut subject = Subject {
        open,
        edited: send,
        answer: |(documents, _): &mut (Documents, i32)| {
            rendered(&documents.get(&uri()).expect("the buffer is open").text)
        },
        scratch: |files: &Files| rendered(&files[PATH]),
    };
    Sweep {
        seeds: &[1, 2, 3, 5, 8, 13],
        edits: 60,
        palette: &[" ", "\t", "x", "é", "👋", "\n", "  \n"],
        span: 6,
    }
    .run(&files, &mut subject);

    // Same-length edits too: nothing after them moves, which is what exposes a stale cache.
    let script = Script::new(&files)
        .replace(PATH, "  w", "xxw")
        .replace(PATH, "\t", " ");
    subject.script(&files, &script);
}
