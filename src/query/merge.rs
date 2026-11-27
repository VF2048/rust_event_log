use std::cmp::{Ordering, Reverse};
use crate::segment::header::EventRef;

pub fn merge_posting_lists(
    lists: Vec<Vec<EventRef>>,
    limit: usize,
) -> Vec<EventRef> {
    if limit == 0 {
        return Vec::new();
    }

    let capacity = lists.iter().map(Vec::len).sum();
    let mut merge = Vec::with_capacity(capacity);

    for list in lists {
        merge.extend(list);
    }

    merge.sort_unstable_by(newest_first);

    merge.dedup();
    merge.truncate(limit);

    merge

}

fn newest_first(a: &EventRef, b: &EventRef) -> Ordering {
    b.timestamp
        .cmp(&a.timestamp)
        .then_with(|| a.segment_id.cmp(&b.segment_id))
        .then_with(|| a.block_offset.cmp(&b.block_offset))
        .then_with(|| a.record_index.cmp(&b.record_index))
}
