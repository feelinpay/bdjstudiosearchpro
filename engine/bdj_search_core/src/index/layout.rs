use bytemuck::{Pod, Zeroable};

pub const MAGIC: &[u8; 8] = b"BDJXIDX\0";

/// Versión 2 del formato.
///
/// Añade tres columnas respecto a la versión 1:
/// - `NameRank`: posición alfabética de cada entrada, la permutación inversa de
///   `NameOrder`. Permite ordenar por nombre en tiempo proporcional al número
///   de resultados en lugar de al tamaño del índice.
/// - `ChildOff` / `ChildIdx`: lista de hijos de cada carpeta en formato
///   comprimido por filas (CSR). Es lo que hace que entrar en una carpeta con
///   cincuenta mil archivos sea instantáneo y no toque el disco.
///
/// Un índice de la versión 1 se rechaza al abrirlo y el servicio lo reconstruye.
pub const CURRENT_VERSION: u32 = 2;
pub const NUM_SECTIONS: usize = 17;

/// Bytes de relleno para que la tabla de secciones ocupe un múltiplo de 64.
const SECTION_TABLE_PADDING: usize = {
    let used = NUM_SECTIONS * 16;
    let aligned = used.div_ceil(64) * 64;
    aligned - used
};

#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Header {
    pub magic: [u8; 8],
    pub version: u32,
    pub header_crc: u32,
    pub entry_count: u64,
    pub arena_len: u64,
    pub created_at: u64,
    pub generation: u64,
    pub install_id: [u8; 16],
}

impl Header {
    pub fn compute_crc(&self) -> u32 {
        // Compute checksum of fields from entry_count onwards (bytes 16..64)
        let bytes = bytemuck::bytes_of(self);
        xxhash_rust::xxh3::xxh3_64(&bytes[16..64]) as u32
    }

    pub fn is_valid(&self) -> bool {
        if &self.magic != MAGIC {
            return false;
        }
        if self.version != CURRENT_VERSION {
            return false;
        }
        if self.header_crc != self.compute_crc() {
            return false;
        }
        true
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct SectionDescriptor {
    pub offset: u64,
    pub len: u64,
}

#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SectionTable {
    pub sections: [SectionDescriptor; NUM_SECTIONS],
    pub _padding: [u8; SECTION_TABLE_PADDING],
}

impl Default for SectionTable {
    fn default() -> Self {
        Self {
            sections: [SectionDescriptor::default(); NUM_SECTIONS],
            _padding: [0u8; SECTION_TABLE_PADDING],
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionId {
    Parent = 0,
    NameOff = 1,
    NameLen = 2,
    Flags = 3,
    ExtId = 4,
    Volume = 5,
    Size = 6,
    Mtime = 7,
    Ctime = 8,
    Alive = 9,
    NameOrder = 10,
    NameArena = 11,
    ExtTable = 12,
    VolTable = 13,
    /// Posición alfabética de cada entrada (permutación inversa de NameOrder).
    NameRank = 14,
    /// Desplazamientos CSR: los hijos de `i` son `ChildIdx[ChildOff[i]..ChildOff[i+1]]`.
    ChildOff = 15,
    ChildIdx = 16,
}

pub const FLAG_DIR: u8 = 0x01;
pub const FLAG_HIDDEN: u8 = 0x02;
pub const FLAG_SYSTEM: u8 = 0x04;
pub const FLAG_SYMLINK: u8 = 0x08;
pub const FLAG_NON_ASCII: u8 = 0x10;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabla_de_secciones_alineada_a_64() {
        assert_eq!(std::mem::size_of::<SectionTable>() % 64, 0);
        assert_eq!(std::mem::size_of::<Header>(), 64);
    }
}
