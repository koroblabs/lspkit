use super::*;

#[test]
fn a_string_reports_its_capacity_and_not_its_length() {
    // Resident bytes: spare capacity is still held.
    let mut text = String::with_capacity(64);
    text.push_str("short");

    assert_eq!(text.heap_size(), 64);
}

#[test]
fn a_vector_of_flat_elements_is_its_capacity() {
    let mut numbers: Vec<Box<str>> = Vec::with_capacity(4);
    numbers.push("ab".into());

    // Four slots of `Box<str>`, plus the two bytes the one element owns.
    assert_eq!(numbers.heap_size(), 4 * size_of::<Box<str>>() + 2);
}

#[test]
fn an_empty_table_costs_nothing() {
    // hashbrown does not allocate until the first insert.
    assert_eq!(table::<u32, u32>(0), 0);
}

#[test]
fn a_table_rounds_up_to_the_next_power_of_two_of_buckets() {
    // Seven entries need eight buckets; eight need sixteen, because the load factor stops at
    // seven eighths. Each bucket is an 8-byte entry and a control byte, plus a 16-byte group.
    assert_eq!(table::<u32, u32>(7), 8 * 8 + 8 + 16);
    assert_eq!(table::<u32, u32>(8), 16 * 8 + 16 + 16);
}

#[test]
fn a_single_entry_takes_four_buckets() {
    assert_eq!(table::<u32, u32>(1), 4 * 8 + 4 + 16);
}

#[test]
fn a_table_entry_carries_its_alignment_padding() {
    // `(u8, u64)` is 16 bytes, not 9.
    assert_eq!(size_of::<(u8, u64)>(), 16);
    assert_eq!(table::<u8, u64>(3), 4 * 16 + 4 + 16);
}
