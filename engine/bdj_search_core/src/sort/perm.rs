pub struct PermutationSort;

impl PermutationSort {
    /// Filters the precalculated name_order permutation against the matched alive IDs.
    /// This is O(N) where N is alive entries, avoiding expensive string comparisons at query time.
    pub fn sort_by_name_order(matched_bitset: &[u64], name_order: &[u32], ascending: bool) -> Vec<u32> {
        let mut sorted = Vec::with_capacity(name_order.len());
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
