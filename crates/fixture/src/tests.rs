//! Tests of the fixture format. A marker one byte off would make hundreds of snapshots agree on the
//! wrong position, so the format's edge cases are pinned here.

use expect_test::expect;

use crate::{Fixture, TextSize};

/// Every file, its flags, its markers and its text, as one string.
fn render(fixture: &Fixture) -> String {
    fixture
        .files()
        .iter()
        .map(|file| {
            let markers: Vec<String> = fixture
                .markers()
                .iter()
                .filter(|(_, marker)| marker.path == file.path())
                .map(|(name, marker)| format!("{name}@{}", u32::from(marker.offset)))
                .collect();
            format!(
                "{} flags={:?} verbatim={} markers={markers:?}\n{}",
                file.path(),
                file.flags(),
                file.is_verbatim(),
                file.text()
            )
        })
        .collect::<Vec<_>>()
        .join("---\n")
}

#[test]
fn files_flags_markers_and_text() {
    let fixture = Fixture::parse(
        r#"
        //- /orthra.toml
        languages = ["en-US", "uk-UA"]

        //- /run.sh  raw  host=shell
        echo "$0 @a@b"

        //- /src/Use.php  open  php:8.5
        <?php
        function go(Repo $r) {
            $u = $r->fi@member@nd(1);
        }

        //- /src/lib.rs
        /// Retruns the fi$0rst @word@item.
        /// Mail a@b.com or c@d.org; @@ is not a marker.


        "#,
    );
    expect![[r#"
        /orthra.toml flags=[] verbatim=false markers=[]
        languages = ["en-US", "uk-UA"]
        ---
        /run.sh flags=["host=shell"] verbatim=true markers=[]
        echo "$0 @a@b"
        ---
        /src/Use.php flags=["open", "php:8.5"] verbatim=false markers=["member@44"]
        <?php
        function go(Repo $r) {
            $u = $r->find(1);
        }
        ---
        /src/lib.rs flags=[] verbatim=false markers=["0@18", "word@22"]
        /// Retruns the first item.
        /// Mail a@b.com or c@d.org; @@ is not a marker.
    "#]]
    .assert_eq(&render(&fixture));

    assert_eq!(fixture.subject().path(), "/src/lib.rs");
    let member = fixture.markers().position("member");
    let text = fixture.file("/src/Use.php").text();
    assert_eq!(&text[usize::from(member.offset)..][..5], "nd(1)");
    let cursor = fixture.markers().cursor();
    assert_eq!(cursor.path, "/src/lib.rs");
    assert_eq!(
        &fixture.subject().text()[usize::from(cursor.offset)..][..4],
        "rst "
    );
    fixture.markers().position("word");
    fixture.markers().assert_all_used();
}

#[test]
fn variables_and_interpolation_are_not_markers() {
    // Named markers use `@` because `$name$` read `"$a$b"` as a marker called `a`. `$` is a marker
    // only before `0`, which no PHP or shell variable starts with.
    let fixture = Fixture::parse(
        r#"
//- /a.php
<?php
$a = $b;
echo "$a$b";
echo "{$a}{$b}";
$obj->prop = @$x;
"#,
    );
    expect![[r#"
        <?php
        $a = $b;
        echo "$a$b";
        echo "{$a}{$b}";
        $obj->prop = @$x;
    "#]]
    .assert_eq(fixture.subject().text());
    assert!(fixture.markers().is_empty());
}

#[test]
fn a_marker_can_sit_inside_a_name() {
    let fixture = Fixture::parse("//- /a.php\n$ca@prop@che = 1;\n$loc$0al = 2;\n");
    let text = fixture.subject().text();
    assert_eq!(text, "$cache = 1;\n$local = 2;\n");
    let prop = fixture.markers().position("prop");
    assert_eq!(&text[usize::from(prop.offset)..][..3], "che");
    let cursor = fixture.markers().cursor();
    assert_eq!(&text[usize::from(cursor.offset)..][..2], "al");
}

#[test]
fn a_backslash_makes_a_sigil_text() {
    let fixture = Fixture::parse(
        r#"
//- /a.php
$sink = "\$0 and \@name@";
/** {\@inherit@tag@doc} */
 * \@@here@
"#,
    );
    expect![[r#"
        $sink = "$0 and @name@";
        /** {@inheritdoc} */
         * @
    "#]]
    .assert_eq(fixture.subject().text());
    expect![[r#"["here@50", "tag@38"]"#]].assert_eq(&format!(
        "{:?}",
        fixture
            .markers()
            .iter()
            .map(|(name, marker)| format!("{name}@{}", u32::from(marker.offset)))
            .collect::<Vec<_>>()
    ));
}

#[test]
fn a_flag_the_server_names_leaves_the_text_as_written() {
    let fixture = Fixture::parse_with(
        "//- /a.md  expect\nfoo @a@ $0\n//- /b.md\nfoo\n",
        &["expect"],
    );
    let file = fixture.file("/a.md");
    assert!(file.is_verbatim());
    assert_eq!(file.flags(), ["expect"]);
    assert_eq!(file.text(), "foo @a@ $0\n");
    assert!(fixture.markers().is_empty());
    assert!(!fixture.file("/b.md").is_verbatim());
}

#[test]
fn indentation_and_trailing_lines_are_layout() {
    let fixture = Fixture::parse(
        "
            //- /a.txt
            one
                two

            //- /b.txt
            //- /c.txt
            three",
    );
    let texts: Vec<&str> = fixture.files().iter().map(|file| file.text()).collect();
    assert_eq!(texts, ["one\n    two\n", "", "three\n"]);
}

#[test]
fn each_clone_keeps_its_own_account_of_what_was_asked() {
    // The open-versus-closed harness runs one fixture twice. The first run must not satisfy the
    // second run's check, or a dimension could stop asking anything and still pass.
    let fixture = Fixture::parse("//- /a.php\n$x->fi@one@nd();\n");
    let first = fixture.markers().clone();
    first.position("one");
    first.assert_all_used();

    let second = fixture.markers().clone();
    let result = std::panic::catch_unwind(move || second.assert_all_used());
    assert!(
        result.is_err(),
        "a fresh clone should not inherit what was asked"
    );
}

#[test]
fn a_harness_can_spell_paths_its_own_way() {
    let (_, markers) = Fixture::parse("//- /src/a.php\nfi$0nd\n").into_parts();
    let markers = markers.map_paths(|path| path.trim_start_matches('/').to_owned());
    assert_eq!(markers.cursor().path, "src/a.php");
    assert_eq!(markers.cursor().offset, TextSize::from(2));
}

#[test]
#[should_panic(expected = "fixture declares unused marker(s): `stray`")]
fn a_marker_nobody_asked_about_fails_by_name() {
    let fixture = Fixture::parse("//- /a.txt\nfoo@stray@bar $0\n");
    fixture.markers().cursor();
    fixture.markers().assert_all_used();
}

#[test]
#[should_panic(expected = "fixture has no marker `typo`; it declares `real`")]
fn asking_for_an_undeclared_marker_names_the_declared_ones() {
    Fixture::parse("//- /a.txt\nre@real@al\n")
        .markers()
        .position("typo");
}

#[test]
#[should_panic(expected = "fixture has two markers named `x`")]
fn a_marker_name_is_unique_across_files() {
    Fixture::parse("//- /a.txt\n@x@a\n//- /b.txt\n@x@b\n");
}

#[test]
#[should_panic(expected = "fixture has two files at /a.txt")]
fn a_duplicate_path_is_a_panic() {
    Fixture::parse("//- /a.txt\none\n//- /a.txt\ntwo\n");
}

#[test]
#[should_panic(expected = "fixture content must start with a `//- /path` header")]
fn content_before_the_first_header_is_a_panic() {
    Fixture::parse("stray\n//- /a.txt\none\n");
}

#[test]
#[should_panic(expected = "fixture paths start at the root, with the slash: \"a.txt\"")]
fn a_header_without_the_slash_is_a_panic() {
    Fixture::parse("//- a.txt\none\n");
}

#[test]
#[should_panic(expected = "fixture paths start at the root, with the slash: \"a.txt\"")]
fn a_lookup_spells_the_path_the_way_the_header_does() {
    Fixture::parse("//- /a.txt\none\n").file("a.txt");
}
