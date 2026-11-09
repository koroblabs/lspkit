//! The process's memory, as the operating system charges it.
//!
//! It is read from the system instead of summed from a server's own structures, because a memory
//! budget is about what the machine gives up to run the server. That includes allocator overhead
//! and fragmentation, which a sum of `size_of` misses. It is the figure the system itself acts on:
//! `VmRSS` on Linux, and on macOS the physical footprint rather than the resident set, which there
//! still counts pages the allocator has already given back (see [`current`]).

/// Bytes this process costs the machine, or `None` if the platform did not say.
///
/// `VmRSS` from `/proc/self/status` on Linux, the physical footprint from the `footprint` tool on
/// macOS, and `ps`'s resident set elsewhere, so it is `None` on Windows and wherever the tool is
/// missing or its output does not parse. A memory [`Budget`](crate::Budget) judged against `None`
/// is "not measured", neither a pass nor a failure.
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

    #[cfg(target_os = "macos")]
    {
        // The physical footprint: what macOS charges the process and what Activity Monitor shows.
        // The resident set `ps` reports also counts pages an allocator gave back with
        // `MADV_FREE_REUSABLE`, which the kernel treats as free and takes when it needs them.
        // After mimalloc's `mi_collect`, a process at 150 MB of RSS had a footprint of 47 MB, and
        // the kernel's own accounting showed the other 101 MB as reusable: `rss ≈ footprint +
        // reusable` held in every sample (lspkit, macOS 27, 2026-11-09). Measuring RSS there would
        // call memory returned not returned. `ps` stands in if the tool is missing or has changed.
        footprint().or_else(ps_rss)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        ps_rss()
    }
}

/// `ps`'s resident set. No /proc, and the task-info API needs `unsafe`, which the servers forbid.
/// One subprocess per report is an acceptable cost for a tool.
#[cfg(not(target_os = "linux"))]
fn ps_rss() -> Option<u64> {
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

/// The physical footprint from `/usr/bin/footprint`, a system tool that reads the kernel's
/// accounting for the caller's own process without privileges, in about 20 ms.
#[cfg(target_os = "macos")]
fn footprint() -> Option<u64> {
    let output = std::process::Command::new("footprint")
        .args(["--format", "bytes", &std::process::id().to_string()])
        .output()
        .ok()?;
    parse_footprint(&String::from_utf8_lossy(&output.stdout))
}

/// The footprint in the tool's report, whose first line reads
/// `phrost [4242]: 64-bit    Footprint: 248512512 B (16384 bytes per page)` when bytes are asked
/// for. The formatted units are read too, in case the tool stops taking the format.
#[cfg(any(test, target_os = "macos"))]
pub(crate) fn parse_footprint(report: &str) -> Option<u64> {
    let (_, after) = report.split_once("Footprint:")?;
    let mut words = after.split_whitespace();
    let amount = words.next()?;
    let unit: u64 = match words.next()? {
        "B" => 1,
        "KB" => 1 << 10,
        "MB" => 1 << 20,
        "GB" => 1 << 30,
        _ => return None,
    };
    // Integer arithmetic, so a fraction such as `237.5 MB` is exact and nothing is cast.
    let (whole, fraction) = amount.split_once('.').unwrap_or((amount, ""));
    let whole: u64 = whole.parse().ok()?;
    let scale = 10u64.checked_pow(u32::try_from(fraction.len()).ok()?)?;
    let fraction: u64 = if fraction.is_empty() {
        0
    } else {
        fraction.parse().ok()?
    };
    Some(whole.checked_mul(unit)? + fraction.checked_mul(unit)? / scale)
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
