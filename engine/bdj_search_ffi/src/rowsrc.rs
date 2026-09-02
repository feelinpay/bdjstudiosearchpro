//! Lectura de filas en el espacio de identificadores unificado.
//!
//! Desde que el servicio publica la capa de cambios aparte del índice, un
//! resultado de búsqueda puede mezclar entradas de dos archivos: el índice base
//! y `overlay.bdjo`. El convenio es el mismo que usa el servicio por dentro:
//!
//! - `id < base_count` → la entrada vive en el índice base.
//! - `id >= base_count` → vive en la capa, en la posición `id - base_count`.
//!
//! Todo lo que la interfaz pide de una fila —nombre, carpeta, extensión, tamaño,
//! fecha, banderas y ruta completa— pasa por aquí, para que la regla esté escrita
//! una sola vez. Antes cada función de la superficie FFI indexaba la vista del
//! base directamente, y con la capa presente eso devolvía el archivo equivocado:
//! un identificador por encima de `base_count` caía fuera de rango y se leía como
//! cero, o peor, dentro de rango de otra columna.

use bdj_search_core::index::overlay::OverlayIndex;
use bdj_search_core::index::overlay_file::OverlaySnapshot;
use bdj_search_core::index::view::IndexView;

/// De dónde salen las filas de un resultado.
///
/// La capa es opcional: mientras el servicio no haya publicado ninguna —o
/// mientras la publicada sea de otra generación y se haya descartado— esto se
/// comporta exactamente igual que la vista del base sola.
pub struct RowSource<'a> {
    pub base: &'a IndexView<'a>,
    pub overlay: Option<&'a IndexView<'a>>,
    /// Índice unificado, necesario para reconstruir rutas que suben de la capa
    /// al base.
    pub overlay_ids: Option<&'a OverlayIndex>,
    pub base_count: u32,
}

impl<'a> RowSource<'a> {
    /// Solo el índice base.
    pub fn plain(base: &'a IndexView<'a>) -> Self {
        Self {
            base,
            overlay: None,
            overlay_ids: None,
            base_count: base.entry_count() as u32,
        }
    }

    /// Base más capa publicada.
    pub fn with_overlay(
        base: &'a IndexView<'a>,
        snapshot: &'a OverlaySnapshot,
        overlay_view: &'a IndexView<'a>,
        overlay_ids: Option<&'a OverlayIndex>,
    ) -> Self {
        Self {
            base,
            overlay: Some(overlay_view),
            overlay_ids,
            base_count: snapshot.base_count,
        }
    }

    /// Localiza un identificador unificado: en qué vista está y en qué posición.
    ///
    /// Devuelve `None` cuando el identificador cae fuera de las dos, que es lo
    /// que ocurre con un resultado guardado de antes de una compactación.
    fn locate(&self, id: u32) -> Option<(&IndexView<'a>, usize, bool)> {
        if id < self.base_count {
            let idx = id as usize;
            if idx < self.base.entry_count() {
                return Some((self.base, idx, false));
            }
            return None;
        }
        let vista = self.overlay?;
        let idx = (id - self.base_count) as usize;
        if idx < vista.entry_count() {
            Some((vista, idx, true))
        } else {
            None
        }
    }

    pub fn name(&self, id: u32) -> String {
        match self.locate(id) {
            Some((v, i, _)) => v.get_name(i).unwrap_or("").to_string(),
            None => String::new(),
        }
    }

    pub fn extension(&self, id: u32) -> String {
        match self.locate(id) {
            Some((v, i, _)) => v.get_extension(i).to_string(),
            None => String::new(),
        }
    }

    pub fn size(&self, id: u32) -> u64 {
        self.locate(id).map(|(v, i, _)| v.size[i]).unwrap_or(0)
    }

    pub fn mtime(&self, id: u32) -> u32 {
        self.locate(id).map(|(v, i, _)| v.mtime[i]).unwrap_or(0)
    }

    pub fn flags(&self, id: u32) -> u8 {
        self.locate(id).map(|(v, i, _)| v.flags[i]).unwrap_or(0)
    }

    pub fn is_dir(&self, id: u32) -> bool {
        self.locate(id).map(|(v, i, _)| v.is_dir(i)).unwrap_or(false)
    }

    /// Prefijo de montaje del volumen de una entrada.
    ///
    /// Se busca primero en la tabla de la vista donde está la entrada y después
    /// en la del base: la capa solo conoce los volúmenes que ha tocado desde la
    /// última compactación, así que la suya puede estar incompleta.
    fn mount_prefix(&self, view: &IndexView<'_>, idx: usize) -> String {
        let vol = view.volume[idx];
        view.vol_table
            .get(vol)
            .map(|v| v.mount_prefix.clone())
            .or_else(|| self.base.vol_table.get(vol).map(|v| v.mount_prefix.clone()))
            .unwrap_or_default()
    }

    /// Ruta completa de una fila.
    pub fn full_path(&self, id: u32) -> String {
        match self.locate(id) {
            None => String::new(),
            Some((v, i, false)) => {
                let _ = v;
                self.base.resolve_full_path(i)
            }
            Some((v, i, true)) => {
                // La cadena de padres de una entrada de la capa puede acabar en
                // una carpeta del base, así que la ruta hay que reconstruirla
                // con el índice unificado y no con la vista de la capa sola.
                match self.overlay_ids {
                    Some(oi) => {
                        let prefijo = self.mount_prefix(v, i);
                        oi.resolve_path(Some(self.base), id, &prefijo)
                    }
                    None => v.get_name(i).unwrap_or("").to_string(),
                }
            }
        }
    }

    /// Carpeta contenedora, que es lo que se pinta en la columna «Ruta».
    pub fn parent_path(&self, id: u32) -> String {
        match self.locate(id) {
            None => String::new(),
            Some((_, i, false)) => self.base.resolve_parent_path(i),
            Some((v, i, true)) => {
                let padre = v.parent[i];
                if padre == u32::MAX {
                    return self.mount_prefix(v, i);
                }
                match self.overlay_ids {
                    Some(oi) => {
                        let prefijo = self.mount_prefix(v, i);
                        oi.resolve_path(Some(self.base), padre, &prefijo)
                    }
                    None => String::new(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bdj_search_core::index::overlay_file::{OVERLAY_FILE_NAME, write_overlay};
    use bdj_search_core::index::{IndexBuilder, MmapIndex};
    use std::fs::File;
    use std::io::BufWriter;
    use tempfile::TempDir;

    #[test]
    fn una_fila_de_la_capa_se_lee_con_su_ruta_completa() {
        // El caso que se rompía: un identificador por encima de `base_count`
        // indexaba la vista del base y devolvía otro archivo o cadena vacía.
        let dir = TempDir::new().unwrap();
        let base_path = dir.path().join("index.bdjx");

        let mut b = IndexBuilder::new();
        b.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);
        let raiz = b.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
        let musica = b.add_entry(raiz, "Musica", true, false, false, 0, 0, 0, 0);
        b.add_entry(musica, "viejo.wav", false, false, false, 0, 11, 1, 1);
        {
            let f = File::create(&base_path).unwrap();
            let mut w = BufWriter::new(f);
            b.write_to(&mut w).unwrap();
        }
        let mmap = MmapIndex::open(&base_path).unwrap();
        let base_view = mmap.view().unwrap();
        let base_count = base_view.entry_count() as u32;

        let mut capa = OverlayIndex::with_base_count(base_count);
        capa.builder.vol_table = base_view.vol_table.clone();
        let nuevo = capa.add_entry(musica, "nuevo.wav", false, false, false, 0, 99, 7, 7);

        let cp = dir.path().join(OVERLAY_FILE_NAME);
        write_overlay(&mut capa, base_view.header.generation, 1, &cp).unwrap();
        let snap = OverlaySnapshot::open(&cp, base_view.header.generation).unwrap();
        let capa_view = snap.view().unwrap();
        let oi = snap.to_overlay_index(Some(&mmap));

        let src = RowSource::with_overlay(&base_view, &snap, &capa_view, oi.as_ref());

        // Fila del base.
        assert_eq!(src.name(2), "viejo.wav");
        assert_eq!(src.size(2), 11);
        assert_eq!(src.full_path(2), "C:\\Musica\\viejo.wav");

        // Fila de la capa.
        assert_eq!(src.name(nuevo), "nuevo.wav");
        assert_eq!(src.size(nuevo), 99);
        assert_eq!(src.extension(nuevo), "wav");
        assert_eq!(src.mtime(nuevo), 7);
        assert_eq!(src.full_path(nuevo), "C:\\Musica\\nuevo.wav");
        assert_eq!(src.parent_path(nuevo), "C:\\Musica");
    }

    #[test]
    fn un_identificador_fuera_de_rango_devuelve_vacio_y_no_revienta() {
        // Pasa de verdad: la interfaz puede pedir filas de un resultado anterior
        // a una compactación, cuyos identificadores ya no significan nada.
        let dir = TempDir::new().unwrap();
        let base_path = dir.path().join("index.bdjx");
        let mut b = IndexBuilder::new();
        b.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);
        b.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
        {
            let f = File::create(&base_path).unwrap();
            let mut w = BufWriter::new(f);
            b.write_to(&mut w).unwrap();
        }
        let mmap = MmapIndex::open(&base_path).unwrap();
        let view = mmap.view().unwrap();
        let src = RowSource::plain(&view);

        assert_eq!(src.name(9_999), "");
        assert_eq!(src.size(9_999), 0);
        assert_eq!(src.full_path(9_999), "");
        assert!(!src.is_dir(9_999));
    }
}
