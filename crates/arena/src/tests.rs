use super::*;

arena_id!(
    /// Test-only id family.
    pub(crate) struct TestId;
);

#[test]
fn allocates_in_order_and_addresses_by_id() {
    let mut arena = Arena::<TestId, &str>::new();
    let first = arena.alloc("a");
    let second = arena.alloc("b");

    assert_eq!(arena[first], "a");
    assert_eq!(arena[second], "b");
    assert_eq!(arena.len(), 2);
    assert_eq!(
        arena.entries().map(|(id, _)| id).collect::<Vec<_>>(),
        vec![first, second]
    );
}

#[test]
fn debug_output_names_the_id_family() {
    assert_eq!(format!("{:?}", TestId::from_index(7)), "TestId(7)");
}

arena_id!(
    pub(crate) struct ItemId;
);

#[test]
fn lists_stay_separate_even_when_pushed_out_of_order() {
    let mut lists = IdVec::<ItemId>::new();
    let ids: Vec<ItemId> = (0..5).map(ItemId::from_index).collect();

    // In practice an inner declaration finishes before the outer one.
    let inner = lists.push_slice(&ids[3..5]);
    let outer = lists.push_slice(&ids[0..3]);

    assert_eq!(lists.slice(inner), &ids[3..5]);
    assert_eq!(lists.slice(outer), &ids[0..3]);
    assert_eq!(lists.len(), 5);
}

#[test]
fn the_empty_slice_needs_no_storage() {
    let lists = IdVec::<ItemId>::new();
    assert!(IdSlice::<ItemId>::EMPTY.is_empty());
    assert!(lists.slice(IdSlice::EMPTY).is_empty());
}
