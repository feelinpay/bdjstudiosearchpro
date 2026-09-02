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

    /// Reasigna el padre de una entrada ya insertada.
    ///
    /// La enumeración de la MFT entrega los registros en orden arbitrario: un
    /// archivo puede aparecer antes que su carpeta. Por eso se insertan todos
    /// con un padre provisional y se corrigen en una segunda pasada, cuando ya
    /// existe el mapa completo de números de referencia a índices.
    pub fn set_parent(&mut self, idx: u32, parent: u32) {
        let i = idx as usize;
        if i < self.parents.len() {
            // Un padre que es uno mismo produciría un ciclo al resolver la ruta.
            self.parents[i] = if parent == idx { u32::MAX } else { parent };
        }
    }

    /// Rellena tamaño y fechas en la segunda fase del indexado.
    ///
    /// `FSCTL_ENUM_USN_DATA` entrega nombres y jerarquía en segundos, pero no
    /// trae ni tamaño ni fechas; esas llegan después recorriendo directorios.
    pub fn set_metadata(&mut self, idx: u32, size: u64, mtime: u32, ctime: u32) {
        let i = idx as usize;
        if i < self.sizes.len() {
            self.sizes[i] = size;
            self.mtimes[i] = mtime;
            self.ctimes[i] = ctime;
        }
    }

    pub fn set_attributes(&mut self, idx: u32, is_hidden: bool, is_system: bool) {
        let i = idx as usize;
        if i >= self.flags.len() {
            return;
        }
        let mut flag = self.flags[i];
        if is_hidden {
            flag |= super::layout::FLAG_HIDDEN;
        } else {
            flag &= !super::layout::FLAG_HIDDEN;
        }
        if is_system {
            flag |= super::layout::FLAG_SYSTEM;
        } else {
            flag &= !super::layout::FLAG_SYSTEM;
        }
        self.flags[i] = flag;
    }

    pub fn name_at(&self, idx: u32) -> &str {
        let i = idx as usize;
        if i >= self.name_offs.len() {
            return "";
        }
        let off = self.name_offs[i] as usize;
        let len = self.name_lens[i] as usize;
        std::str::from_utf8(&self.arena.as_slice()[off..off + len]).unwrap_or("")
    }

    pub fn is_dir(&self, idx: u32) -> bool {
        let i = idx as usize;
        i < self.flags.len() && (self.flags[i] & FLAG_DIR) != 0
    }

    /// Descarta las entradas a partir de `len`.
    ///
    /// Sirve para deshacer un escaneo que fallo a medias: sin esto, caer al
    /// recorrido de directorios despues de una lectura parcial de la MFT
    /// duplicaria en el indice todo lo que si se habia leido.
    ///
    /// Los nombres ya escritos siguen en la arena, sin que nadie los referencie.
    /// Es memoria desperdiciada, no corrupcion, y solo ocurre en un fallo.
    pub fn truncate_to(&mut self, len: u32) {
        let n = (len as usize).min(self.parents.len());
        self.parents.truncate(n);
        self.name_offs.truncate(n);
        self.name_lens.truncate(n);
        self.flags.truncate(n);
        self.ext_ids.truncate(n);
        self.volumes.truncate(n);
        self.sizes.truncate(n);
        self.mtimes.truncate(n);
        self.ctimes.truncate(n);
        self.name_order.truncate(n);

        let words = n.div_ceil(64);
        self.alive.truncate(words);
        // Apaga los bits sobrantes de la ultima palabra.
        if !n.is_multiple_of(64)
            && let Some(last) = self.alive.last_mut()
        {
            let keep = n % 64;
            *last &= (1u64 << keep) - 1;
        }
    }

    pub fn volume_of(&self, idx: u32) -> u8 {
        let i = idx as usize;
        if i < self.volumes.len() {
            self.volumes[i]
        } else {
            0
        }
    }

    pub fn parent_of(&self, idx: u32) -> u32 {
        let i = idx as usize;
        if i < self.parents.len() {
            self.parents[i]
        } else {
            u32::MAX
        }
    }

    /// Reconstruye la ruta de una entrada durante la construcción del índice.
    ///
    /// Aplica el mismo criterio que `IndexView::resolve_full_path`: la raíz del
    /// volumen no aporta segmento porque ya está en el prefijo de montaje.
    pub fn resolve_path(&self, idx: u32, mount_prefix: &str) -> String {
        let mut segments: Vec<&str> = Vec::with_capacity(16);
        let mut curr = idx;

        for _ in 0..super::view::IndexView::MAX_PATH_DEPTH {
            let i = curr as usize;
            if i >= self.parents.len() {
                break;
            }
            let parent = self.parents[i];
            if parent == u32::MAX {
                break;
            }
            let name = self.name_at(curr);
            if !name.is_empty() {
                segments.push(name);
            }
            if parent == curr {
                break;
            }
            curr = parent;
        }

        let sep = if mount_prefix.contains('\\') { '\\' } else { '/' };
        let mut path = String::with_capacity(mount_prefix.len() + 128);
        path.push_str(mount_prefix);
        for seg in segments.iter().rev() {
            if !path.ends_with('\\') && !path.ends_with('/') {
                path.push(sep);
            }
            path.push_str(seg);
        }
        path
    }

    /// Agrupa las entradas por carpeta contenedora, ordenadas por padre.
    ///
    /// Es lo que permite recorrer cada directorio del disco una sola vez en la
    /// segunda fase, en lugar de abrir un identificador por archivo.
    pub fn children_by_parent(&self) -> Vec<(u32, u32)> {
        let mut pairs: Vec<(u32, u32)> = self
            .parents
            .iter()
            .enumerate()
            .filter(|&(_, &p)| p != u32::MAX)
            .map(|(i, &p)| (p, i as u32))
            .collect();
        pairs.par_sort_unstable();
        pairs
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

    /// Permutación inversa de `name_order`: `name_rank[id]` es la posición
    /// alfabética de `id`.
    ///
    /// Ordenar un resultado por nombre pasa así a ser ordenar por una clave
    /// entera de 4 bytes que ya está en el índice. Antes había que recorrer
    /// `name_order` entera —diez millones de posiciones— filtrando por un mapa
    /// de bits, aunque el resultado fueran tres filas.
    fn compute_name_rank(&self) -> Vec<u32> {
        let mut rank = vec![0u32; self.name_order.len()];
        for (pos, &id) in self.name_order.iter().enumerate() {
            let i = id as usize;
            if i < rank.len() {
                rank[i] = pos as u32;
            }
        }
        rank
    }

    /// Lista de hijos de cada entrada en formato comprimido por filas.
    ///
    /// Devuelve `(desplazamientos, hijos)`: los hijos de `i` son
    /// `hijos[desplazamientos[i]..desplazamientos[i + 1]]`, ordenados por
    /// identificador. Es una ordenación por conteo en dos pasadas, sin
    /// comparaciones.
    ///
    /// Con esto, abrir una carpeta es leer un rango contiguo de memoria; sin
    /// esto haría falta recorrer el índice entero o preguntarle al disco.
    fn compute_children(&self) -> (Vec<u32>, Vec<u32>) {
        let n = self.parents.len();
        // Una posición extra para cerrar el último rango.
        let mut offsets = vec![0u32; n + 1];
        if n == 0 {
            return (offsets, Vec::new());
        }

        let mut total = 0usize;
        for &p in &self.parents {
            if p != u32::MAX && (p as usize) < n {
                offsets[p as usize + 1] += 1;
                total += 1;
            }
        }
        for i in 0..n {
            offsets[i + 1] += offsets[i];
        }

        let mut cursor = offsets.clone();
        let mut children = vec![0u32; total];
        for (child, &p) in self.parents.iter().enumerate() {
            if p != u32::MAX && (p as usize) < n {
                let slot = cursor[p as usize] as usize;
                children[slot] = child as u32;
                cursor[p as usize] += 1;
            }
        }

        (offsets, children)
    }

    /// Writes the index to any `io::Write` sink, returning total bytes written.
    pub fn write_to<W: Write>(&mut self, writer: &mut W) -> io::Result<usize> {
        self.compute_name_order();
        let name_rank = self.compute_name_rank();
        let (child_off, child_idx) = self.compute_children();

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
        assign_section(SectionId::NameRank, name_rank.len() * 4);
        assign_section(SectionId::ChildOff, child_off.len() * 4);
        assign_section(SectionId::ChildIdx, child_idx.len() * 4);

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
        write_aligned(writer, bytemuck::cast_slice(&name_rank))?;
        write_aligned(writer, bytemuck::cast_slice(&child_off))?;
        write_aligned(writer, bytemuck::cast_slice(&child_idx))?;

        writer.flush()?;
        Ok(bytes_written)
    }
}

/// Garantiza que el archivo de índice tenga permisos restrictivos (solo lectura/escritura del propietario).
pub fn set_restrictive_permissions(_path: &std::path::Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(_path, perms);
    }
    Ok(())
}
