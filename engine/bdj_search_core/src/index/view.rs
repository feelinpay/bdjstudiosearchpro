use super::ext_table::InternedExtensions;
use super::layout::{Header, SectionId, SectionTable, FLAG_DIR};
use super::vol_table::VolumeTable;
use std::fmt;

pub struct IndexView<'a> {
    pub header: &'a Header,
    pub section_table: &'a SectionTable,
    pub parent: &'a [u32],
    pub name_off: &'a [u32],
    pub name_len: &'a [u8],
    pub flags: &'a [u8],
    pub ext_id: &'a [u16],
    pub volume: &'a [u8],
    pub size: &'a [u64],
    pub mtime: &'a [u32],
    pub ctime: &'a [u32],
    pub alive: &'a [u64],
    pub name_order: &'a [u32],
    pub name_arena: &'a [u8],
    pub ext_table: InternedExtensions,
    pub vol_table: VolumeTable,
}

impl<'a> fmt::Debug for IndexView<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IndexView")
            .field("entry_count", &self.header.entry_count)
            .field("generation", &self.header.generation)
            .finish()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ViewError {
    BufferTooSmall,
    InvalidMagic,
    InvalidVersion(u32),
    CorruptedChecksum,
    UnalignedSection(SectionId),
    SectionOutOfBounds(SectionId),
}

impl std::fmt::Display for ViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for ViewError {}

impl<'a> IndexView<'a> {
    /// Zero-copy initialization directly from an aligned mmapped byte buffer.
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, ViewError> {
        let min_size = std::mem::size_of::<Header>() + std::mem::size_of::<SectionTable>();
        if bytes.len() < min_size {
            return Err(ViewError::BufferTooSmall);
        }

        // Cast Header
        let header_slice = &bytes[0..std::mem::size_of::<Header>()];
        let header: &Header = bytemuck::from_bytes(header_slice);
        if !header.is_valid() {
            if &header.magic != super::layout::MAGIC {
                return Err(ViewError::InvalidMagic);
            }
            if header.version != super::layout::CURRENT_VERSION {
                return Err(ViewError::InvalidVersion(header.version));
            }
            return Err(ViewError::CorruptedChecksum);
        }

        // Cast SectionTable
        let sec_start = std::mem::size_of::<Header>();
        let sec_end = sec_start + std::mem::size_of::<SectionTable>();
        let sec_slice = &bytes[sec_start..sec_end];
        let section_table: &SectionTable = bytemuck::from_bytes(sec_slice);

        let count = header.entry_count as usize;

        // Helper to extract typed slice
        let get_slice = |sec_id: SectionId, elem_size: usize| -> Result<&'a [u8], ViewError> {
            let desc = &section_table.sections[sec_id as usize];
            let start = desc.offset as usize;
            let len = desc.len as usize;
            let end = start + len;

            if !start.is_multiple_of(64) {
                return Err(ViewError::UnalignedSection(sec_id));
            }
            if end > bytes.len() {
                return Err(ViewError::SectionOutOfBounds(sec_id));
            }
            let slice = &bytes[start..end];
            if !slice.len().is_multiple_of(elem_size) {
                return Err(ViewError::SectionOutOfBounds(sec_id));
            }
            Ok(slice)
        };

        let parent: &'a [u32] = bytemuck::cast_slice(get_slice(SectionId::Parent, 4)?);
        let name_off: &'a [u32] = bytemuck::cast_slice(get_slice(SectionId::NameOff, 4)?);
        let name_len: &'a [u8] = get_slice(SectionId::NameLen, 1)?;
        let flags: &'a [u8] = get_slice(SectionId::Flags, 1)?;
        let ext_id: &'a [u16] = bytemuck::cast_slice(get_slice(SectionId::ExtId, 2)?);
        let volume: &'a [u8] = get_slice(SectionId::Volume, 1)?;
        let size: &'a [u64] = bytemuck::cast_slice(get_slice(SectionId::Size, 8)?);
        let mtime: &'a [u32] = bytemuck::cast_slice(get_slice(SectionId::Mtime, 4)?);
        let ctime: &'a [u32] = bytemuck::cast_slice(get_slice(SectionId::Ctime, 4)?);
        let alive: &'a [u64] = bytemuck::cast_slice(get_slice(SectionId::Alive, 8)?);
        let name_order: &'a [u32] = bytemuck::cast_slice(get_slice(SectionId::NameOrder, 4)?);
        let name_arena: &'a [u8] = get_slice(SectionId::NameArena, 1)?;

        let ext_bytes = get_slice(SectionId::ExtTable, 1)?;
        let ext_table = InternedExtensions::decode(ext_bytes).unwrap_or_default();

        let vol_bytes = get_slice(SectionId::VolTable, 1)?;
        let vol_table = VolumeTable::decode(vol_bytes).unwrap_or_default();

        debug_assert_eq!(parent.len(), count);

        Ok(Self {
            header,
            section_table,
            parent,
            name_off,
            name_len,
            flags,
            ext_id,
            volume,
            size,
            mtime,
            ctime,
            alive,
            name_order,
            name_arena,
            ext_table,
            vol_table,
        })
    }

    #[inline(always)]
    pub fn entry_count(&self) -> usize {
        self.header.entry_count as usize
    }

    #[inline(always)]
    pub fn is_alive(&self, idx: usize) -> bool {
        if idx >= self.entry_count() {
            return false;
        }
        let word = idx / 64;
        let bit = idx % 64;
        (self.alive[word] & (1 << bit)) != 0
    }

    #[inline(always)]
    pub fn is_dir(&self, idx: usize) -> bool {
        (self.flags[idx] & FLAG_DIR) != 0
    }

    #[inline(always)]
    pub fn get_name(&self, idx: usize) -> Option<&str> {
        if idx >= self.entry_count() {
            return None;
        }
        let off = self.name_off[idx] as usize;
        let len = self.name_len[idx] as usize;
        std::str::from_utf8(&self.name_arena[off..off + len]).ok()
    }

    #[inline(always)]
    pub fn get_name_bytes(&self, idx: usize) -> &[u8] {
        let off = self.name_off[idx] as usize;
        let len = self.name_len[idx] as usize;
        &self.name_arena[off..off + len]
    }

    #[inline(always)]
    pub fn get_extension(&self, idx: usize) -> &str {
        if idx >= self.entry_count() {
            return "";
        }
        self.ext_table.get_name(self.ext_id[idx]).unwrap_or("")
    }

    /// Resolves full path on the fly by walking up `parent` indices.
    /// Never stores duplicated full path strings in memory or disk.
    pub fn resolve_full_path(&self, idx: usize) -> String {
        if idx >= self.entry_count() {
            return String::new();
        }

        let mut segments: Vec<&str> = Vec::with_capacity(8);
        let mut curr = idx as u32;
        let vol_id = self.volume[idx];

        while curr != u32::MAX {
            let u_idx = curr as usize;
            if u_idx >= self.entry_count() {
                break;
            }
            if let Some(name) = self.get_name(u_idx) {
                segments.push(name);
            }
            curr = self.parent[u_idx];
        }

        let mount_prefix = self
            .vol_table
            .get(vol_id)
            .map(|v| v.mount_prefix.as_str())
            .unwrap_or(if cfg!(windows) { "C:\\" } else { "/" });

        let mut path = String::with_capacity(mount_prefix.len() + 128);
        path.push_str(mount_prefix);

        for (i, seg) in segments.iter().rev().enumerate() {
            if i > 0 || !path.ends_with('\\') && !path.ends_with('/') {
                if cfg!(windows) {
                    path.push('\\');
                } else {
                    path.push('/');
                }
            }
            path.push_str(seg);
        }

        path
    }
}
