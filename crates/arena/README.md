# lspkit-arena

Typed dense arenas for the phases of an indexer. Each phase allocates its entities into an `Arena`
and hands out small typed ids instead of references. A later phase can then refer to an entity
without knowing its layout, since it only carries the id.

Ids are `u32` newtypes declared with `arena_id!`, so ids of different entity families cannot be
mixed up. `IdVec` stores many short id lists (a class's members, a function's parameters) in one
buffer instead of one `Vec` per entity.

## Example

```rust
use lspkit_arena::{Arena, IdVec, arena_id};

arena_id!(
    /// One lowered class in a file.
    pub struct ClassId;
);

let mut classes = Arena::<ClassId, &str>::new();
let user = classes.alloc("User");
let post = classes.alloc("Post");
assert_eq!(classes[user], "User");
assert_eq!(classes.len(), 2);

// Many short id lists share one buffer.
let mut lists = IdVec::new();
let both = lists.push_slice(&[user, post]);
assert_eq!(lists.slice(both), [user, post]);
```

## Notes

- An id means something only together with the arena that issued it. Crossing a store boundary needs
  a reference type that also says which arena to look in.
- Storage is a plain dense `Vec`.
- `IdVec::push_slice` takes a whole list at once, so each list is contiguous whatever order the
  caller produces ids in. A declaration nested inside another during lowering cannot corrupt the
  outer list.
- A project with tens of thousands of classes pays one allocation for all their member lists instead
  of one per class.

Part of [lspkit](../../README.md), a toolkit for language servers.

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or
[MIT license](../../LICENSE-MIT) at your option.
