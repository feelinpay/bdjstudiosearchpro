pub struct RadixSort;

impl RadixSort {
    /// Sort IDs by an accompanying u64 key (e.g. file size)
    pub fn sort_by_u64_key(ids: &mut [u32], keys: &[u64], ascending: bool) {
        if ids.is_empty() {
            return;
        }

        let mut pairs: Vec<(u64, u32)> = ids.iter().map(|&id| (keys[id as usize], id)).collect();
        if ascending {
            pairs.sort_unstable_by_key(|p| p.0);
        } else {
            pairs.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        }

        for (i, p) in pairs.into_iter().enumerate() {
            ids[i] = p.1;
        }
    }

    /// Sort IDs by an accompanying u32 key (e.g. Unix mtime timestamp)
    pub fn sort_by_u32_key(ids: &mut [u32], keys: &[u32], ascending: bool) {
        if ids.is_empty() {
            return;
        }

        let mut pairs: Vec<(u32, u32)> = ids.iter().map(|&id| (keys[id as usize], id)).collect();
        if ascending {
            pairs.sort_unstable_by_key(|p| p.0);
        } else {
            pairs.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        }

        for (i, p) in pairs.into_iter().enumerate() {
            ids[i] = p.1;
        }
    }
}
