use std::cmp::Reverse;
use crate::segment::header::EventRef;

pub fn merge_posting_lists(
    lists: Vec<Vec<EventRef>>,
    limit: usize,
) -> Vec<EventRef> {
    let mut flat: Vec<EventRef> = lists.into_iter().flatten().collect();
    flat.sort_unstable_by_key(|item| Reverse(*item));
    flat.dedup();
    flat.truncate(limit);
    flat
}
