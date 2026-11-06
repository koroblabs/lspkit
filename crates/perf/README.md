# lspkit-perf

The arithmetic behind a language server's `perf` command: timings with nearest-rank percentiles, the
process's resident memory, and budgets with a verdict that fails the run. The server decides what to
measure and what each budget is. Sharing the arithmetic means every server computes a p95 the same
way.

## Example

```rust
use std::time::Duration;
use lspkit_perf::{Budget, Judged, Samples, Verdict};

let mut hover = Samples::new("hover");
for millis in 1..=20u64 {
    hover.record(Duration::from_millis(millis), true, &format!("a.php:{millis}"));
}
let p95 = hover.percentile(95).expect("samples were taken");
assert_eq!(p95, Duration::from_millis(19));

let mut verdict = Verdict::new();
let budget = Budget::ms("hover p95", 12.0);
let judged = verdict.judge(&budget, Some(p95.as_secs_f64() * 1000.0));
assert_eq!(judged, Judged::Over);
// A `perf` command returns this, so an exceeded budget is a non-zero exit.
let over = verdict.into_result().unwrap_err();
assert_eq!(over.to_string(), "over budget: hover p95");
```

## Notes

- Percentiles use nearest rank, so the result is always a duration some request took. phrost used
  nearest rank and orthra rounded an interpolated index, so the same timings gave two different
  p95s.
- `Samples` also counts how many requests answered and records where the worst one was.
- A budget for something the server does not do yet is listed as `unimplemented` instead of left
  out, since a report covering three of six rows reads as if all six were within budget.
- `rss::current` reads the process's resident memory as the operating system counts it, or returns
  `None` where it cannot.

Part of [lspkit](../../README.md), a toolkit for language servers.

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or
[MIT license](../../LICENSE-MIT) at your option.
