use super::arena::NameArena;
use super::ext_table::InternedExtensions;
use super::layout::{
    Header, SectionDescriptor, SectionId, SectionTable, CURRENT_VERSION, FLAG_DIR, FLAG_NON_ASCII,
    MAGIC,
};
use super::vol_table::VolumeTable;
use rayon::prelude::*;
use std::io::{self, Write};

#[derive(Debug, Default)]
pub struct IndexBuilder {
    pub parents: Vec<u32>,
    pub name_offs: Vec<u32>,
    pub name_lens: Vec<u8>,
    pub flags: Vec<u8>,
    pub ext_ids: Vec<u16>,
    pub volumes: Vec<u8>,
    pub sizes: Vec<u64>,
    pub mtimes: Vec<u32>,
    pub ctimes: Vec<u32>,
    pub alive: Vec<u64>,
    pub name_order: Vec<u32>,
    pub arena: NameArena,
    pub ext_table: InternedExtensions,
    pub vol_table: VolumeTable,
    pub generation: u64,
    pub install_id: [u8; 16],
}

impl IndexBuilder {
    pub fn new() -> Self {
        Self {
            ext_table: InternedExtensions::new(),
            vol_table: VolumeTable::new(),
            generation: 1,
            install_id: [0u8; 16],
            ..Default::default()
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            parents: Vec::with_capacity(capacity),
            name_offs: Vec::with_capacity(capacity),
            name_lens: Vec::with_capacity(capacity),
            flags: Vec::with_capacity(capacity),
            ext_ids: Vec::with_capacity(capacity),
            volumes: Vec::with_capacity(capacity),
            sizes: Vec::with_capacity(capacity),
            mtimes: Vec::with_capacity(capacity),
            ctimes: Vec::with_capacity(capacity),
            alive: Vec::with_capacity(capacity.div_ceil(64)),
            name_order: Vec::with_capacity(capacity),
            arena: NameArena::with_capacity(capacity * 24),
            ext_table: InternedExtensions::new(),
            vol_table: VolumeTable::new(),
            generation: 1,
            install_id: [0u8; 16],
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
        let id = self.parents.len() as u32;
        let (off, len, non_ascii) = self.arena.push(name);

        let mut flag = 0u8;
        if is_dir {
            flag |= FLAG_DIR;
        }
        if is_hidden {
            flag |= super::layout::FLAG_HIDDEN;
        }
        if is_system {
            flag |= super::layout::FLAG_SYSTEM;
        }
        if non_ascii {
            flag |= FLAG_NON_ASCII;
        }

        let ext_id = if is_dir {
            0
        } else if let Some((_, ext)) = name.rsplit_once('.') {
            self.ext_table.intern(ext)
        } else {
            0
        };

        self.parents.push(parent);
        self.name_offs.push(off);
        self.name_lens.push(len);
        self.flags.push(flag);
        self.ext_ids.push(ext_id);
        self.volumes.push(vol_id);
        self.sizes.push(size);
        self.mtimes.push(mtime);
        self.ctimes.push(ctime);
        self.name_order.push(id);

        let word_idx = (id / 64) as usize;
        let bit_idx = id % 64;
        if word_idx >= self.alive.len() {
            self.alive.push(0);
        }
        self.alive[word_idx] |= 1 << bit_idx;

        id
    }

    pub fn count(&self) -> usize {
        self.parents.len()
    }

    /// Sorts `name_order` in parallel using Rayon for O(N) query time ordering.
    pub fn compute_name_order(&mut self) {
        let arena_bytes = self.arena.as_slice();
        let name_offs = &self.name_offs;
        let name_lens = &self.name_lens;

        self.name_order.par_sort_unstable_by(|&a, &b| {
            let a_off = name_offs[a as usize] as usize;
            let a_len = name_lens[a as usize] as usize;
            let b_off = name_offs[b as usize] as usize;
            let b_len = name_lens[b as usize] as usize;

            let a_bytes = &arena_bytes[a_off..a_off + a_len];
            let b_bytes = &arena_bytes[b_off..b_off + b_len];

            // Case-insensitive ASCII comparison first
            for (byte_a, byte_b) in a_bytes.iter().zip(b_bytes.iter()) {
                let diff = byte_a.to_ascii_lowercase().cmp(&byte_b.to_ascii_lowercase());
                if diff != std::cmp::Ordering::Equal {
                    return diff;
                }
            }
            a_len.cmp(&b_len)
        });
    }

    /// Writes the index to any `io::Write` sink, returning total bytes written.
    pub fn write_to<W: Write>(&mut self, writer: &mut W) -> io::Result<usize> {
        self.compute_name_order();

        let count = self.count() as u64;
        let arena_bytes = self.arena.as_slice();
        let ext_bytes = self.ext_table.encode();
        let vol_bytes = self.vol_table.encode();

        let mut section_table = SectionTable::default();
        let mut current_offset = (std::mem::size_of::<Header>() + std::mem::size_of::<SectionTable>()) as u64;

        // Macro to assign section offset and length
        let mut assign_section = |sec_id: SectionId, byte_len: usize| {
            // Align to 64 bytes
            let rem = current_offset % 64;
            if rem != 0 {
                current_offset += 64 - rem;
            }
            section_table.sections[sec_id as usize] = SectionDescriptor {
                offset: current_offset,
                len: byte_len as u64,
            };
            current_offset += byte_len as u64;
        };

        assign_section(SectionId::Parent, self.parents.len() * 4);
        assign_section(SectionId::NameOff, self.name_offs.len() * 4);
        assign_section(SectionId::NameLen, self.name_lens.len());
        assign_section(SectionId::Flags, self.flags.len());
        assign_section(SectionId::ExtId, self.ext_ids.len() * 2);
        assign_section(SectionId::Volume, self.volumes.len());
        assign_section(SectionId::Size, self.sizes.len() * 8);
        assign_section(SectionId::Mtime, self.mtimes.len() * 4);
        assign_section(SectionId::Ctime, self.ctimes.len() * 4);
        assign_section(SectionId::Alive, self.alive.len() * 8);
        assign_section(SectionId::NameOrder, self.name_order.len() * 4);
        assign_section(SectionId::NameArena, arena_bytes.len());
        assign_section(SectionId::ExtTable, ext_bytes.len());
        assign_section(SectionId::VolTable, vol_bytes.len());

        let mut header = Header {
            magic: *MAGIC,
            version: CURRENT_VERSION,
            header_crc: 0,
            entry_count: count,
            arena_len: arena_bytes.len() as u64,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            generation: self.generation,
            install_id: self.install_id,
        };
        header.header_crc = header.compute_crc();

        // 1. Write Header (64 bytes)
        writer.write_all(bytemuck::bytes_of(&header))?;
        let mut bytes_written = std::mem::size_of::<Header>();

        // 2. Write SectionTable (256 bytes)
        writer.write_all(bytemuck::bytes_of(&section_table))?;
        bytes_written += std::mem::size_of::<SectionTable>();

        // Helper to write section with alignment padding
        let mut write_aligned = |w: &mut W, data: &[u8]| -> io::Result<()> {
            let rem = bytes_written % 64;
            if rem != 0 {
                let pad = 64 - rem;
                w.write_all(&vec![0u8; pad])?;
                bytes_written += pad;
            }
            w.write_all(data)?;
            bytes_written += data.len();
            Ok(())
        };

        write_aligned(writer, bytemuck::cast_slice(&self.parents))?;
        write_aligned(writer, bytemuck::cast_slice(&self.name_offs))?;
        write_aligned(writer, &self.name_lens)?;
        write_aligned(writer, &self.flags)?;
        write_aligned(writer, bytemuck::cast_slice(&self.ext_ids))?;
        write_aligned(writer, &self.volumes)?;
        write_aligned(writer, bytemuck::cast_slice(&self.sizes))?;
        write_aligned(writer, bytemuck::cast_slice(&self.mtimes))?;
        write_aligned(writer, bytemuck::cast_slice(&self.ctimes))?;
        write_aligned(writer, bytemuck::cast_slice(&self.alive))?;
        write_aligned(writer, bytemuck::cast_slice(&self.name_order))?;
        write_aligned(writer, arena_bytes)?;
        write_aligned(writer, &ext_bytes)?;
        write_aligned(writer, &vol_bytes)?;

        writer.flush()?;
        Ok(bytes_written)
    }
}
