use expect_test::expect;
use lspkit_fixture::Fixture;

use crate::check;

/// Renders every finding in the fixture's subject, one per line. The snapshot covers the whole
/// result, so an unexpected finding fails the test as a missing one does.
fn render(source: &str) -> String {
    let fixture = Fixture::parse(source);
    let text = fixture.subject().text();
    let rendered = check(text)
        .into_iter()
        .map(|finding| {
            let start = u32::from(finding.range.start());
            let end = u32::from(finding.range.end());
            format!("{}..{end} {} {}\n", start, finding.code, finding.message)
        })
        .collect();
    fixture.markers().assert_all_used();
    rendered
}

#[test]
fn whitespace_at_the_end_of_a_line_is_found_and_nothing_else() {
    expect![[r#"
        3..6 trailing-whitespace whitespace at the end of a line
        16..17 trailing-whitespace whitespace at the end of a line
    "#]]
    .assert_eq(&render("//- /a.txt\none \t \ntwo\nthree\t\r\nfour"));
}

#[test]
fn a_finding_starts_where_the_whitespace_does() {
    let fixture = Fixture::parse("//- /a.txt\nkept@here@  \n");
    let found = check(fixture.subject().text());
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].range.start(),
        fixture.markers().position("here").offset
    );
    fixture.markers().assert_all_used();
}

/// Uses plain text, not a fixture: a fixture treats a whitespace-only line as layout and empties
/// it, so this case cannot be written as one.
#[test]
fn a_line_of_nothing_but_whitespace_is_one_finding_from_its_start() {
    let found: Vec<_> = check("one\n  \ntwo\n")
        .into_iter()
        .map(|finding| finding.range)
        .collect();
    assert_eq!(format!("{found:?}"), "[4..6]");
}
