use expect_test::{Expect, expect};

use crate::{Baseline, Counts, Outcome};

fn counts(pairs: &[(&str, usize)]) -> Counts {
    pairs
        .iter()
        .map(|(code, count)| ((*code).to_owned(), *count))
        .collect()
}

fn check(agreed: &[(&str, usize)], found: &[(&str, usize)], expect: Expect) {
    let comparison = Baseline::new(None, counts(agreed)).compare(&counts(found));
    expect.assert_eq(&format!(
        "{:?}\n{}",
        comparison.outcome(),
        comparison.report()
    ));
}

#[test]
fn a_count_that_rises_fails_and_one_that_falls_can_lower_the_baseline() {
    let agreed = [("known-typo", 39), ("unknown-word", 768)];
    check(
        &agreed,
        &agreed,
        expect![[r#"
        Held
    "#]],
    );
    check(
        &agreed,
        &[("known-typo", 37), ("unknown-word", 768)],
        expect![[r#"
            Fell
              known-typo                            39 -> 37  fell
        "#]],
    );
    check(
        &agreed,
        &[("known-typo", 30), ("unknown-word", 770)],
        expect![[r#"
            Rose
              unknown-word                         768 -> 770  ROSE
              known-typo                            39 -> 30  fell
        "#]],
    );
}

#[test]
fn a_code_the_baseline_does_not_name_is_a_rise_and_one_gone_is_a_fall() {
    check(
        &[("unknown-word", 768), ("retired", 4)],
        &[("mixed-script", 1), ("unknown-word", 768)],
        expect![[r#"
            Rose
              mixed-script                           - -> 1  NEW
              retired                                4 -> 0  fell
        "#]],
    );
}

#[test]
fn the_file_reads_both_forms_and_writes_one() {
    let written = Baseline::new(
        Some(1711),
        counts(&[("possibly-null", 12), ("undefined-method", 3)]),
    );
    let text = written.render("phrost check --ratchet --accept");
    expect![[r#"
        # What this project has agreed to be told about itself, by code.
        #
        # Written by `phrost check --ratchet --accept`. A count may fall; one that rises fails the check, and so does
        # a code nothing here mentions -- both are decisions rather than accidents, and this file
        # is the diff somebody reads.

        files = 1711

        [counts]
        "possibly-null" = 12
        "undefined-method" = 3
    "#]]
    .assert_eq(&text);
    assert_eq!(Baseline::parse(&text).expect("it reads back"), written);

    // orthra's older form: codes at the top level, under its own comment.
    let flat = "# orthra\nknown-typo = 39\nunknown-word = 768\n";
    assert_eq!(
        Baseline::parse(flat).expect("the older form reads"),
        Baseline::new(None, counts(&[("known-typo", 39), ("unknown-word", 768)]))
    );
    let error = Baseline::parse("[counts]\nx = -1\n").expect_err("a count is not negative");
    assert_eq!(error.to_string(), "x: not a count");
}

#[test]
fn a_missing_baseline_is_none_and_not_an_error() {
    let missing = std::env::temp_dir().join("lspkit-ratchet-there-is-no-such-file.toml");
    assert!(Baseline::load(&missing).expect("not an error").is_none());
    assert_eq!(
        Outcome::Held,
        Baseline::default().compare(&Counts::new()).outcome()
    );
}

#[test]
fn a_baseline_round_trips_through_its_file_whatever_the_codes_hold() {
    let baseline = Baseline::new(
        Some(12),
        counts(&[
            ("plain", 1),
            ("quo\"te", 2),
            ("back\\slash", 3),
            ("e\u{301}", 4),
            ("tab\tand\nline", 5),
        ]),
    );
    let text = baseline.render("tool --accept\n# [counts]");
    assert_eq!(Baseline::parse(&text).expect("it reads back"), baseline);

    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("baseline.toml");
    baseline.store(&path, "tool --accept").expect("stored");
    assert_eq!(
        Baseline::load(&path).expect("it reads back"),
        Some(baseline)
    );
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .expect("a directory")
        .collect();
    assert_eq!(names.len(), 1, "no temporary file is left beside it");
}
