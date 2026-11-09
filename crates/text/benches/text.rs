//! Times the hot paths of lspkit-text: building an index, applying one edit, converting columns.
//!
//! `cargo bench -p lspkit-text --features lsp [-- <filter>]`. Each figure is the best of seven
//! rounds, in microseconds per call, with the median of the seven beside it. Run it before and
//! after a change, on an idle machine. A median well above the best says the run was disturbed:
//! a build on the machine, or the thread on an efficiency core, where the same code read 60 %
//! slower (199 µs against 325 on an M2 Max); compare best figures from runs whose median is close.
use std::{hint::black_box, time::Instant};

use ls_types::{Position, Range, TextDocumentContentChangeEvent};
use lspkit_text::{Encoding, LineIndex, TextSize, lsp};

fn main() {
    // `cargo bench` passes `--bench` to the binary; a word after `--` is the filter.
    let filter = std::env::args()
        .skip(1)
        .find(|arg| !arg.starts_with("--"))
        .unwrap_or_default();
    let bench = |name: &str, iterations: usize, work: &mut dyn FnMut()| {
        if !name.contains(&filter) {
            return;
        }
        let mut rounds = [0.0f64; 7];
        for round in &mut rounds {
            let started = Instant::now();
            for _ in 0..iterations {
                work();
            }
            *round = started.elapsed().as_secs_f64() * 1e6 / iterations as f64;
        }
        rounds.sort_by(f64::total_cmp);
        println!(
            "{name:<60} {:>10.3} µs   median {:>10.3}",
            rounds[0], rounds[3]
        );
    };

    let lf = code(25_000, "\n");
    let crlf = code(25_000, "\r\n");
    let cyrillic = comments(12_000);
    let minified: String = "abcdefgh(){};".repeat(80_000);
    for (name, text) in [
        ("LineIndex::new, 1.4 MB of code, \\n", &lf),
        ("LineIndex::new, 1.4 MB of code, \\r\\n", &crlf),
        ("LineIndex::new, 1.3 MB of Cyrillic comments", &cyrillic),
        ("LineIndex::new, 1 MB minified on one line", &minified),
    ] {
        bench(name, 20, &mut || {
            black_box(LineIndex::new(black_box(text)));
        });
    }

    // One edit as an editor sends it: a character inserted, then deleted, so the text is as it
    // was after each pair.
    for (name, text, line) in [
        ("apply insert+delete at line 10 of 1.4 MB", &lf, 10u32),
        ("apply insert+delete at line 12 500 of 1.4 MB", &lf, 12_500),
        (
            "apply insert+delete at the last line of 1.4 MB",
            &lf,
            24_999,
        ),
        (
            "apply insert+delete at line 12 500 of 1.4 MB, \\r\\n",
            &crlf,
            12_500,
        ),
    ] {
        let mut edited = text.clone();
        bench(name, 200, &mut || {
            lsp::apply(&mut edited, vec![change(line, 4, 4, "x")], Encoding::Utf16).unwrap();
            lsp::apply(&mut edited, vec![change(line, 4, 5, "")], Encoding::Utf16).unwrap();
        });
        assert_eq!(&edited, text);
    }

    // Column conversion on a non-ASCII line, near the end of 2 KB of Cyrillic.
    let long = "Привіт світе, це довгий рядок з кирилицею та emoji 👋 ".repeat(20) + "END\n";
    let index = LineIndex::new(&long);
    let end = TextSize::try_from(long.find("END").unwrap()).unwrap();
    for encoding in [Encoding::Utf16, Encoding::Utf32] {
        bench(
            &format!("line_col_in {encoding:?}, byte 1 840 of a 2 KB Cyrillic line"),
            2000,
            &mut || {
                black_box(index.line_col_in(&long, end, encoding));
            },
        );
        let pos = index.line_col_in(&long, end, encoding).unwrap();
        bench(
            &format!("offset_in {encoding:?}, back to byte 1 840"),
            2000,
            &mut || {
                black_box(index.offset_in(&long, pos, encoding));
            },
        );
    }
    // The ASCII shortcuts, deep in a minified line.
    let index = LineIndex::new(&minified);
    let deep = TextSize::from(500_000);
    bench(
        "line_col_in Utf16, byte 500 000 of a minified line",
        200,
        &mut || {
            black_box(index.line_col_in(&minified, deep, Encoding::Utf16));
        },
    );
    let pos = index.line_col_in(&minified, deep, Encoding::Utf16).unwrap();
    bench("offset_in Utf16, back to byte 500 000", 200, &mut || {
        black_box(index.offset_in(&minified, pos, Encoding::Utf16));
    });
    // Byte columns, the lookup inside a server.
    let index = LineIndex::new(&lf);
    let offsets: Vec<TextSize> = (0..1000u32)
        .map(|i| TextSize::from((i * 1013) % lf.len() as u32))
        .collect();
    bench("line_col, 1 000 lookups", 50, &mut || {
        for &offset in &offsets {
            black_box(index.line_col(offset));
        }
    });
}

fn code(lines: usize, terminator: &str) -> String {
    let mut text = String::new();
    for i in 0..lines {
        text.push_str(&format!(
            "    let value_{i} = compute(alpha, beta, gamma) + {i};{terminator}"
        ));
    }
    text
}

fn comments(lines: usize) -> String {
    let mut text = String::new();
    for i in 0..lines {
        text.push_str(&format!(
            "    // Привіт, світе! Рядок номер {i} із кирилицею та emoji 👋 and an ascii tail\n"
        ));
    }
    text
}

fn change(line: u32, from: u32, to: u32, text: &str) -> TextDocumentContentChangeEvent {
    TextDocumentContentChangeEvent {
        range: Some(Range::new(
            Position::new(line, from),
            Position::new(line, to),
        )),
        range_length: None,
        text: text.to_owned(),
    }
}
