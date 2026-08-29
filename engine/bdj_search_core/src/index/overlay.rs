use super::builder::IndexBuilder;
use super::mmap::MmapIndex;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct OverlayIndex {
    pub builder: IndexBuilder,
    pub tombstones: Vec<u32>, // IDs in base that have been deleted or modified
}

impl OverlayIndex {
    pub fn new() -> Self {
        Self {
            builder: IndexBuilder::new(),
            tombstones: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_entry(
        &mut self,
        parent: u32,
        name: &str,
        is_dir: bool,
        is_hidden: bool,
        is_system: bool,
        vol_id: u8,
        size: u64,
        mtime: u32,
        ctime: u32,
    ) -> u32 {
        self.builder.add_entry(
            parent, name, is_dir, is_hidden, is_system, vol_id, size, mtime, ctime,
        )
    }

    pub fn mark_deleted(&mut self, base_id: u32) {
        self.tombstones.push(base_id);
    }

    pub fn overlay_count(&self) -> usize {
        self.builder.count()
    }

    pub fn should_compact(&self, base_count: usize) -> bool {
        if base_count == 0 {
            return self.overlay_count() > 0;
        }
        // Compact if overlay exceeds 5% of base count
        let threshold = (base_count / 20).max(500);
        self.overlay_count() >= threshold
    }

    /// Compacts the base index and the current overlay into a new `.bdjx.tmp` file,
    /// calls fsync, and atomically renames it over `target_path`.
    pub fn compact(
        &mut self,
        base: Option<&MmapIndex>,
        target_path: &Path,
        new_generation: u64,
    ) -> io::Result<usize> {
        let mut new_builder = IndexBuilder::new();
        new_builder.generation = new_generation;

        // 1. Copy surviving entries from Base
        if let Some(mmap_base) = base {
            let view = mmap_base.view().map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let count = view.entry_count();
            let mut tombstone_set = std::collections::HashSet::new();
            for &t in &self.tombstones {
                tombstone_set.insert(t);
            }

            for idx in 0..count {
                if !view.is_alive(idx) || tombstone_set.contains(&(idx as u32)) {
                    continue; // Skip tombstones
                }
                let name = view.get_name(idx).unwrap_or("");
                let is_dir = view.is_dir(idx);
                let is_hidden = (view.flags[idx] & super::layout::FLAG_HIDDEN) != 0;
                let is_system = (view.flags[idx] & super::layout::FLAG_SYSTEM) != 0;

                new_builder.add_entry(
                    view.parent[idx],
                    name,
                    is_dir,
                    is_hidden,
                    is_system,
                    view.volume[idx],
                    view.size[idx],
                    view.mtime[idx],
                    view.ctime[idx],
                );
            }
        }

        // 2. Append new entries from Overlay
        let overlay_count = self.builder.count();
        let name_arena = self.builder.arena.as_slice();
        for idx in 0..overlay_count {
            let off = self.builder.name_offs[idx] as usize;
            let len = self.builder.name_lens[idx] as usize;
            let name = std::str::from_utf8(&name_arena[off..off + len]).unwrap_or("");
            let is_dir = (self.builder.flags[idx] & super::layout::FLAG_DIR) != 0;
            let is_hidden = (self.builder.flags[idx] & super::layout::FLAG_HIDDEN) != 0;
            let is_system = (self.builder.flags[idx] & super::layout::FLAG_SYSTEM) != 0;

            new_builder.add_entry(
                self.builder.parents[idx],
                name,
                is_dir,
                is_hidden,
                is_system,
                self.builder.volumes[idx],
                self.builder.sizes[idx],
                self.builder.mtimes[idx],
                self.builder.ctimes[idx],
            );
        }

        // 3. Write to temporary file with fsync
        let tmp_path = PathBuf::from(format!("{}.tmp", target_path.display()));
        let tmp_file = File::create(&tmp_path)?;
        let mut writer = BufWriter::with_capacity(2 * 1024 * 1024, tmp_file);
        let written = new_builder.write_to(&mut writer)?;
        writer.flush()?;
        writer.into_inner()?.sync_all()?;

        // 4. Atomic rename over target_path
        std::fs::rename(&tmp_path, target_path)?;

        // Reset overlay state after successful compaction
        self.builder = IndexBuilder::new();
        self.tombstones.clear();

        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_overlay_and_compaction() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        let mut overlay = OverlayIndex::new();
        overlay.add_entry(u32::MAX, "FolderA", true, false, false, 0, 0, 100, 100);
        overlay.add_entry(0, "Track1.wav", false, false, false, 0, 5000, 100, 100);

        overlay.compact(None, path, 1).unwrap();

        // Verify newly compacted file
        let mmap_index = MmapIndex::open(path).unwrap();
        let view = mmap_index.view().unwrap();
        assert_eq!(view.entry_count(), 2);
        assert_eq!(view.get_name(0), Some("FolderA"));
        assert_eq!(view.get_name(1), Some("Track1.wav"));
        assert_eq!(view.get_extension(1), "wav");
    }
}
