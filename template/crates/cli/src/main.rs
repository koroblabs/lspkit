//! The `kitserver` binary: `serve` for an editor, `check [--ratchet] [--accept] <path>...` for a
//! person or CI, and `perf <path>...` for the budgets. `check` and `serve` run the same
//! `kitserver_check::check`, so the command line and the editor report a file the same way.

mod files;
mod perf;
mod ratchet;

/// Counts every allocation, takes large blocks from the operating system through a bounded cache,
/// and leaves small ones to the system allocator (see `lspkit_heap::Heap`). A freed parse buffer's
/// memory is then returned on every platform, which system allocators do not guarantee. On macOS,
/// lspkit-heap's `override` feature (in this crate's `Cargo.toml`) makes mimalloc the system
/// allocator for the whole process, so `release_freed` gives back what a parser written in C
/// freed as well; glibc does that by itself.
#[global_allocator]
static HEAP: lspkit_heap::Heap = lspkit_heap::Heap;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => serve(),
        Some("check") => check(&args[1..]),
        Some("perf") => perf::run(&args[1..]),
        _ => {
            eprintln!(
                "usage: kitserver serve | check [--ratchet] [--accept] <path>... | perf <path>..."
            );
            ExitCode::from(2)
        }
    }
}

/// Runs the server on its own runtime.
///
/// Workers get a 64 MiB stack because panic containment cannot catch a stack overflow. In phrost, a
/// 4 000-deep expression in an unopened file overflowed a tokio worker's default 2 MiB and aborted
/// the server. The stack is reserved address space, so its pages cost memory only once used.
fn serve() -> ExitCode {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(64 * 1024 * 1024)
        .build();
    match runtime {
        Ok(runtime) => runtime.block_on(kitserver_lsp::serve()),
        Err(err) => {
            eprintln!("kitserver: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Prints every finding under `paths`, one per line, and exits with 1 if there were any. With
/// `--ratchet`, it exits with 1 only if a code's count rose above the project's baseline.
fn check(args: &[String]) -> ExitCode {
    let ratchet = args.iter().any(|arg| arg == "--ratchet");
    let accept = args.iter().any(|arg| arg == "--accept");
    let paths: Vec<String> = args
        .iter()
        .filter(|arg| !arg.starts_with("--"))
        .cloned()
        .collect();
    let Some(project) = paths.first() else {
        eprintln!("usage: kitserver check [--ratchet] [--accept] <path>...");
        return ExitCode::from(2);
    };

    let files = files::under(&paths);
    let mut counts = lspkit_ratchet::Counts::new();
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        for finding in kitserver_check::check(&text) {
            *counts.entry(finding.code.to_owned()).or_default() += 1;
            // The newlines before the finding, plus one. Counting `lines()` of the text before it
            // would be one short for a finding at the start of a line.
            let line = text[..usize::from(finding.range.start())]
                .matches('\n')
                .count()
                + 1;
            println!(
                "{}:{line}: {} {}",
                file.display(),
                finding.code,
                finding.message
            );
        }
    }
    if ratchet || accept {
        return ratchet::run(std::path::Path::new(project), files.len(), counts, accept);
    }
    if counts.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
