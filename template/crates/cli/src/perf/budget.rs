//! The performance budgets. This file is their source of truth; `CLAUDE.md` quotes it.
//!
//! `kitserver perf` measures each one on real files, and a change must stay under it. Both were set
//! before anything was measured. Lower one when measurements sit far below it, so a regression
//! shows before it reaches users.

use lspkit_perf::Budget;

pub(crate) const BUDGETS: &[Budget] = &[
    // p95 time to check one file from its text. The check runs each time typing pauses.
    Budget::ms("check per file", 5.0),
    // Resident memory of the whole process after the run, since that is what the machine pays.
    Budget::mb("RSS after the run", 50.0),
];
