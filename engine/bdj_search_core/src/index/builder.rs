use super::arena::NameArena;
use super::ext_table::InternedExtensions;
use super::layout::{
    Header, SectionDescriptor, SectionId, SectionTable, CURRENT_VERSION, FLAG_DIR, FLAG_NON_ASCII,
    MAGIC, SIZE_IN_SIDE_TABLE,
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
    /// Tamaño de cada archivo, en cuatro bytes.
    ///
    /// El valor [`SIZE_IN_SIDE_TABLE`] significa «mira en `sizes_big`». La
    /// inmensa mayoría de los archivos caben aquí; guardar ocho bytes por
    /// entrada era pagar el caso raro en todas.
    pub sizes: Vec<u32>,
    /// Los pocos archivos de 4 GiB o más: identificador → tamaño real.
    ///
    /// Es un mapa durante la construcción porque la fase 2 rellena los tamaños
    /// por carpetas, en orden arbitrario; al publicar se ordena por
    /// identificador para poder buscarlo por bisección.
    pub sizes_big: std::collections::HashMap<u32, u64>,
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

/// Ordena por tramos de ocho bytes, bajando solo donde sigue habiendo empate.
///
/// Es una ordenación radix por posición (MSD) con dígitos de ocho bytes. En cada
/// nivel se ordena por el tramo `[desde, desde+8)` del nombre —una clave entera,
/// sin comparar cadenas— y solo se baja al tramo siguiente dentro de los grupos
/// que han quedado empatados.
///
/// # Por qué recursivo y no un solo tramo
///
/// Con un único tramo de ocho bytes, una biblioteca de DJ se ordena fatal:
/// quinientos archivos que empiezan por «Michael Jackson - …» comparten los ocho
/// primeros bytes, así que todos caen en el mismo grupo y hay que desempatarlos
/// comparando cadenas, que es justo lo que se quería evitar. Medido: con un solo
/// tramo, ordenar dos millones de entradas con ocho artistas distintos costaba
/// 3,1 s, **peor** que comparar cadenas desde el principio (1,65 s).
///
/// Bajando por tramos, cada nivel solo toca lo que sigue empatado, y dos nombres
/// solo se comparan de verdad cuando son iguales hasta el final.
fn ordenar_por_tramos<'a, F>(pares: &mut [(u64, u32)], desde: usize, trozo: &F)
where
    F: Fn(u32) -> &'a [u8] + Sync,
{
    if pares.len() <= 1 {
        return;
    }

    // Grupos pequeños: comparar directamente sale más barato que contar.
    const UMBRAL: usize = 24;
    if pares.len() <= UMBRAL {
        pares.sort_unstable_by(|&(_, a), &(_, b)| comparar_nombres(trozo(a), trozo(b)));
        return;
    }

    crate::sort::RadixSort::sort_pairs_slice(pares);

    let mut i = 0usize;
    while i < pares.len() {
        let mut j = i + 1;
        while j < pares.len() && pares[j].0 == pares[i].0 {
            j += 1;
        }
        if j - i > 1 {
            let siguiente = desde + 8;
            // Si ningún nombre del grupo llega al tramo siguiente, son iguales
            // hasta donde importa y no hay nada más que ordenar.
            let hay_mas = pares[i..j].iter().any(|&(_, id)| trozo(id).len() > siguiente);
            if hay_mas {
                for p in pares[i..j].iter_mut() {
                    p.0 = clave_de_tramo(trozo(p.1), siguiente);
                }
                ordenar_por_tramos(&mut pares[i..j], siguiente, trozo);
            }
        }
        i = j;
    }
}

/// Ocho bytes del nombre a partir de `desde`, en minúsculas, como entero.
#[inline]
fn clave_de_tramo(bytes: &[u8], desde: usize) -> u64 {
    let mut k = 0u64;
    for i in 0..8 {
        let b = bytes
            .get(desde + i)
            .map(|c| c.to_ascii_lowercase())
            .unwrap_or(0);
        k = (k << 8) | b as u64;
    }
    k
}

/// El orden de siempre: sin distinguir mayúsculas, y a igualdad, el más corto
/// primero.
#[inline]
fn comparar_nombres(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    for (x, y) in a.iter().zip(b.iter()) {
        let d = x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase());
        if d != std::cmp::Ordering::Equal {
            return d;
        }
    }
    a.len().cmp(&b.len())
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
            sizes_big: std::collections::HashMap::new(),
            mtimes: Vec::with_capacity(capacity),
            ctimes: Vec::with_capacity(capacity),
            alive: Vec::with_capacity(capacity.div_ceil(64)),
            // `name_order` no se reserva: se llena de una vez al ordenar, y
            // reservarlo aquí ocuparía cuatro bytes por archivo durante todo el
            // escaneo sin usarse.
            name_order: Vec::new(),
            arena: NameArena::with_capacity(capacity * 24),
            ext_table: InternedExtensions::new(),
            vol_table: VolumeTable::new(),
            generation: 1,
            install_id: [0u8; 16],
        }
    }

    /// Reserva la capacidad óptima según el perfil de memoria de la máquina (`build_batch`).
    /// En gama baja (4 GB) reserva ~1.9 M de entradas, evitando picos de RSS.
    pub fn with_tuning() -> Self {
        let batch = crate::tuning::Tuning::current().build_batch;
        Self::with_capacity(batch)
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
        self.sizes.push(Self::guardar_tamano(&mut self.sizes_big, id, size));
        self.mtimes.push(mtime);
        self.ctimes.push(ctime);

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

    /// Guarda un tamaño en cuatro bytes, apartando el que no quepa.
    ///
    /// Devuelve lo que hay que poner en la columna: el tamaño mismo, o la marca
    /// que dice «está en la tabla aparte». Un archivo que deja de ser grande
    /// —se trunca— sale de la tabla, para que no crezca sola.
    fn guardar_tamano(
        grandes: &mut std::collections::HashMap<u32, u64>,
        id: u32,
        size: u64,
    ) -> u32 {
        if size >= SIZE_IN_SIDE_TABLE as u64 {
            grandes.insert(id, size);
            SIZE_IN_SIDE_TABLE
        } else {
            grandes.remove(&id);
            size as u32
        }
    }

    /// Tamaño real de una entrada, mirando la tabla aparte si hace falta.
    pub fn size_at(&self, idx: u32) -> u64 {
        let i = idx as usize;
        match self.sizes.get(i) {
            Some(&SIZE_IN_SIDE_TABLE) => self.sizes_big.get(&idx).copied().unwrap_or(0),
            Some(&s) => s as u64,
            None => 0,
        }
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
            self.sizes[i] = Self::guardar_tamano(&mut self.sizes_big, idx, size);
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
        // `name_order` se genera al ordenar, así que aquí basta con vaciarlo:
        // dejar dentro identificadores que ya no existen apuntaría fuera de
        // rango en la próxima publicación.
        self.name_order.clear();

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

    /// Clave entera con la que se ordena un nombre sin compararlo como cadena.
    ///
    /// Son los ocho primeros bytes del nombre en minúsculas, empaquetados en un
    /// `u64` de mayor a menor peso. Comparar dos de estas claves como enteros da
    /// exactamente el mismo orden que comparar los ocho primeros bytes de los
    /// nombres; los nombres más cortos quedan antes que los que empiezan igual y
    /// siguen, porque los huecos se rellenan con ceros.
    #[inline]
    fn clave_de_nombre(bytes: &[u8]) -> u64 {
        let mut k = 0u64;
        for i in 0..8 {
            let b = bytes.get(i).map(|c| c.to_ascii_lowercase()).unwrap_or(0);
            k = (k << 8) | b as u64;
        }
        k
    }

    /// Calcula el orden alfabético de todas las entradas.
    ///
    /// # Por qué no se comparan cadenas
    ///
    /// La versión anterior hacía `par_sort_unstable_by` comparando los nombres
    /// byte a byte dentro del comparador. Sobre diez millones de entradas eso son
    /// unos 230 millones de comparaciones, y **cada una** salta a dos posiciones
    /// arbitrarias de una arena de varios cientos de megas: un fallo de caché por
    /// comparación, más el paso a minúsculas byte a byte.
    ///
    /// Medido sobre dos millones de entradas en dos núcleos: **1,65 s**. Es la
    /// operación más cara de todo el sistema, más cara que escribir el índice
    /// entero a disco (0,84 s), y se paga en cada compactación.
    ///
    /// # Cómo se hace ahora
    ///
    /// 1. Una pasada **secuencial** por la arena construyendo, para cada entrada,
    ///    una clave de ocho bytes. Secuencial porque los identificadores se
    ///    asignan en el mismo orden en que se escriben los nombres, así que
    ///    recorrerlos en orden recorre la arena de principio a fin.
    /// 2. Una ordenación radix de esos pares `(clave, id)` —la misma que ya usa
    ///    el motor para ordenar resultados—, que no compara nada: cuenta.
    /// 3. Solo dentro de los grupos que comparten los ocho primeros bytes se
    ///    comparan los nombres de verdad. En un disco real esos grupos son
    ///    pequeños; en el peor caso —diez mil archivos que empiezan igual— se
    ///    comparan diez mil, no diez millones.
    ///
    /// El resultado es **idéntico** al de la versión anterior; hay una prueba
    /// que lo comprueba contra el comparador de cadenas sobre nombres difíciles.
    pub fn compute_name_order(&mut self) {
        // Hasta que se ordena, `name_order` es la identidad: 0, 1, 2, 3…
        //
        // Se guardaba entrada por entrada durante todo el escaneo, cuatro bytes
        // por archivo para almacenar un número que ya se sabía. Sobre diez
        // millones de archivos son cuarenta megas de memoria ocupados durante el
        // escaneo entero para no aportar nada. Se genera aquí, que es el único
        // sitio donde hace falta.
        let n = self.count();
        self.name_order.clear();
        self.name_order.extend(0..n as u32);
        if n <= 1 {
            return;
        }

        let arena_bytes = self.arena.as_slice();
        let name_offs = &self.name_offs;
        let name_lens = &self.name_lens;

        let trozo = |id: u32| -> &[u8] {
            let i = id as usize;
            let off = name_offs[i] as usize;
            let len = name_lens[i] as usize;
            arena_bytes.get(off..off + len).unwrap_or(&[])
        };

        let mut pares: Vec<(u64, u32)> = self
            .name_order
            .iter()
            .map(|&id| (Self::clave_de_nombre(trozo(id)), id))
            .collect();

        ordenar_por_tramos(&mut pares, 0, &trozo);

        for (destino, (_, id)) in self.name_order.iter_mut().zip(pares) {
            *destino = id;
        }
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

        // La tabla de tamaños grandes, ordenada por identificador para poder
        // buscarla por bisección al leer.
        let mut grandes: Vec<(u32, u64)> =
            self.sizes_big.iter().map(|(&k, &v)| (k, v)).collect();
        grandes.sort_unstable_by_key(|p| p.0);
        let big_ids: Vec<u32> = grandes.iter().map(|p| p.0).collect();
        let big_vals: Vec<u64> = grandes.iter().map(|p| p.1).collect();

        assign_section(SectionId::Parent, self.parents.len() * 4);
        assign_section(SectionId::NameOff, self.name_offs.len() * 4);
        assign_section(SectionId::NameLen, self.name_lens.len());
        assign_section(SectionId::Flags, self.flags.len());
        assign_section(SectionId::ExtId, self.ext_ids.len() * 2);
        assign_section(SectionId::Volume, self.volumes.len());
        assign_section(SectionId::Size, self.sizes.len() * 4);
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
        assign_section(SectionId::SizeBigId, big_ids.len() * 4);
        assign_section(SectionId::SizeBigVal, big_vals.len() * 8);

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
        write_aligned(writer, bytemuck::cast_slice(&big_ids))?;
        write_aligned(writer, bytemuck::cast_slice(&big_vals))?;

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
