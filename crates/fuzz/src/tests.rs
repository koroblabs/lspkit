use std::{
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
};

use expect_test::expect;

use crate::{Edit, Edits, Files, Random, Script, Subject, Sweep};

#[test]
fn the_same_seed_gives_the_same_edits_and_every_one_lands_on_a_boundary() {
    let text = "héllo 👋 world\nsecond line\n";
    let palette = &["x", "\n", "е", "//"];
    let first = Edits::new(7, palette).sequence(text, 60);
    assert_eq!(first, Edits::new(7, palette).sequence(text, 60));
    assert_ne!(first, Edits::new(8, palette).sequence(text, 60));

    let mut edited = text.to_owned();
    let mut kinds = [0; 4];
    for edit in &first {
        assert!(edit.fits(&edited), "{edit} on {edited:?}");
        let kind = match (edit.range.is_empty(), edit.insert.is_empty()) {
            (true, _) => 0,
            (false, true) => 1,
            (false, false) if u32::from(edit.range.len()) as usize == edit.insert.len() => 3,
            (false, false) => 2,
        };
        kinds[kind] += 1;
        edit.apply(&mut edited);
    }
    assert!(
        kinds.iter().all(|&n| n > 0),
        "insertions, deletions, replacements and same-length ones: {kinds:?}"
    );
}

#[test]
fn an_edit_names_what_it_changes() {
    let text = "let a = 1;";
    let edit = Edit::replace(text, "a", "bee");
    assert_eq!(edit.to_string(), r#"4..5 -> "bee""#);
    let mut changed = text.to_owned();
    edit.apply(&mut changed);
    assert_eq!(changed, "let bee = 1;");
    assert_ne!(Random::new(0).number(), 0, "a zero seed still moves");
}

fn files() -> Files {
    [("a.txt", "alpha beta\n"), ("b.txt", "gamma\n")]
        .into_iter()
        .map(|(path, text)| (path.to_owned(), text.to_owned()))
        .collect()
}

/// How many words each file has, one line per file.
#[derive(PartialEq)]
struct Words(String);

impl fmt::Display for Words {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn words(files: &Files) -> Words {
    Words(
        files
            .iter()
            .map(|(path, text)| format!("{path} {}\n", text.split_whitespace().count()))
            .collect(),
    )
}

fn panic_message(run: impl FnOnce()) -> String {
    let failed = catch_unwind(AssertUnwindSafe(run)).expect_err("the sweep fails");
    failed
        .downcast_ref::<String>()
        .cloned()
        .expect("a formatted message")
}

/// A live state that keeps up passes: it holds the texts and answers from them.
#[test]
fn a_live_state_that_keeps_up_passes() {
    let sweep = Sweep {
        seeds: &[1, 2, 3],
        edits: 50,
        palette: &[" ", "word", "\n"],
        span: 6,
    };
    sweep.run(
        &files(),
        &mut Subject {
            open: Files::clone,
            edited: |live: &mut Files, path: &str, _: &Edit, now: &str| {
                live.insert(path.to_owned(), now.to_owned());
            },
            answer: |live: &mut Files| words(live),
            scratch: words,
        },
    );
}

/// A live state that ignores edits to one file falls behind. The sweep names the seed and the
/// edit, and shrinks the list to the single edit that suffices.
#[test]
fn a_live_state_that_misses_an_edit_is_named_and_shrunk() {
    let sweep = Sweep {
        seeds: &[5],
        edits: 50,
        palette: &[" word "],
        span: 6,
    };
    let message = panic_message(|| {
        sweep.run(
            &files(),
            &mut Subject {
                open: Files::clone,
                edited: |live: &mut Files, path: &str, _: &Edit, now: &str| {
                    if path != "b.txt" {
                        live.insert(path.to_owned(), now.to_owned());
                    }
                },
                answer: |live: &mut Files| words(live),
                scratch: words,
            },
        );
    });
    let head = message.lines().take(8).collect::<Vec<_>>().join("\n");
    expect![[r#"
        seed 5, edit 7 (b.txt: 2..2 -> " word "): the live state and a fresh build disagree
        first difference at line 2:
          from scratch: b.txt 2
          incremental:  b.txt 1

        shrunk to 1 of 8 edits:
          b.txt: 6..6 -> " word "
    "#]]
    .assert_eq(&head);
}

/// A cache keyed by a text's length, like the stale state in orthra's mutants. Edits that change
/// the length always miss it; a same-length edit hits it and exposes the stale answer.
#[test]
fn a_cache_that_misses_only_same_length_edits_is_caught_by_one() {
    struct Cached {
        texts: Files,
        // What the last answer was, and the total length it was taken at.
        kept: Option<(usize, String)>,
    }
    let answer = |live: &mut Cached| {
        let length: usize = live.texts.values().map(String::len).sum();
        match &live.kept {
            Some((at, said)) if *at == length => Words(said.clone()),
            _ => {
                let said = words(&live.texts).0;
                live.kept = Some((length, said.clone()));
                Words(said)
            }
        }
    };
    let subject = || Subject {
        open: |files: &Files| Cached {
            texts: files.clone(),
            kept: None,
        },
        edited: |live: &mut Cached, path: &str, _: &Edit, now: &str| {
            live.texts.insert(path.to_owned(), now.to_owned());
        },
        answer,
        scratch: words,
    };

    // A length change misses the cache, so it never answers stale.
    let script = Script::new(&files()).replace("a.txt", "beta", "be ta");
    subject().script(&files(), &script);

    // Two words become one, at the same length.
    let script = Script::new(&files()).replace("a.txt", "alpha beta", "alpha-beta");
    let message = panic_message(|| subject().script(&files(), &script));
    expect![[r#"
        a script, edit 0 (a.txt: 0..10 -> "alpha-beta"): the live state and a fresh build disagree
        first difference at line 1:
          from scratch: a.txt 1
          incremental:  a.txt 2"#]]
    .assert_eq(&message.lines().take(4).collect::<Vec<_>>().join("\n"));
}

/// Dropping an edit moves the later ones to where they land without it, reading an edge as it was
/// meant: after deleting `gam` and then `m`, dropping the first leaves `m` deleted at 3.
#[test]
fn a_dropped_edit_moves_the_later_ones_to_where_they_land_without_it() {
    let step = |path: &str, start: u32, end: u32, insert: &str| crate::Step {
        path: path.to_owned(),
        edit: Edit::new(crate::TextRange::new(start.into(), end.into()), insert),
    };
    let render = |steps: Option<Vec<crate::Step>>| {
        steps.map(|steps| {
            steps
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        })
    };
    let without = |steps: &[crate::Step], at| render(crate::shrink::without(steps, at));

    // "gamma": "gam" deleted, then "m", then something pasted at the end.
    let deletions = [
        step("b", 0, 3, ""),
        step("b", 0, 1, ""),
        step("b", 1, 1, "!"),
        step("a", 0, 0, "untouched"),
    ];
    assert_eq!(
        without(&deletions, 0).as_deref(),
        Some(r#"b: 3..4 -> "", b: 4..4 -> "!", a: 0..0 -> "untouched""#)
    );

    // An insertion, then an edit inside what it inserted: nowhere to move it to.
    let inside = [step("b", 2, 2, "xyz"), step("b", 3, 4, "q")];
    assert_eq!(without(&inside, 0), None);

    // An insertion, then edits on both sides of it.
    let around = [
        step("b", 2, 2, "xyz"),
        step("b", 0, 1, "Q"),
        step("b", 6, 7, ""),
    ];
    assert_eq!(
        without(&around, 0).as_deref(),
        Some(r#"b: 0..1 -> "Q", b: 3..4 -> """#)
    );
}

/// A deletion dropped, then an insertion exactly at the gap it left, then an edit inside what was
/// inserted. The insertion lands after the gap, so the later edit moves by what the deletion
/// removed.
#[test]
fn an_edit_inside_an_insertion_at_a_dropped_deletions_gap_moves_past_what_it_removed() {
    let step = |start: u32, end: u32, insert: &str| crate::Step {
        path: "b".to_owned(),
        edit: Edit::new(crate::TextRange::new(start.into(), end.into()), insert),
    };
    // "abcdef": "cd" deleted, "xyz" inserted at 2, then the "y" of it replaced.
    let steps = [step(2, 4, ""), step(2, 2, "xyz"), step(3, 4, "Q")];
    let kept = crate::shrink::without(&steps, 0).expect("the edit is outside what was dropped");
    let rendered: Vec<String> = kept.iter().map(ToString::to_string).collect();
    assert_eq!(rendered, [r#"b: 4..4 -> "xyz""#, r#"b: 5..6 -> "Q""#]);
}

#[test]
fn renderings_that_differ_only_where_lines_do_not_show_are_said_to() {
    assert_eq!(
        crate::first_difference("a\nb", "a\nb"),
        "the renderings are identical line by line; the values differ only by `PartialEq`"
    );
    assert_eq!(
        crate::first_difference("a\nb\n", "a\nb"),
        "the renderings are identical line by line; they differ only in a trailing newline"
    );
    assert_eq!(
        crate::first_difference("a\nb", "a\nc"),
        "first difference at line 2:\n  from scratch: b\n  incremental:  c"
    );
}
