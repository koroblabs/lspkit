/// A ceiling one measurement must stay under, in the unit it is written in.
///
/// Budgets are set before anything is measured. Lower one that measurements sit far below, so a
/// regression shows before it reaches users.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    pub what: &'static str,
    pub limit: f64,
    pub unit: &'static str,
    /// Whether the server does this yet. A budget for something it does not do is still listed,
    /// because a report covering three of six rows reads as if all six were within budget.
    pub implemented: bool,
}

impl Budget {
    /// A latency, in milliseconds.
    pub const fn ms(what: &'static str, limit: f64) -> Budget {
        Budget {
            what,
            limit,
            unit: "ms",
            implemented: true,
        }
    }

    /// A size in MiB (2^20 bytes), as [`crate::rss::megabytes`] counts them, labeled "MB" because
    /// consumers already snapshot that label.
    pub const fn mb(what: &'static str, limit: f64) -> Budget {
        Budget {
            what,
            limit,
            unit: "MB",
            implemented: true,
        }
    }

    /// Marks the budget as one for something the server does not do yet.
    pub const fn unimplemented(self) -> Budget {
        Budget {
            implemented: false,
            ..self
        }
    }

    /// The limit as a report prints it: `12.00 ms`.
    pub fn limit_text(&self) -> String {
        format!("{:.2} {}", self.limit, self.unit)
    }
}

/// What one row of a report came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Judged {
    Ok,
    Over,
    /// Implemented, and the sweep took no sample of it.
    NotMeasured,
    NotImplemented,
}

impl Judged {
    /// The word a report prints in its verdict column.
    pub fn word(self) -> &'static str {
        match self {
            Judged::Ok => "ok",
            Judged::Over => "OVER BUDGET",
            Judged::NotMeasured => "not measured",
            Judged::NotImplemented => "not implemented",
        }
    }
}

/// Every budget judged in one run, and whether the run passes.
#[derive(Debug, Default)]
pub struct Verdict {
    over: Vec<&'static str>,
}

impl Verdict {
    pub fn new() -> Verdict {
        Verdict::default()
    }

    /// Judges one measurement against its budget, remembering it if it is over.
    ///
    /// `None` means no sample was taken, as for a memory budget where [`crate::rss::current`]
    /// returned `None`. An implemented budget judged against it is [`Judged::NotMeasured`], which
    /// does not fail the run. An unimplemented budget is [`Judged::NotImplemented`] whatever was
    /// measured, and a NaN measurement is over.
    pub fn judge(&mut self, budget: &Budget, measured: Option<f64>) -> Judged {
        let judged = match measured {
            _ if !budget.implemented => Judged::NotImplemented,
            None => Judged::NotMeasured,
            Some(value) if value.is_nan() || value > budget.limit => Judged::Over,
            Some(_) => Judged::Ok,
        };
        if judged == Judged::Over {
            self.over.push(budget.what);
        }
        judged
    }

    /// The budgets that were exceeded, in the order they were judged.
    pub fn over(&self) -> &[&'static str] {
        &self.over
    }

    /// `Ok` when every budget held, or the ones that did not. A `perf` command returns this, so an
    /// exceeded budget becomes a non-zero exit.
    pub fn into_result(self) -> Result<(), OverBudget> {
        if self.over.is_empty() {
            return Ok(());
        }
        Err(OverBudget { over: self.over })
    }
}

/// The budgets a run went over, in the order they were judged. Prints as `over budget: hover p95`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverBudget {
    pub over: Vec<&'static str>,
}

impl std::fmt::Display for OverBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "over budget: {}", self.over.join(", "))
    }
}

impl std::error::Error for OverBudget {}

/// The message, for a command whose errors are strings.
impl From<OverBudget> for String {
    fn from(over: OverBudget) -> String {
        over.to_string()
    }
}
