use crate::index::view::IndexView;

pub struct PermutationSort;

impl PermutationSort {
    /// In-place ASCII case-insensitive sort for small result sets (< 1000 items).
    /// Avoids allocating bitsets and avoids scanning millions of entries in name_order.
    #[inline]
    pub fn sort_in_place_small(ids: &mut [u32], view: &IndexView, ascending: bool) {
        ids.sort_unstable_by(|&a, &b| {
            let name_a = view.get_name(a as usize).unwrap_or("");
            let name_b = view.get_name(b as usize).unwrap_or("");
            let ord = unicase::Ascii::new(name_a).cmp(&unicase::Ascii::new(name_b));
            if ascending {
                ord
            } else {
                ord.reverse()
            }
        });
    }

    /// Filters the precalculated name_order permutation against the matched alive IDs.
    /// This is O(N) where N is alive entries, avoiding expensive string comparisons at query time.
    pub fn sort_by_name_order(
        matched_bitset: &[u64],
        name_order: &[u32],
        ascending: bool,
        matched_count: usize,
    ) -> Vec<u32> {
        let mut sorted = Vec::with_capacity(matched_count);
        if ascending {
            for &id in name_order {
                let w = (id / 64) as usize;
                let b = id % 64;
                if (matched_bitset[w] & (1 << b)) != 0 {
                    sorted.push(id);
                }
            }
        } else {
            for &id in name_order.iter().rev() {
                let w = (id / 64) as usize;
                let b = id % 64;
                if (matched_bitset[w] & (1 << b)) != 0 {
                    sorted.push(id);
                }
            }
        }
        sorted
    }
}
