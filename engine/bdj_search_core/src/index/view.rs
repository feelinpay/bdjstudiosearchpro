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
    /// Posición alfabética de cada entrada. Permutación inversa de `name_order`.
    pub name_rank: &'a [u32],
    /// Desplazamientos CSR de la lista de hijos. Longitud `entry_count + 1`.
    pub child_off: &'a [u32],
    /// Identificadores de los hijos, agrupados por padre.
    pub child_idx: &'a [u32],
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
        let name_rank: &'a [u32] = bytemuck::cast_slice(get_slice(SectionId::NameRank, 4)?);
        let child_off: &'a [u32] = bytemuck::cast_slice(get_slice(SectionId::ChildOff, 4)?);
        let child_idx: &'a [u32] = bytemuck::cast_slice(get_slice(SectionId::ChildIdx, 4)?);
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
            name_rank,
            child_off,
            child_idx,
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

    /// Número de entradas vivas.
    ///
    /// Recorre el mapa de bits contando con `count_ones`, que es una única
    /// instrucción por palabra de 64 bits: diez millones de entradas se cuentan
    /// en unas ciento cincuenta mil operaciones.
    pub fn alive_count(&self) -> usize {
        let count = self.entry_count();
        let full_words = count / 64;
        let mut total: usize = self.alive[..full_words.min(self.alive.len())]
            .iter()
            .map(|w| w.count_ones() as usize)
            .sum();
        let rest = count % 64;
        if rest != 0 && full_words < self.alive.len() {
            let mask = (1u64 << rest) - 1;
            total += (self.alive[full_words] & mask).count_ones() as usize;
        }
        total
    }

    #[inline(always)]
    pub fn is_dir(&self, idx: usize) -> bool {
        (self.flags[idx] & FLAG_DIR) != 0
    }

    #[inline(always)]
    pub fn is_hidden(&self, idx: usize) -> bool {
        if idx >= self.entry_count() {
            return false;
        }
        (self.flags[idx] & crate::index::layout::FLAG_HIDDEN) != 0
            || self.get_name_bytes(idx).first() == Some(&b'.')
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

    /// Profundidad máxima al reconstruir una ruta.
    ///
    /// Ni NTFS ni APFS admiten jerarquías tan hondas, así que superarla solo
    /// puede significar que la columna `parent` tiene un ciclo. Sin este tope,
    /// un índice corrupto colgaría el proceso y agotaría la memoria.
    pub const MAX_PATH_DEPTH: usize = 128;

    /// Reconstruye la ruta completa subiendo por `parent`.
    ///
    /// La raíz del volumen no aporta segmento: su nombre ya está representado
    /// por el prefijo de montaje de la tabla de volúmenes. Incluirla producía
    /// rutas duplicadas del tipo `C:\C:\archivo.wav`.
    pub fn resolve_full_path(&self, idx: usize) -> String {
        if idx >= self.entry_count() {
            return String::new();
        }

        let mut segments: Vec<&str> = Vec::with_capacity(16);
        let mut curr = idx as u32;
        let vol_id = self.volume[idx];

        for _ in 0..Self::MAX_PATH_DEPTH {
            let u_idx = curr as usize;
            if u_idx >= self.entry_count() {
                break;
            }
            let parent = self.parent[u_idx];
            // `parent == u32::MAX` marca la raíz del volumen: no se añade.
            if parent == u32::MAX {
                break;
            }
            if let Some(name) = self.get_name(u_idx)
                && !name.is_empty()
            {
                segments.push(name);
            }
            if parent == curr {
                break; // autorreferencia: índice corrupto
            }
            curr = parent;
        }

        let mount_prefix = self
            .vol_table
            .get(vol_id)
            .map(|v| v.mount_prefix.as_str())
            .unwrap_or(if cfg!(windows) { "C:\\" } else { "/" });

        let sep = if mount_prefix.contains('\\') || cfg!(windows) {
            '\\'
        } else {
            '/'
        };

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

    /// Ruta de la carpeta que contiene a `idx`, para la columna «Ruta».
    pub fn resolve_parent_path(&self, idx: usize) -> String {
        if idx >= self.entry_count() {
            return String::new();
        }
        let parent = self.parent[idx];
        if parent == u32::MAX {
            return self.resolve_full_path(idx);
        }
        self.resolve_full_path(parent as usize)
    }

    /// Igual que `resolve_full_path`, pero escribiendo sobre buffers prestados.
    ///
    /// El filtro `ruta:` evalúa esto **una vez por entrada candidata**. Con la
    /// versión que devuelve `String` eso eran dos reservas de memoria por
    /// archivo: sobre diez millones, veinte millones de reservas por pulsación
    /// de tecla. Aquí los dos buffers se reutilizan durante todo el recorrido.
    pub fn resolve_full_path_into(&self, idx: usize, segments: &mut Vec<u32>, out: &mut String) {
        segments.clear();
        out.clear();
        if idx >= self.entry_count() {
            return;
        }

        let mut curr = idx as u32;
        let vol_id = self.volume[idx];

        for _ in 0..Self::MAX_PATH_DEPTH {
            let u_idx = curr as usize;
            if u_idx >= self.entry_count() {
                break;
            }
            let parent = self.parent[u_idx];
            if parent == u32::MAX {
                break;
            }
            segments.push(curr);
            if parent == curr {
                break;
            }
            curr = parent;
        }

        let mount_prefix = self
            .vol_table
            .get(vol_id)
            .map(|v| v.mount_prefix.as_str())
            .unwrap_or(if cfg!(windows) { "C:\\" } else { "/" });

        let sep = if mount_prefix.contains('\\') || cfg!(windows) {
            '\\'
        } else {
            '/'
        };

        out.push_str(mount_prefix);
        for &id in segments.iter().rev() {
            let name = self.get_name(id as usize).unwrap_or("");
            if name.is_empty() {
                continue;
            }
            if !out.ends_with('\\') && !out.ends_with('/') {
                out.push(sep);
            }
            out.push_str(name);
        }
    }

    /// Hijos directos de `idx`, ordenados por identificador.
    ///
    /// Es una rodaja de memoria ya mapeada: no reserva, no compara y no toca el
    /// disco. Un índice de la versión 1 —o uno recién migrado sin esta columna—
    /// devuelve una rodaja vacía en lugar de fallar.
    #[inline]
    pub fn children(&self, idx: usize) -> &'a [u32] {
        let n = self.entry_count();
        if idx >= n || self.child_off.len() < n + 1 {
            return &[];
        }
        let start = self.child_off[idx] as usize;
        let end = self.child_off[idx + 1] as usize;
        if start > end || end > self.child_idx.len() {
            return &[];
        }
        &self.child_idx[start..end]
    }

    /// Número de hijos directos, sin materializar la lista.
    #[inline]
    pub fn child_count(&self, idx: usize) -> usize {
        self.children(idx).len()
    }

    /// Sube por la jerarquía devolviendo los ancestros de `idx`, de la raíz del
    /// volumen hacia abajo, sin incluir a `idx`. Es la miga de pan del explorador.
    pub fn ancestors(&self, idx: usize) -> Vec<u32> {
        let mut chain = Vec::with_capacity(16);
        if idx >= self.entry_count() {
            return chain;
        }
        let mut curr = self.parent[idx];
        for _ in 0..Self::MAX_PATH_DEPTH {
            if curr == u32::MAX || (curr as usize) >= self.entry_count() {
                break;
            }
            chain.push(curr);
            let next = self.parent[curr as usize];
            if next == curr {
                break;
            }
            curr = next;
        }
        chain.reverse();
        chain
    }

    /// Raíces de volumen: entradas sin padre. Son el punto de partida del árbol
    /// lateral del explorador.
    pub fn volume_roots(&self) -> Vec<u32> {
        (0..self.entry_count())
            .filter(|&i| self.parent[i] == u32::MAX && self.is_alive(i))
            .map(|i| i as u32)
            .collect()
    }
}
