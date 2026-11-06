//! Typed dense arenas for ids local to one stage of work.
//!
//! A stage (one pass that builds a set of entities) allocates them into a dense [`Arena`] and hands
//! out small typed ids instead of references. A later stage can then refer to an entity without
//! knowing its layout, since it only carries the id.
//!
//! An id names an entity only within the arena that issued it. To point into one of several arenas,
//! carry which arena alongside the id.

use std::{
    fmt,
    marker::PhantomData,
    ops::{Index, IndexMut},
};

/// A stable typed index into an [`Arena`].
///
/// Implementations are newtypes around `u32`. Storage stays a plain dense `Vec`, and the types
/// keep callers from mixing unrelated id families.
pub trait ArenaId: Copy + Eq {
    /// The id for the entity at `index`.
    ///
    /// # Panics
    ///
    /// The ids [`arena_id!`] declares panic when `index` does not fit into `u32`.
    fn from_index(index: usize) -> Self;
    fn index(self) -> usize;
}

/// Declares an [`ArenaId`] newtype.
///
/// ```
/// lspkit_arena::arena_id!(
///     /// One node of a syntax tree.
///     pub struct NodeId;
/// );
/// ```
#[macro_export]
macro_rules! arena_id {
    ($(#[$meta:meta])* $vis:vis struct $name:ident;) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        $vis struct $name(u32);

        impl $crate::ArenaId for $name {
            /// # Panics
            ///
            /// When `index` does not fit into `u32`.
            fn from_index(index: usize) -> Self {
                Self(u32::try_from(index).expect("arena index should fit into u32"))
            }

            fn index(self) -> usize {
                self.0 as usize
            }
        }

        impl ::std::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }
    };
}

/// Dense storage for one entity family, addressed by a typed id.
pub struct Arena<Id, T> {
    items: Vec<T>,
    // `fn() -> Id` keeps the arena covariant in `Id` and never claims to own one.
    id: PhantomData<fn() -> Id>,
}

impl<Id: ArenaId, T> Arena<Id, T> {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            id: PhantomData,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            items: Vec::with_capacity(capacity),
            id: PhantomData,
        }
    }

    /// Stores `value` and returns its id.
    ///
    /// # Panics
    ///
    /// When the arena already holds more entities than the id family can address (for the ids
    /// [`arena_id!`] declares, `u32::MAX`).
    pub fn alloc(&mut self, value: T) -> Id {
        let id = Id::from_index(self.items.len());
        self.items.push(value);
        id
    }

    #[must_use]
    pub fn get(&self, id: Id) -> Option<&T> {
        self.items.get(id.index())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &T> {
        self.items.iter()
    }

    /// Iterates over the items with their ids, in allocation order. Callers may rely on that
    /// order, for example for stable snapshot output.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = (Id, &T)> {
        self.items
            .iter()
            .enumerate()
            .map(|(index, item)| (Id::from_index(index), item))
    }

    /// Releases the spare capacity a builder left behind, so a retained snapshot does not keep it.
    pub fn shrink_to_fit(&mut self) {
        self.items.shrink_to_fit();
    }
}

impl<Id: ArenaId, T> Default for Arena<Id, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Id: ArenaId, T> Index<Id> for Arena<Id, T> {
    type Output = T;

    /// # Panics
    ///
    /// When `id` was issued by another arena and is out of range here.
    fn index(&self, id: Id) -> &T {
        &self.items[id.index()]
    }
}

impl<Id: ArenaId, T> IndexMut<Id> for Arena<Id, T> {
    fn index_mut(&mut self, id: Id) -> &mut T {
        &mut self.items[id.index()]
    }
}

impl<Id: ArenaId, T: fmt::Debug> fmt::Debug for Arena<Id, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.items.iter()).finish()
    }
}

impl<Id: ArenaId, T> FromIterator<T> for Arena<Id, T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self {
            items: iter.into_iter().collect(),
            id: PhantomData,
        }
    }
}

/// Flat storage for many short id lists that belong to arena entities.
///
/// A class's members or a function's parameters form a list. Giving every entity its own `Vec`
/// costs one heap allocation per class, and a project can have tens of thousands of classes.
/// `IdVec` pushes the lists into one buffer and hands back an [`IdSlice`] for each.
///
/// `push_slice` takes a whole list at once, so each list is contiguous whatever order the caller
/// produces ids in. A declaration nested inside another during lowering cannot corrupt the outer
/// list.
pub struct IdVec<Id> {
    items: Vec<Id>,
}

impl<Id: Copy> IdVec<Id> {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Copies `ids` into the buffer and returns a handle to them.
    ///
    /// # Panics
    ///
    /// When the buffer or `ids` outgrows `u32`.
    pub fn push_slice(&mut self, ids: &[Id]) -> IdSlice<Id> {
        let start = u32::try_from(self.items.len()).expect("id list length should fit into u32");
        self.items.extend_from_slice(ids);

        IdSlice {
            start,
            len: u32::try_from(ids.len()).expect("id list length should fit into u32"),
            id: PhantomData,
        }
    }

    /// The ids a handle names.
    ///
    /// # Panics
    ///
    /// When `slice` came from another `IdVec` and does not lie inside this one.
    #[must_use]
    pub fn slice(&self, slice: IdSlice<Id>) -> &[Id] {
        let start = slice.start as usize;
        &self.items[start..start + slice.len as usize]
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn shrink_to_fit(&mut self) {
        self.items.shrink_to_fit();
    }
}

impl<Id: Copy> Default for IdVec<Id> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Id: fmt::Debug> fmt::Debug for IdVec<Id> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.items.iter()).finish()
    }
}

/// Handle to one id list inside an [`IdVec`].
pub struct IdSlice<Id> {
    start: u32,
    len: u32,
    id: PhantomData<fn() -> Id>,
}

impl<Id> IdSlice<Id> {
    /// An empty list, valid against any [`IdVec`] without pushing anything.
    pub const EMPTY: Self = Self {
        start: 0,
        len: 0,
        id: PhantomData,
    };

    #[must_use]
    pub fn len(self) -> usize {
        self.len as usize
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

// Derived impls would demand `Id: Clone`/`Id: Copy`, which a handle does not need.
impl<Id> Clone for IdSlice<Id> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Id> Copy for IdSlice<Id> {}

impl<Id> PartialEq for IdSlice<Id> {
    fn eq(&self, other: &Self) -> bool {
        self.start == other.start && self.len == other.len
    }
}

impl<Id> Eq for IdSlice<Id> {}

impl<Id> fmt::Debug for IdSlice<Id> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IdSlice({}..{})", self.start, self.start + self.len)
    }
}

#[cfg(test)]
mod tests;

// Runs the README's examples as doctests so they stay in sync with the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
