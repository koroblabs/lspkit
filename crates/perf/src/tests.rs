use std::time::Duration;

use expect_test::expect;

use crate::{Budget, Judged, Samples, Verdict, rss};

fn millis(values: &[u64]) -> Samples {
    let mut samples = Samples::new("hover");
    for (at, value) in values.iter().enumerate() {
        samples.record(
            Duration::from_millis(*value),
            *value % 2 == 0,
            &format!("a.php:{at}"),
        );
    }
    samples
}

#[test]
fn a_percentile_is_a_duration_some_request_actually_took() {
    // Twenty samples, 1..=20 ms. Nearest rank puts p95 at the 19th and p50 at the 10th. Rounding
    // an interpolated index, as orthra did, puts p50 at the 11th.
    let samples = millis(&(1..=20).rev().collect::<Vec<_>>());
    let at = |percent| samples.percentile(percent).map(|d| d.as_millis());
    assert_eq!(at(95), Some(19));
    assert_eq!(at(50), Some(10));
    assert_eq!(at(100), Some(20));
    assert_eq!(at(0), Some(1));
    assert_eq!(samples.max().map(|d| d.as_millis()), Some(20));
    assert_eq!(Samples::new("none").percentile(95), None);
}

#[test]
fn the_worst_keeps_its_location_and_the_answer_rate_is_counted() {
    let samples = millis(&[3, 9, 4, 9, 2]);
    let worst = samples.worst().expect("a worst sample");
    assert_eq!(worst.location, "a.php:1", "the first of two equal worst");
    assert_eq!(samples.answered(), 2);
    assert!((samples.answered_percent() - 40.0).abs() < f64::EPSILON);
}

#[test]
fn a_verdict_names_every_budget_over_and_fails_the_run() {
    let budgets = [
        Budget::ms("hover", 12.0),
        Budget::ms("diagnostics", 30.0),
        Budget::mb("RSS", 40.0),
        Budget::ms("rename", 12.0),
        Budget::ms("formatting", 12.0).unimplemented(),
    ];
    let measured = [Some(0.2), Some(31.5), Some(f64::NAN), None, None];
    let mut verdict = Verdict::new();
    let words: Vec<String> = budgets
        .iter()
        .zip(measured)
        .map(|(budget, value)| {
            let judged = verdict.judge(budget, value);
            format!("{} {} {}", budget.what, budget.limit_text(), judged.word())
        })
        .collect();
    expect![[r#"
        hover 12.00 ms ok
        diagnostics 30.00 ms OVER BUDGET
        RSS 40.00 MB OVER BUDGET
        rename 12.00 ms not measured
        formatting 12.00 ms not implemented"#]]
    .assert_eq(&words.join("\n"));
    assert_eq!(
        verdict.into_result().map_err(|over| over.to_string()),
        Err("over budget: diagnostics, RSS".to_owned())
    );
    assert_eq!(
        Verdict::new().judge(&budgets[0], Some(12.0)),
        Judged::Ok,
        "the limit itself holds"
    );
}

#[test]
fn this_process_has_a_resident_size() {
    let bytes = rss::current().expect("the platform says");
    assert!(bytes > 0);
    assert!(rss::format(bytes).ends_with(" MB"));
}
