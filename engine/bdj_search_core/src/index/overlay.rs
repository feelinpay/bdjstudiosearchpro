use super::builder::IndexBuilder;
use super::layout::{FLAG_DIR, FLAG_HIDDEN, FLAG_SYSTEM};
use super::mmap::MmapIndex;
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

/// Capa de cambios recientes sobre un índice base inmutable.
///
/// # Espacio de identificadores unificado
///
/// El índice base ocupa `0..base_count`. Las entradas de esta capa ocupan
/// `base_count..base_count + overlay_count`. Los padres se guardan **siempre**
/// en ese espacio unificado, de modo que un archivo nuevo puede colgar tanto de
/// una carpeta que ya estaba en el índice como de otra creada hace un segundo,
/// sin necesitar dos convenciones distintas.
///
/// Al compactar, ambos rangos se renumeran y los padres se traducen con el mapa
/// de reubicación. Antes esto no se hacía y cualquier borrado desplazaba los
/// índices dejando a los archivos colgando de la carpeta equivocada.
#[derive(Default)]
pub struct OverlayIndex {
    pub builder: IndexBuilder,
    /// Identificadores del índice base anulados: borrados, movidos o sustituidos
    /// por una versión nueva que vive en esta capa.
    pub tombstones: HashSet<u32>,
    /// Frontera entre el espacio de identificadores del base y el de esta capa.
    pub base_count: u32,
}

impl OverlayIndex {
    pub fn new() -> Self {
        Self {
            builder: IndexBuilder::new(),
            tombstones: HashSet::new(),
            base_count: 0,
        }
    }

    /// Ancla la capa a un índice base concreto.
    pub fn with_base_count(base_count: u32) -> Self {
        Self {
            builder: IndexBuilder::new(),
            tombstones: HashSet::new(),
            base_count,
        }
    }

    pub fn set_base_count(&mut self, base_count: u32) {
        self.base_count = base_count;
    }

    /// Añade una entrada. `parent` va en el espacio unificado.
    /// Devuelve el identificador unificado de la entrada nueva.
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
        let local = self.builder.add_entry(
            parent, name, is_dir, is_hidden, is_system, vol_id, size, mtime, ctime,
        );
        self.base_count + local
    }

    /// Marca una entrada del base como anulada.
    ///
    /// Un identificador de esta misma capa se ignora: una entrada creada y
    /// borrada entre dos compactaciones no llega a existir para nadie, y
    /// tratarla como lápida del base borraría a un tercero.
    pub fn mark_deleted(&mut self, unified_id: u32) {
        if unified_id < self.base_count {
            self.tombstones.insert(unified_id);
        }
    }

    pub fn overlay_count(&self) -> usize {
        self.builder.count()
    }

    pub fn tombstone_count(&self) -> usize {
        self.tombstones.len()
    }

    /// Hay trabajo pendiente que publicar.
    pub fn has_changes(&self) -> bool {
        self.overlay_count() > 0 || !self.tombstones.is_empty()
    }

    /// Umbral por volumen de cambios: el 5 % del base, con un mínimo de 500.
    pub fn should_compact(&self, base_count: usize) -> bool {
        if !self.has_changes() {
            return false;
        }
        if base_count == 0 {
            return true;
        }
        let threshold = (base_count / 20).max(500);
        self.overlay_count() + self.tombstones.len() >= threshold
    }

    /// Nombre y padre de un identificador unificado, venga del base o de la capa.
    fn entry_of<'a>(
        &'a self,
        base: Option<&'a super::view::IndexView<'a>>,
        id: u32,
    ) -> Option<(&'a str, u32)> {
        if id < self.base_count {
            let view = base?;
            let idx = id as usize;
            if idx >= view.entry_count() {
                return None;
            }
            Some((view.get_name(idx).unwrap_or(""), view.parent[idx]))
        } else {
            let local = id - self.base_count;
            if local as usize >= self.builder.count() {
                return None;
            }
            Some((self.builder.name_at(local), self.builder.parents[local as usize]))
        }
    }

    /// Reconstruye la ruta de un identificador unificado.
    ///
    /// La cadena de padres puede cruzar de la capa al base —un archivo nuevo
    /// dentro de una carpeta que ya existía— y también al revés. Sin esto no se
    /// puede saber dónde vive un archivo recién creado, y por tanto tampoco leer
    /// su tamaño y su fecha del disco.
    pub fn resolve_path(
        &self,
        base: Option<&super::view::IndexView<'_>>,
        id: u32,
        mount_prefix: &str,
    ) -> String {
        let mut segments: Vec<&str> = Vec::with_capacity(16);
        let mut curr = id;

        for _ in 0..super::view::IndexView::MAX_PATH_DEPTH {
            let Some((name, parent)) = self.entry_of(base, curr) else {
                break;
            };
            if parent == u32::MAX {
                break; // raíz del volumen: su nombre está en el prefijo
            }
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

    /// Funde base y capa de cambios en un índice nuevo y lo publica de forma
    /// atómica sobre `target_path`.
    pub fn compact(
        &mut self,
        base: Option<&MmapIndex>,
        target_path: &Path,
        new_generation: u64,
    ) -> io::Result<usize> {
        let mut new_builder = IndexBuilder::new();
        new_builder.generation = new_generation;
        // La tabla de volúmenes y el identificador de instalación se heredan.
        // Perderlos dejaba todas las rutas sin prefijo de montaje a partir de
        // la primera compactación.
        new_builder.vol_table = self.builder.vol_table.clone();
        new_builder.install_id = self.builder.install_id;

        // old id (unificado) -> nuevo id; u32::MAX = no sobrevive
        let mut remap: Vec<u32> = Vec::new();
        let mut old_parents: Vec<u32> = Vec::new();

        // 1. Entradas del base que sobreviven.
        if let Some(mmap_base) = base {
            let view = mmap_base
                .view()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

            if new_builder.vol_table.volumes.is_empty() {
                new_builder.vol_table = view.vol_table.clone();
            }
            new_builder.install_id = view.header.install_id;

            let count = view.entry_count();
            remap.resize(count, u32::MAX);

            for (idx, slot) in remap.iter_mut().enumerate().take(count) {
                if !view.is_alive(idx) || self.tombstones.contains(&(idx as u32)) {
                    continue;
                }
                let new_id = new_builder.add_entry(
                    u32::MAX, // el padre se traduce en la segunda pasada
                    view.get_name(idx).unwrap_or(""),
                    view.is_dir(idx),
                    (view.flags[idx] & FLAG_HIDDEN) != 0,
                    (view.flags[idx] & FLAG_SYSTEM) != 0,
                    view.volume[idx],
                    view.size[idx],
                    view.mtime[idx],
                    view.ctime[idx],
                );
                *slot = new_id;
                old_parents.push(view.parent[idx]);
            }
        }

        // 2. Entradas de la capa de cambios.
        let overlay_count = self.builder.count();
        if remap.len() < self.base_count as usize {
            remap.resize(self.base_count as usize, u32::MAX);
        }
        remap.resize(self.base_count as usize + overlay_count, u32::MAX);

        for idx in 0..overlay_count {
            let new_id = new_builder.add_entry(
                u32::MAX,
                self.builder.name_at(idx as u32),
                (self.builder.flags[idx] & FLAG_DIR) != 0,
                (self.builder.flags[idx] & FLAG_HIDDEN) != 0,
                (self.builder.flags[idx] & FLAG_SYSTEM) != 0,
                self.builder.volumes[idx],
                self.builder.sizes[idx],
                self.builder.mtimes[idx],
                self.builder.ctimes[idx],
            );
            remap[self.base_count as usize + idx] = new_id;
            old_parents.push(self.builder.parents[idx]);
        }

        // 3. Traducción de padres. Un padre que no sobrevivió deja a su hijo
        //    colgando de la raíz del volumen en vez de apuntar a un índice
        //    reutilizado por otra entrada.
        for (position, &old_parent) in old_parents.iter().enumerate() {
            let new_id = position as u32;
            let new_parent = if old_parent == u32::MAX {
                u32::MAX
            } else {
                remap.get(old_parent as usize).copied().unwrap_or(u32::MAX)
            };
            new_builder.set_parent(new_id, new_parent);
        }

        // 4. Publicación atómica: temporal, fsync y renombrado.
        let tmp_path = PathBuf::from(format!("{}.tmp", target_path.display()));
        let tmp_file = File::create(&tmp_path)?;
        let mut writer = BufWriter::with_capacity(2 * 1024 * 1024, tmp_file);
        let written = new_builder.write_to(&mut writer)?;
        writer.flush()?;
        writer.into_inner()?.sync_all()?;
        std::fs::rename(&tmp_path, target_path)?;

        // La capa queda vacía y anclada al nuevo base.
        let vol_table = std::mem::take(&mut self.builder.vol_table);
        self.builder = IndexBuilder::new();
        self.builder.vol_table = vol_table;
        self.tombstones.clear();
        self.base_count = new_builder.count() as u32;

        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn write_base(path: &Path, builder: &mut IndexBuilder) {
        let file = File::create(path).unwrap();
        let mut writer = BufWriter::new(file);
        builder.write_to(&mut writer).unwrap();
    }

    #[test]
    fn test_overlay_and_compaction() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        let mut overlay = OverlayIndex::new();
        overlay.builder.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);
        // Todo índice empieza por la raíz del volumen, que va sin nombre.
        let root = overlay.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
        let folder = overlay.add_entry(root, "FolderA", true, false, false, 0, 0, 100, 100);
        overlay.add_entry(folder, "Track1.wav", false, false, false, 0, 5000, 100, 100);

        overlay.compact(None, path, 1).unwrap();

        let mmap_index = MmapIndex::open(path).unwrap();
        let view = mmap_index.view().unwrap();
        assert_eq!(view.entry_count(), 3);
        assert_eq!(view.get_name(1), Some("FolderA"));
        assert_eq!(view.get_name(2), Some("Track1.wav"));
        assert_eq!(view.get_extension(2), "wav");
        assert_eq!(view.resolve_full_path(2), "C:\\FolderA\\Track1.wav");
    }

    #[test]
    fn test_compaction_preserves_volume_table() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        // Base con su tabla de volúmenes.
        let mut base = IndexBuilder::new();
        base.vol_table.add_or_update("E:\\", "USB Cabina", "exFAT", true);
        let root = base.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
        base.add_entry(root, "set.wav", false, false, false, 0, 10, 5, 5);
        write_base(path, &mut base);

        let mmap = MmapIndex::open(path).unwrap();
        let base_count = mmap.view().unwrap().entry_count() as u32;

        // Un solo cambio dispara la compactación.
        let mut overlay = OverlayIndex::with_base_count(base_count);
        overlay.add_entry(0, "nuevo.wav", false, false, false, 0, 20, 6, 6);

        let out = NamedTempFile::new().unwrap();
        overlay.compact(Some(&mmap), out.path(), 2).unwrap();

        let merged = MmapIndex::open(out.path()).unwrap();
        let view = merged.view().unwrap();
        assert_eq!(
            view.vol_table.get(0).map(|v| v.mount_prefix.as_str()),
            Some("E:\\"),
            "la compactación no debe perder la tabla de volúmenes"
        );
        assert_eq!(view.resolve_full_path(2), "E:\\nuevo.wav");
    }

    #[test]
    fn test_compaction_remaps_parents_after_deletions() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        // C:\  ->  Borrame\  ,  Music\  ->  set.wav
        let mut base = IndexBuilder::new();
        base.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);
        let root = base.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
        let doomed = base.add_entry(root, "Borrame", true, false, false, 0, 0, 0, 0);
        let music = base.add_entry(root, "Music", true, false, false, 0, 0, 0, 0);
        let track = base.add_entry(music, "set.wav", false, false, false, 0, 999, 7, 7);
        write_base(path, &mut base);

        let mmap = MmapIndex::open(path).unwrap();
        assert_eq!(mmap.view().unwrap().resolve_full_path(track as usize), "C:\\Music\\set.wav");

        // Se borra una carpeta anterior a "Music": todos los índices se desplazan.
        let mut overlay = OverlayIndex::with_base_count(4);
        overlay.mark_deleted(doomed);

        let out = NamedTempFile::new().unwrap();
        overlay.compact(Some(&mmap), out.path(), 2).unwrap();

        let merged = MmapIndex::open(out.path()).unwrap();
        let view = merged.view().unwrap();
        assert_eq!(view.entry_count(), 3);

        // La pista debe seguir dentro de Music, no haberse mudado a otra carpeta
        // por el desplazamiento de índices.
        let track_new = (0..view.entry_count())
            .find(|&i| view.get_name(i) == Some("set.wav"))
            .expect("la pista debe sobrevivir");
        assert_eq!(
            view.resolve_full_path(track_new),
            "C:\\Music\\set.wav",
            "la jerarquía debe sobrevivir al renumerado"
        );
    }

    #[test]
    fn test_overlay_entry_can_hang_from_an_overlay_folder() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        let mut base = IndexBuilder::new();
        base.vol_table.add_or_update("D:\\", "Musica", "NTFS", true);
        base.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
        write_base(path, &mut base);

        let mmap = MmapIndex::open(path).unwrap();

        // Carpeta y archivo creados ambos despues de la ultima compactacion.
        let mut overlay = OverlayIndex::with_base_count(1);
        let nueva = overlay.add_entry(0, "Sesion", true, false, false, 0, 0, 0, 0);
        assert_eq!(nueva, 1, "el id unificado sigue al del base");
        overlay.add_entry(nueva, "intro.wav", false, false, false, 0, 42, 0, 0);

        let out = NamedTempFile::new().unwrap();
        overlay.compact(Some(&mmap), out.path(), 2).unwrap();

        let merged = MmapIndex::open(out.path()).unwrap();
        let view = merged.view().unwrap();
        let track = (0..view.entry_count())
            .find(|&i| view.get_name(i) == Some("intro.wav"))
            .unwrap();
        assert_eq!(view.resolve_full_path(track), "D:\\Sesion\\intro.wav");
    }

    #[test]
    fn test_resolve_path_crosses_base_and_overlay() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        // Base: C:\ -> Music\
        let mut base = IndexBuilder::new();
        base.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);
        let root = base.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
        let music = base.add_entry(root, "Music", true, false, false, 0, 0, 0, 0);
        write_base(path, &mut base);

        let mmap = MmapIndex::open(path).unwrap();
        let view = mmap.view().unwrap();
        let base_count = view.entry_count() as u32;

        let mut overlay = OverlayIndex::with_base_count(base_count);

        // Archivo nuevo dentro de una carpeta que ya estaba en el base.
        let nuevo = overlay.add_entry(music, "recien.wav", false, false, false, 0, 0, 0, 0);
        assert_eq!(
            overlay.resolve_path(Some(&view), nuevo, "C:\\"),
            "C:\\Music\\recien.wav"
        );

        // Y una carpeta nueva con un archivo dentro, ambos en la capa.
        let sesion = overlay.add_entry(music, "Sesion", true, false, false, 0, 0, 0, 0);
        let intro = overlay.add_entry(sesion, "intro.wav", false, false, false, 0, 0, 0, 0);
        assert_eq!(
            overlay.resolve_path(Some(&view), intro, "C:\\"),
            "C:\\Music\\Sesion\\intro.wav"
        );

        // Una entrada del base se resuelve igual a traves de la capa.
        assert_eq!(overlay.resolve_path(Some(&view), music, "C:\\"), "C:\\Music");
    }

    #[test]
    fn test_resolve_path_does_not_hang_on_a_cycle() {
        let mut overlay = OverlayIndex::with_base_count(0);
        let a = overlay.add_entry(u32::MAX, "A", true, false, false, 0, 0, 0, 0);
        let b = overlay.add_entry(a, "B", true, false, false, 0, 0, 0, 0);
        // Ciclo deliberado entre dos entradas de la capa.
        overlay.builder.parents[a as usize] = b;

        let resolved = overlay.resolve_path(None, b, "C:\\");
        assert!(resolved.len() < 4096);
    }

    #[test]
    fn test_deleting_an_overlay_entry_does_not_tombstone_the_base() {
        let mut overlay = OverlayIndex::with_base_count(10);
        let nuevo = overlay.add_entry(0, "efimero.tmp", false, false, false, 0, 1, 0, 0);
        assert_eq!(nuevo, 10);

        overlay.mark_deleted(nuevo);
        assert_eq!(
            overlay.tombstone_count(),
            0,
            "borrar una entrada de la capa no debe anular una del base"
        );

        overlay.mark_deleted(3);
        assert_eq!(overlay.tombstone_count(), 1);
    }
}
