//! Heap memory owned by a structure, reported by type.
//!
//! `ps` gives a process's resident total; this crate shows which structures hold it.
//!
//! # Shared allocations are counted once
//!
//! When several holders share one allocation, the owner counts its bytes and every other holder
//! counts only the pointer. In phrost, the interner hands out hundreds of thousands of `Arc<str>`
//! names over about 190 000 distinct strings. Counting the bytes at every holder would overstate
//! the total several times over.
//!
//! Use [`flat`] for elements that own nothing beyond themselves and [`deep`] for elements that do,
//! choosing per field. `flat` and [`vec_flat`] trust the caller that the elements own nothing;
//! nothing checks it.

use std::mem::{size_of, size_of_val};

/// Heap memory a value owns, excluding the value itself.
///
/// A value inside another structure is already counted by its holder, as a field or as an element
/// of the holder's buffer.
pub trait MemSize {
    fn heap_size(&self) -> usize;
}

/// The size of a slice's buffer, for elements that own nothing beyond themselves.
///
/// This counts the length, since a slice has no capacity. For a `Vec`, use [`vec_flat`]: a vector
/// grown by pushing can have nearly twice its length in capacity.
pub fn flat<T>(items: &[T]) -> usize {
    size_of_val(items)
}

/// The size of a slice's buffer, plus what each element owns.
pub fn deep<T: MemSize>(items: &[T]) -> usize {
    flat(items) + items.iter().map(MemSize::heap_size).sum::<usize>()
}

/// A `Vec`'s whole allocation, capacity included, plus what its elements own.
///
/// Capacity is counted because a vector grown by pushing holds memory beyond its length, and the
/// report explains resident bytes.
pub fn vec<T: MemSize>(items: &Vec<T>) -> usize {
    items.capacity() * size_of::<T>() + items.iter().map(MemSize::heap_size).sum::<usize>()
}

/// [`vec()`] for elements that own nothing.
pub fn vec_flat<T>(items: &Vec<T>) -> usize {
    items.capacity() * size_of::<T>()
}

/// A hash table's own allocation, for `K` and `V` that own nothing beyond themselves.
///
/// This is an estimate from the length, since nothing exposes the real figure. It follows
/// `hashbrown`'s `capacity_to_buckets` (0.15): 4 buckets for 1..=3 entries, 8 for 4..=7, otherwise
/// `(len * 8 / 7).next_power_of_two()`. Each bucket holds a `(K, V)` with its alignment padding
/// plus one control byte, and the control bytes have a trailing group of `GROUP_WIDTH` more. That
/// width is 16 with SSE2 and 8 on NEON or the generic fallback; this assumes 16. `hashbrown` gives
/// tables of tiny entries (under 4 bytes) a larger minimum, which this ignores. The error is always
/// in the same direction, so a change in the report still reflects a change in the program.
pub fn table<K, V>(len: usize) -> usize {
    const GROUP_WIDTH: usize = 16;
    let buckets = match len {
        0 => return 0,
        1..=3 => 4,
        4..=7 => 8,
        _ => (len.saturating_mul(8) / 7).next_power_of_two(),
    };
    buckets * size_of::<(K, V)>() + buckets + GROUP_WIDTH
}

impl MemSize for String {
    fn heap_size(&self) -> usize {
        self.capacity()
    }
}

impl MemSize for Box<str> {
    fn heap_size(&self) -> usize {
        self.len()
    }
}

impl<T: MemSize> MemSize for Box<[T]> {
    fn heap_size(&self) -> usize {
        deep(self)
    }
}

impl<T: MemSize> MemSize for Option<T> {
    fn heap_size(&self) -> usize {
        self.as_ref().map_or(0, MemSize::heap_size)
    }
}

impl<T: MemSize> MemSize for Vec<T> {
    fn heap_size(&self) -> usize {
        vec(self)
    }
}

#[cfg(test)]
mod tests;

// Runs the README's examples as doctests so they stay in sync with the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
