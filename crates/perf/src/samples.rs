use std::time::Duration;

/// Every timing collected for one kind of request, and where the worst one was.
#[derive(Debug, Clone)]
pub struct Samples {
    what: &'static str,
    durations: Vec<Duration>,
    /// How many produced an answer rather than a deterministic empty result. Latency means little
    /// without it: a server that answers nothing is fast, and 1 ms without an answer rate looks
    /// like success.
    answered: usize,
    worst: Option<Worst>,
}

/// The slowest sample, and where it was taken, so the slow case can be opened and looked at.
#[derive(Debug, Clone)]
pub struct Worst {
    pub duration: Duration,
    /// `path:line:col`, or whatever names the case for the server measuring it.
    pub location: String,
}

impl Samples {
    pub fn new(what: &'static str) -> Samples {
        Samples {
            what,
            durations: Vec::new(),
            answered: 0,
            worst: None,
        }
    }

    /// Records one timing, keeping the location only of the worst so far.
    pub fn record(&mut self, duration: Duration, answered: bool, location: &str) {
        self.durations.push(duration);
        if answered {
            self.answered += 1;
        }
        if self
            .worst
            .as_ref()
            .is_none_or(|worst| duration > worst.duration)
        {
            self.worst = Some(Worst {
                duration,
                location: location.to_owned(),
            });
        }
    }

    pub fn what(&self) -> &'static str {
        self.what
    }

    pub fn len(&self) -> usize {
        self.durations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.durations.is_empty()
    }

    pub fn answered(&self) -> usize {
        self.answered
    }

    /// The share that answered, in percent; 0 with no samples.
    #[expect(
        clippy::cast_precision_loss,
        reason = "a percentage for display, over at most a few thousand samples"
    )]
    pub fn answered_percent(&self) -> f64 {
        if self.durations.is_empty() {
            return 0.0;
        }
        self.answered as f64 * 100.0 / self.durations.len() as f64
    }

    pub fn worst(&self) -> Option<&Worst> {
        self.worst.as_ref()
    }

    /// The `percent`th percentile by nearest rank, or `None` with no samples.
    ///
    /// Nearest rank always returns a duration some request actually took. With a few thousand
    /// samples, the difference from interpolation is noise. A `percent` above 100 is clamped to
    /// 100, the maximum.
    pub fn percentile(&self, percent: u32) -> Option<Duration> {
        if self.durations.is_empty() {
            return None;
        }
        let mut sorted = self.durations.clone();
        sorted.sort_unstable();
        let rank = (sorted.len() as u64 * u64::from(percent.min(100)))
            .div_ceil(100)
            .max(1);
        sorted
            .get(usize::try_from(rank).unwrap_or(usize::MAX) - 1)
            .copied()
    }

    pub fn max(&self) -> Option<Duration> {
        self.durations.iter().max().copied()
    }
}

/// A duration in milliseconds, the unit every latency budget is written in: `12.00 ms`.
pub fn ms(duration: Duration) -> String {
    format!("{:.2} ms", duration.as_secs_f64() * 1000.0)
}
