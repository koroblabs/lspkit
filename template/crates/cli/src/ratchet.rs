//! `kitserver check --ratchet`: compares finding counts per code against a baseline that may only
//! go down. `--accept` writes a new baseline. `lspkit_ratchet` does the counting, the comparison
//! and the file format. kitserver picks the file name, and writes a lower count back only when
//! `--accept` is given.

use std::{path::Path, process::ExitCode};

use lspkit_ratchet::{Baseline, Counts, Outcome};

const FILE: &str = "kitserver-baseline.toml";
const COMMAND: &str = "kitserver check --ratchet --accept";

/// Where the baseline lives: inside `path` if it is a directory, else next to it.
fn baseline_beside(path: &Path) -> std::path::PathBuf {
    let dir = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(Path::new("."))
    };
    dir.join(FILE)
}

pub(crate) fn run(project: &Path, files: usize, found: Counts, accept: bool) -> ExitCode {
    let path = baseline_beside(project);
    if accept {
        return match Baseline::new(Some(files), found).store(&path, COMMAND) {
            Ok(()) => {
                println!("baseline written to {}", path.display());
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("kitserver: {err}");
                ExitCode::from(2)
            }
        };
    }
    let agreed = match Baseline::load(&path) {
        Ok(Some(agreed)) => agreed,
        Ok(None) => {
            eprintln!(
                "kitserver: no baseline at {} -- write one with --accept",
                path.display()
            );
            return ExitCode::from(2);
        }
        Err(err) => {
            eprintln!("kitserver: {err}");
            return ExitCode::from(2);
        }
    };
    let comparison = agreed.compare(&found);
    print!("{}", comparison.report());
    match comparison.outcome() {
        Outcome::Held => {
            println!("ratchet held");
            ExitCode::SUCCESS
        }
        Outcome::Fell => {
            println!("ratchet held, and a count fell: --accept holds the new numbers");
            ExitCode::SUCCESS
        }
        Outcome::Rose => {
            println!(
                "ratchet broken: fix the findings, or --accept them and say why in the commit"
            );
            ExitCode::from(1)
        }
    }
}
