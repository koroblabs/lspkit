//! The process's resident memory, as the operating system counts it.
//!
//! It is read from the system instead of summed from a server's own structures, because a memory
//! budget is about what the machine gives up to run the server. That includes allocator overhead
//! and fragmentation, which a sum of `size_of` misses.

/// Resident bytes of this process, or `None` if the platform did not say.
///
/// Read from `/proc/self/status` on Linux and from `ps` elsewhere, so it is `None` on Windows,
/// and wherever `/proc` or `ps` is missing or its output does not parse. A memory
/// [`Budget`](crate::Budget) judged against `None` is "not measured", neither a pass nor a failure.
pub fn current() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        // `VmRSS` instead of `statm`: it states its unit, while `statm` needs the page size, which
        // takes `unsafe` or a dependency.
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let kib = status
            .lines()
            .find(|line| line.starts_with("VmRSS:"))?
            .split_whitespace()
            .nth(1)?
            .parse::<u64>()
            .ok()?;
        Some(kib * 1024)
    }

    #[cfg(not(target_os = "linux"))]
    {
        // No /proc, and the task-info API needs `unsafe`, which the servers forbid. One subprocess
        // per report is an acceptable cost for a tool.
        let output = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        let kib = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u64>()
            .ok()?;
        Some(kib * 1024)
    }
}

/// `bytes` in the unit memory budgets use: MiB (2^20 bytes), labeled "MB" because consumers'
/// snapshots already print that label.
#[expect(clippy::cast_precision_loss, reason = "a size for display and budgets")]
pub fn megabytes(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// Bytes the way a person reads them: `237.1 MB` (MiB, as [`megabytes`]).
pub fn format(bytes: u64) -> String {
    format!("{:.1} MB", megabytes(bytes))
}
