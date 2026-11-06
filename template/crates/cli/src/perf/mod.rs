//! `kitserver perf <path>...`: measures every budget in [`budget`] on real files and exits with 1
//! when one is over. Run it on an idle machine with a release build. A concurrent `cargo build`
//! looks exactly like a regression.

mod budget;

use std::{process::ExitCode, time::Instant};

use lspkit_perf::{Samples, Verdict, ms, rss};

use self::budget::BUDGETS;

pub(crate) fn run(paths: &[String]) -> ExitCode {
    let files = crate::files::under(paths);
    if files.is_empty() {
        eprintln!("usage: kitserver perf <path>...");
        return ExitCode::from(2);
    }

    // Only the check is timed. Each file is read, checked and dropped, because a server holds only
    // the open documents, not the whole project.
    let mut checks = Samples::new("check per file");
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let started = Instant::now();
        let found = kitserver_check::check(&text);
        checks.record(
            started.elapsed(),
            !found.is_empty(),
            &file.display().to_string(),
        );
    }

    let measured = [
        checks.percentile(95).map(|p95| p95.as_secs_f64() * 1000.0),
        rss::current().map(rss::megabytes),
    ];
    println!("kitserver perf: {} files", checks.len());
    let mut verdict = Verdict::new();
    for (budget, value) in BUDGETS.iter().zip(measured) {
        let judged = verdict.judge(budget, value);
        let value = value.map_or_else(
            || "-".to_owned(),
            |value| format!("{value:.2} {}", budget.unit),
        );
        println!(
            "  {:<24} {:>12}  budget {:>10}  {}",
            budget.what,
            value,
            budget.limit_text(),
            judged.word()
        );
    }
    if let Some(worst) = checks.worst() {
        println!("  worst check {} at {}", ms(worst.duration), worst.location);
    }

    match verdict.into_result() {
        Ok(()) => ExitCode::SUCCESS,
        Err(over) => {
            eprintln!("kitserver perf: {over}");
            ExitCode::from(1)
        }
    }
}
