use expect_test::expect;

use crate::{
    reads::{Allowed, DEFAULT_PATTERNS, Reads},
    workspace::SourceFile,
};

fn file(path: &str, text: &str) -> SourceFile {
    SourceFile {
        path: path.to_owned(),
        text: text.to_owned(),
    }
}

const READS: Reads<'static> = Reads {
    patterns: DEFAULT_PATTERNS,
    unpoliced: &[
        ("crates/cli/", "a command line"),
        ("crates/gone/", "deleted"),
    ],
    allowed: &[
        Allowed {
            file: "crates/vfs/src/source.rs",
            why: "reading a file's text is what the VFS is",
        },
        Allowed {
            file: "crates/index/src/old.rs",
            why: "it used to map the cache",
        },
        Allowed {
            file: "crates/index/src/never.rs",
            why: " ",
        },
    ],
    min_policed: 3,
};

#[test]
fn reads_exemptions_and_their_staleness() {
    let files = [
        file(
            "crates/index/src/lookup.rs",
            "// fs::read_to_string in a comment is prose\n\
             let stamp = FileStamp::new(); // not fs::File\n\
             if path.exists() { return fallback(); }\n\
             let text = std::fs::read_to_string(&p)?;\n",
        ),
        file("crates/index/src/tests.rs", "std::fs::read(\"fixture\");\n"),
        file(
            "crates/index/src/tests/disk.rs",
            "std::fs::read(\"fixture\");\n",
        ),
        file("crates/cli/src/main.rs", "std::fs::read(\"anything\");\n"),
        file("crates/vfs/src/source.rs", "fs::read_to_string(path)\n"),
        file("crates/index/src/old.rs", "pub fn lookup() {}\n"),
    ];
    expect![[r#"
        crates/index/src/lookup.rs:3: `.exists()` reaches the filesystem on the request path. If it is where a fact comes from, add the file to the allowed reads with what it is for; if it makes up for a missing answer, it is a fallback
        crates/index/src/lookup.rs:4: `fs::read` reaches the filesystem on the request path. If it is where a fact comes from, add the file to the allowed reads with what it is for; if it makes up for a missing answer, it is a fallback
        crates/index/src/old.rs is allowed to read and no longer does: remove the exemption, or the next person to touch the file inherits it
        crates/index/src/never.rs is allowed to read without a reason
        crates/index/src/never.rs is allowed to read, and was not scanned or does not exist
        crates/gone/ is unpoliced and matches no file
    "#]]
    .assert_eq(&(READS.violations(&files).join("\n") + "\n"));
}

#[test]
fn a_slash_slash_inside_a_string_does_not_hide_the_read() {
    let files = [
        file(
            "crates/index/src/a.rs",
            "let u = \"http://x\"; std::fs::read(p);\n\
             let v = \"a\\\"//b\"; std::fs::read(p);\n\
             let w = 1; // \"quoted\" std::fs::read(p)\n",
        ),
        file("crates/index/src/b.rs", "x\n"),
        file("crates/index/src/c.rs", "x\n"),
    ];
    let reads = Reads {
        unpoliced: &[],
        allowed: &[],
        ..READS
    };
    expect![[r#"
        crates/index/src/a.rs:1: `fs::read` reaches the filesystem on the request path. If it is where a fact comes from, add the file to the allowed reads with what it is for; if it makes up for a missing answer, it is a fallback
        crates/index/src/a.rs:2: `fs::read` reaches the filesystem on the request path. If it is where a fact comes from, add the file to the allowed reads with what it is for; if it makes up for a missing answer, it is a fallback
    "#]].assert_eq(&(reads.violations(&files).join("\n") + "\n"));
}

#[test]
fn a_scan_that_saw_too_little_says_so() {
    let reads = Reads {
        unpoliced: &[],
        allowed: &[],
        ..READS
    };
    assert_eq!(
        reads.violations(&[file("crates/a/src/lib.rs", "")]),
        ["only 1 files were policed, fewer than 3, so the scan proves nothing"]
    );
}
