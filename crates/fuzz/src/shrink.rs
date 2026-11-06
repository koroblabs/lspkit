//! Shrinks a failing list of edits to the fewest that still fail.
//!
//! The search is greedy. It first drops each other file's edits at once, then tries dropping each
//! remaining edit in turn, keeping the drop if the failure remains. Later edits to the same file
//! were made against a text that had the dropped edit, so their positions are mapped to where they
//! land without it, carried past each edit in between as an editor carries its own positions. A
//! later edit inside what the dropped edit inserted has no such position, so that drop is skipped.

use std::fmt::Display;

use crate::{Edit, Files, Step, Subject, TextRange, TextSize};

pub(crate) fn fewest<L, R, O, E, A, S>(
    files: &Files,
    failing: &[Step],
    subject: &mut Subject<O, E, A, S>,
) -> Vec<Step>
where
    R: PartialEq + Display,
    O: FnMut(&Files) -> L,
    E: FnMut(&mut L, &str, &Edit, &str),
    A: FnMut(&mut L) -> R,
    S: FnMut(&Files) -> R,
{
    let mut steps = failing.to_vec();
    // Drop each other file's edits at once. Edits to different files do not move each other, so no
    // positions need mapping, and they are usually most of a random list.
    let failing_path = failing.last().map(|step| step.path.clone());
    let others: Vec<String> = steps
        .iter()
        .map(|step| step.path.clone())
        .filter(|path| Some(path) != failing_path.as_ref())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    for path in others {
        let candidate: Vec<Step> = steps
            .iter()
            .filter(|step| step.path != path)
            .cloned()
            .collect();
        if let Some(failure) = subject.replay(files, &candidate) {
            steps = candidate[..=failure.step].to_vec();
        }
    }
    let mut at = 0;
    while at < steps.len() {
        let candidate = without(&steps, at);
        match candidate.and_then(|candidate| {
            subject
                .replay(files, &candidate)
                .map(|failure| candidate[..=failure.step].to_vec())
        }) {
            // Still failing without it: keep it out, and look at what is now in its place.
            Some(shorter) => steps = shorter,
            None => at += 1,
        }
    }
    steps
}

/// `steps` without the one at `dropped`, the later edits to its file moved to where they land.
pub(crate) fn without(steps: &[Step], dropped: usize) -> Option<Vec<Step>> {
    let gone = &steps[dropped];
    // Where the dropped edit's inserted text sits in the text at each later step, and how many
    // bytes the edit removed.
    let mut region_start = u32::from(gone.edit.range.start());
    let region_len = u32::try_from(gone.edit.insert.len()).ok()?;
    let removed = u32::from(gone.edit.range.len());

    let mut kept = steps[..dropped].to_vec();
    for step in &steps[dropped + 1..] {
        if step.path != gone.path {
            kept.push(step.clone());
            continue;
        }
        let (start, end) = (
            u32::from(step.edit.range.start()),
            u32::from(step.edit.range.end()),
        );
        // A position at the edge of a pure deletion is ambiguous: the removed bytes could be on
        // either side. A start goes after them and an end before, so a later edit covers what it
        // covered; an insertion goes after them at both ends.
        let region_end = region_start + region_len;
        let back = |at: u32, after: bool| -> Option<u32> {
            if at < region_start || (at == region_start && (region_len > 0 || !after)) {
                Some(at)
            } else if at >= region_end {
                Some(at - region_len + removed)
            } else {
                None
            }
        };
        let moved_start = back(start, true)?;
        let moved_end = if start == end {
            moved_start
        } else {
            back(end, false)?
        };
        if moved_start > moved_end {
            return None;
        }
        let moved = TextRange::new(TextSize::from(moved_start), TextSize::from(moved_end));
        kept.push(Step {
            path: step.path.clone(),
            edit: Edit::new(moved, step.edit.insert.clone()),
        });

        // Carry the region past this edit, in the text that still has the dropped one.
        let inserted = u32::try_from(step.edit.insert.len()).ok()?;
        // An insertion at the gap of a dropped deletion landed after it (above), so the gap stays
        // before what was inserted and the region does not move.
        let at_gap = region_len == 0 && start == end && start == region_start;
        if end <= region_start && !at_gap {
            region_start = region_start + inserted - (end - start);
        } else if start < region_start + region_len || (region_len == 0 && start < region_start) {
            // It overlapped the dropped edit's region, so later positions cannot be mapped.
            return None;
        }
    }
    Some(kept)
}
