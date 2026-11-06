use std::collections::HashSet;

use crate::reported::Reported;

#[test]
fn only_a_file_that_has_problems_or_had_them_is_worth_sending() {
    let mut reported = Reported::default();
    assert!(reported.note(&"a.php", true), "problems are sent");
    assert!(
        !reported.note(&"b.php", false),
        "a clean file nobody was told about is not"
    );
    assert!(
        reported.note(&"a.php", true),
        "problems again are sent again"
    );
    assert!(
        reported.note(&"a.php", false),
        "fixed: the client still shows the old ones"
    );
    assert!(
        !reported.note(&"a.php", false),
        "and once told, it is not told twice"
    );
}

#[test]
fn a_whole_pass_clears_what_it_no_longer_sees_but_not_what_was_opened() {
    let mut reported = Reported::default();
    for file in ["kept.php", "deleted.php", "opened.php"] {
        reported.note(&file, true);
    }
    reported.forget(&"opened.php");

    let seen: HashSet<&str> = ["kept.php"].into();
    assert_eq!(reported.unseen(&seen), ["deleted.php"]);
    assert!(reported.contains(&"kept.php"));
    assert!(!reported.contains(&"deleted.php"));
    assert!(reported.unseen(&seen).is_empty(), "told once");
}
