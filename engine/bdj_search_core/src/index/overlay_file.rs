//! Publicación de la capa de cambios como archivo propio.
//!
//! # El problema que resuelve
//!
//! El servicio guarda los cambios recientes en una [`OverlayIndex`] que vive en
//! su memoria. La aplicación solo abría `index.bdjx`, así que el único camino
//! por el que un cambio del disco llegaba a la pantalla era **reescribir el
//! índice entero**. Sobre diez millones de entradas eso son 950 MB y unos 25
//! segundos: el usuario copiaba un archivo y no lo veía aparecer.
//!
//! Aquí la capa se publica aparte, en un archivo pequeño. Escribirlo cuesta
//! milisegundos porque solo contiene lo que ha cambiado desde la última
//! compactación, así que puede publicarse cada pocos cientos de milisegundos.
//! La compactación pasa a ser lo que debía haber sido siempre: mantenimiento en
//! segundo plano, no el camino por el que viajan los cambios.
//!
//! # Formato
//!
//! ```text
//!   0  magic "BDJOVL\0\0"        8 bytes
//!   8  version                   u32
//!  12  base_count                u32
//!  16  base_generation           u64
//!  24  overlay_generation        u64
//!  32  blob_len                  u32   longitud real del índice incrustado
//!  36  tomb_count                u32
//!  40  dead_count                u32
//!  44  reserved                  u32
//!  48  reserved                  [u8; 16]
//!  64  blob                      un `.bdjx` completo con las entradas nuevas
//!  ..  tombstones                [u32; tomb_count]
//!  ..  dead_overlay              [u32; dead_count]
//! ```
//!
//! El bloque incrustado empieza en el byte 64 —múltiplo de ocho— y el archivo se
//! mapea desde el principio de una página, así que las columnas de enteros de
//! dentro quedan alineadas y [`IndexView::from_bytes`] las puede leer sin
//! copiar. Las listas de longitud variable van **después** justamente para no
//! desplazar ese offset.
//!
//! Se escribe entero a un temporal y se renombra: la aplicación nunca mapea un
//! archivo a medio escribir.
//!
//! # Por qué `base_generation` es obligatoria
//!
//! Una capa solo tiene sentido sobre el índice base que la originó. Sus
//! identificadores por debajo de `base_count` son posiciones dentro de *ese*
//! base, y una compactación los renumera todos. Aplicar una capa vieja a un base
//! nuevo no da un resultado incompleto: da uno **incorrecto**, con archivos
//! colgando de la carpeta equivocada. Por eso el lector compara generaciones y
//! descarta la capa si no encajan, en vez de intentar aprovecharla.

use super::mmap::MmapIndex;
use super::overlay::OverlayIndex;
use super::view::{IndexView, ViewError};
use memmap2::Mmap;
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

/// Identifica el archivo de capa. Distinto del de un índice para que abrir uno
/// donde se espera el otro falle en el primer byte y no a mitad de las tablas.
pub const OVERLAY_MAGIC: &[u8; 8] = b"BDJOVL\0\0";

pub const OVERLAY_VERSION: u32 = 1;

/// Dónde empieza el índice incrustado. Múltiplo de ocho, por alineación.
const BLOB_OFFSET: usize = 64;

/// Nombre del archivo, junto a `index.bdjx`.
pub const OVERLAY_FILE_NAME: &str = "overlay.bdjo";

/// Ruta de la capa que acompaña a un índice dado.
pub fn overlay_path_for(index_path: &Path) -> PathBuf {
    index_path.with_file_name(OVERLAY_FILE_NAME)
}

#[derive(Debug, PartialEq, Eq)]
pub enum OverlayFileError {
    TooSmall,
    BadMagic,
    BadVersion(u32),
    Truncated,
    /// La capa se escribió sobre otro índice base y no puede aplicarse a este.
    GenerationMismatch {
        expected: u64,
        found: u64,
    },
    View(ViewError),
}

impl std::fmt::Display for OverlayFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooSmall => write!(f, "el archivo de capa es más corto que su cabecera"),
            Self::BadMagic => write!(f, "el archivo no es una capa de cambios"),
            Self::BadVersion(v) => write!(f, "versión de capa desconocida: {v}"),
            Self::Truncated => write!(f, "el archivo de capa está truncado"),
            Self::GenerationMismatch { expected, found } => write!(
                f,
                "la capa es de la generación {found} y el índice va por la {expected}"
            ),
            Self::View(e) => write!(f, "el índice incrustado en la capa no es válido: {e}"),
        }
    }
}

impl std::error::Error for OverlayFileError {}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn read_u64(bytes: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(b)
}

/// Publica la capa de forma atómica junto al índice.
///
/// `base_generation` es la generación del `index.bdjx` sobre el que se apoyan
/// estos cambios; `overlay_generation` sube en cada publicación y es lo que
/// permite a la aplicación saber que hay algo nuevo sin releer el archivo.
///
/// Devuelve los bytes escritos.
pub fn write_overlay(
    overlay: &mut OverlayIndex,
    base_generation: u64,
    overlay_generation: u64,
    path: &Path,
) -> io::Result<usize> {
    // El índice incrustado se serializa con el mismo escritor que el base: es un
    // `.bdjx` normal y corriente, solo que diminuto. Reusarlo evita mantener dos
    // serializadores que puedan divergir.
    let mut blob = Vec::with_capacity(64 * 1024);
    overlay.builder.write_to(&mut blob)?;

    let mut tombs: Vec<u32> = overlay.tombstones.iter().copied().collect();
    let mut deads: Vec<u32> = overlay.dead_overlay.iter().copied().collect();
    // Ordenadas para que el lector pueda buscar por bisección si algún día hay
    // muchas, y para que dos publicaciones del mismo estado den el mismo archivo.
    tombs.sort_unstable();
    deads.sort_unstable();

    let mut header = [0u8; BLOB_OFFSET];
    header[0..8].copy_from_slice(OVERLAY_MAGIC);
    header[8..12].copy_from_slice(&OVERLAY_VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&overlay.base_count.to_le_bytes());
    header[16..24].copy_from_slice(&base_generation.to_le_bytes());
    header[24..32].copy_from_slice(&overlay_generation.to_le_bytes());
    header[32..36].copy_from_slice(&(blob.len() as u32).to_le_bytes());
    header[36..40].copy_from_slice(&(tombs.len() as u32).to_le_bytes());
    header[40..44].copy_from_slice(&(deads.len() as u32).to_le_bytes());

    // El bloque se rellena hasta un múltiplo de cuatro para que las listas de
    // enteros que van detrás queden alineadas.
    let padding = (4 - (blob.len() % 4)) % 4;

    let tmp_path = PathBuf::from(format!("{}.tmp", path.display()));
    {
        let file = File::create(&tmp_path)?;
        let mut w = BufWriter::with_capacity(256 * 1024, file);
        w.write_all(&header)?;
        w.write_all(&blob)?;
        w.write_all(&vec![0u8; padding])?;
        for t in &tombs {
            w.write_all(&t.to_le_bytes())?;
        }
        for d in &deads {
            w.write_all(&d.to_le_bytes())?;
        }
        w.flush()?;
        w.into_inner()
            .map_err(|e| io::Error::other(e.to_string()))?
            .sync_all()?;
    }
    std::fs::rename(&tmp_path, path)?;

    Ok(BLOB_OFFSET + blob.len() + padding + tombs.len() * 4 + deads.len() * 4)
}

/// Borra la capa publicada.
///
/// Se llama justo después de compactar: en ese momento sus cambios ya están
/// dentro del índice base, y dejarla en disco haría que la aplicación los
/// contase dos veces.
pub fn remove_overlay(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Capa publicada, mapeada en memoria y lista para consultar.
///
/// Contiene la vista del índice incrustado —las entradas nuevas— y los conjuntos
/// de identificadores anulados. No sabe nada del índice base: quien la use tiene
/// que aportarlo.
pub struct OverlaySnapshot {
    mmap: Mmap,
    blob_range: (usize, usize),
    pub base_count: u32,
    pub base_generation: u64,
    pub overlay_generation: u64,
    /// Identificadores **del base** que ya no están vivos.
    pub tombstones: HashSet<u32>,
    /// Identificadores locales **de esta capa** que se crearon y borraron antes
    /// de publicarse.
    pub dead: HashSet<u32>,
}

impl std::fmt::Debug for OverlaySnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OverlaySnapshot")
            .field("base_count", &self.base_count)
            .field("base_generation", &self.base_generation)
            .field("overlay_generation", &self.overlay_generation)
            .field("entries", &self.entry_count())
            .field("tombstones", &self.tombstones.len())
            .field("dead", &self.dead.len())
            .finish()
    }
}

impl OverlaySnapshot {
    /// Abre la capa y comprueba que corresponde al índice base indicado.
    ///
    /// `expected_base_generation` es la generación del `index.bdjx` que la
    /// aplicación tiene mapeado. Si no coincide se devuelve
    /// [`OverlayFileError::GenerationMismatch`] y **hay que ignorar la capa**:
    /// justo después de compactar existe una ventana en la que el archivo de
    /// capa todavía es el viejo, y aplicarlo al base nuevo colocaría archivos en
    /// carpetas equivocadas.
    pub fn open(path: &Path, expected_base_generation: u64) -> Result<Self, OverlayFileError> {
        let file = File::open(path).map_err(|_| OverlayFileError::TooSmall)?;
        // Seguridad: el archivo se publica con un renombrado atómico, así que
        // nunca se mapea uno a medio escribir.
        let mmap = unsafe { Mmap::map(&file) }.map_err(|_| OverlayFileError::TooSmall)?;
        Self::from_mmap(mmap, expected_base_generation)
    }

    fn from_mmap(mmap: Mmap, expected_base_generation: u64) -> Result<Self, OverlayFileError> {
        if mmap.len() < BLOB_OFFSET {
            return Err(OverlayFileError::TooSmall);
        }
        if &mmap[0..8] != OVERLAY_MAGIC {
            return Err(OverlayFileError::BadMagic);
        }
        let version = read_u32(&mmap, 8);
        if version != OVERLAY_VERSION {
            return Err(OverlayFileError::BadVersion(version));
        }

        let base_count = read_u32(&mmap, 12);
        let base_generation = read_u64(&mmap, 16);
        let overlay_generation = read_u64(&mmap, 24);
        let blob_len = read_u32(&mmap, 32) as usize;
        let tomb_count = read_u32(&mmap, 36) as usize;
        let dead_count = read_u32(&mmap, 40) as usize;

        if base_generation != expected_base_generation {
            return Err(OverlayFileError::GenerationMismatch {
                expected: expected_base_generation,
                found: base_generation,
            });
        }

        let padding = (4 - (blob_len % 4)) % 4;
        let tomb_start = BLOB_OFFSET + blob_len + padding;
        let dead_start = tomb_start + tomb_count * 4;
        let end = dead_start + dead_count * 4;
        if mmap.len() < end {
            return Err(OverlayFileError::Truncated);
        }

        let mut tombstones = HashSet::with_capacity(tomb_count);
        for i in 0..tomb_count {
            tombstones.insert(read_u32(&mmap, tomb_start + i * 4));
        }
        let mut dead = HashSet::with_capacity(dead_count);
        for i in 0..dead_count {
            dead.insert(read_u32(&mmap, dead_start + i * 4));
        }

        // Se valida aquí para no devolver una capa que reventará al primer uso.
        IndexView::from_bytes(&mmap[BLOB_OFFSET..BLOB_OFFSET + blob_len])
            .map_err(OverlayFileError::View)?;

        Ok(Self {
            mmap,
            blob_range: (BLOB_OFFSET, BLOB_OFFSET + blob_len),
            base_count,
            base_generation,
            overlay_generation,
            tombstones,
            dead,
        })
    }

    /// Vista de las entradas nuevas.
    ///
    /// Ojo: los padres de estas entradas están en el **espacio unificado** —un
    /// archivo nuevo puede colgar de una carpeta que ya estaba en el base—, así
    /// que `resolve_full_path` sobre esta vista sola no sirve. Para la ruta hay
    /// que usar [`OverlayIndex::resolve_path`] con el base delante.
    pub fn view(&self) -> Result<IndexView<'_>, ViewError> {
        IndexView::from_bytes(&self.mmap[self.blob_range.0..self.blob_range.1])
    }

    /// Cuántas entradas nuevas trae, vivas o no.
    pub fn entry_count(&self) -> usize {
        self.view().map(|v| v.entry_count()).unwrap_or(0)
    }

    /// ¿Sigue viva esta entrada del índice base?
    pub fn base_is_alive(&self, base_id: u32) -> bool {
        !self.tombstones.contains(&base_id)
    }

    /// ¿Sigue viva esta entrada de la capa? `local` es su posición dentro de la
    /// vista incrustada, no el identificador unificado.
    pub fn overlay_is_alive(&self, local: u32) -> bool {
        !self.dead.contains(&local)
    }

    /// Reconstruye una [`OverlayIndex`] equivalente.
    ///
    /// La usa la aplicación para resolver rutas que cruzan de la capa al base,
    /// que es lo que la vista incrustada por sí sola no puede hacer.
    pub fn to_overlay_index(&self, base: Option<&MmapIndex>) -> Option<OverlayIndex> {
        let view = self.view().ok()?;
        let mut out = OverlayIndex::with_base_count(self.base_count);
        out.builder.vol_table = view.vol_table.clone();
        if let Some(b) = base
            && let Ok(bv) = b.view()
        {
            // La tabla de volúmenes del base manda: la capa solo conoce los
            // volúmenes que ha tocado desde la última compactación.
            if out.builder.vol_table.volumes.is_empty() {
                out.builder.vol_table = bv.vol_table.clone();
            }
        }
        for idx in 0..view.entry_count() {
            out.builder.add_entry(
                view.parent[idx],
                view.get_name(idx).unwrap_or(""),
                view.is_dir(idx),
                (view.flags[idx] & super::layout::FLAG_HIDDEN) != 0,
                (view.flags[idx] & super::layout::FLAG_SYSTEM) != 0,
                view.volume[idx],
                view.size[idx],
                view.mtime[idx],
                view.ctime[idx],
            );
        }
        // `add_entry` no conserva el padre tal cual cuando apunta al espacio del
        // base, así que se reescriben después.
        for idx in 0..view.entry_count() {
            out.builder.set_parent(idx as u32, view.parent[idx]);
        }
        out.tombstones = self.tombstones.clone();
        out.dead_overlay = self.dead.clone();
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::IndexBuilder;
    use tempfile::TempDir;

    fn capa_de_prueba(base_count: u32) -> OverlayIndex {
        let mut o = OverlayIndex::with_base_count(base_count);
        o.builder.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);
        o
    }

    #[test]
    fn una_capa_publicada_se_relee_igual() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(OVERLAY_FILE_NAME);

        let mut capa = capa_de_prueba(1000);
        capa.add_entry(0, "recien.wav", false, false, false, 0, 4242, 77, 77);
        capa.add_entry(0, "Sesion", true, false, false, 0, 0, 88, 88);
        capa.mark_deleted(17);
        capa.mark_deleted(41);

        write_overlay(&mut capa, 7, 1, &path).unwrap();

        let leida = OverlaySnapshot::open(&path, 7).unwrap();
        assert_eq!(leida.base_count, 1000);
        assert_eq!(leida.base_generation, 7);
        assert_eq!(leida.overlay_generation, 1);
        assert_eq!(leida.entry_count(), 2);
        assert!(leida.tombstones.contains(&17));
        assert!(leida.tombstones.contains(&41));
        assert!(!leida.base_is_alive(17));
        assert!(leida.base_is_alive(18));

        let vista = leida.view().unwrap();
        assert_eq!(vista.get_name(0), Some("recien.wav"));
        assert_eq!(vista.size[0], 4242);
        assert_eq!(vista.get_name(1), Some("Sesion"));
        assert!(vista.is_dir(1));
    }

    #[test]
    fn una_capa_de_otra_generacion_se_rechaza() {
        // Es la comprobación que evita el fallo silencioso: aplicar una capa
        // vieja a un base recién compactado no da un resultado incompleto, da
        // uno incorrecto, porque la compactación renumera todas las entradas.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(OVERLAY_FILE_NAME);

        let mut capa = capa_de_prueba(10);
        capa.add_entry(0, "algo.txt", false, false, false, 0, 1, 0, 0);
        write_overlay(&mut capa, 3, 1, &path).unwrap();

        match OverlaySnapshot::open(&path, 4) {
            Err(OverlayFileError::GenerationMismatch { expected, found }) => {
                assert_eq!(expected, 4);
                assert_eq!(found, 3);
            }
            Ok(_) => panic!("una capa de otra generación no debe aceptarse"),
            Err(otro) => panic!("debería rechazarse por generación, y falló con: {otro}"),
        }
    }

    #[test]
    fn una_capa_vacia_es_valida() {
        // El servicio publica una capa vacía justo después de compactar, para
        // que la aplicación deje de aplicar la anterior sin tener que esperar a
        // que el archivo desaparezca.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(OVERLAY_FILE_NAME);

        let mut capa = capa_de_prueba(500);
        write_overlay(&mut capa, 2, 9, &path).unwrap();

        let leida = OverlaySnapshot::open(&path, 2).unwrap();
        assert_eq!(leida.entry_count(), 0);
        assert!(leida.tombstones.is_empty());
        assert!(leida.dead.is_empty());
    }

    #[test]
    fn un_archivo_que_no_es_una_capa_se_rechaza_en_el_primer_byte() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("basura.bdjo");
        std::fs::write(&path, vec![b'X'; 4096]).unwrap();
        assert_eq!(
            OverlaySnapshot::open(&path, 0).unwrap_err(),
            OverlayFileError::BadMagic
        );
    }

    #[test]
    fn una_capa_truncada_se_rechaza_en_vez_de_leer_basura() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(OVERLAY_FILE_NAME);

        let mut capa = capa_de_prueba(100);
        capa.add_entry(0, "x.wav", false, false, false, 0, 1, 0, 0);
        capa.mark_deleted(5);
        write_overlay(&mut capa, 1, 1, &path).unwrap();

        // Se recortan los últimos bytes: la lista de lápidas queda a medias.
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() - 2]).unwrap();

        assert_eq!(
            OverlaySnapshot::open(&path, 1).unwrap_err(),
            OverlayFileError::Truncated
        );
    }

    #[test]
    fn los_padres_del_base_sobreviven_al_viaje_por_disco() {
        // Un archivo nuevo dentro de una carpeta que ya estaba en el índice: su
        // padre es un identificador del base, no de la capa. Si al releer se
        // perdiera, el archivo aparecería colgando de la raíz.
        let dir = TempDir::new().unwrap();
        let base_path = dir.path().join("index.bdjx");
        let capa_path = dir.path().join(OVERLAY_FILE_NAME);

        let mut base = IndexBuilder::new();
        base.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);
        let raiz = base.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
        let musica = base.add_entry(raiz, "Musica", true, false, false, 0, 0, 0, 0);
        {
            let f = File::create(&base_path).unwrap();
            let mut w = BufWriter::new(f);
            base.write_to(&mut w).unwrap();
        }
        let mmap_base = MmapIndex::open(&base_path).unwrap();
        let base_count = mmap_base.view().unwrap().entry_count() as u32;

        let mut capa = OverlayIndex::with_base_count(base_count);
        capa.builder.vol_table = base.vol_table.clone();
        let nuevo = capa.add_entry(musica, "intro.wav", false, false, false, 0, 9, 0, 0);
        assert_eq!(nuevo, base_count);

        write_overlay(&mut capa, 1, 1, &capa_path).unwrap();

        let leida = OverlaySnapshot::open(&capa_path, 1).unwrap();
        let vista = leida.view().unwrap();
        assert_eq!(
            vista.parent[0], musica,
            "el padre debe seguir apuntando a la carpeta del base"
        );

        // Y la ruta completa tiene que poder reconstruirse cruzando las dos.
        let reconstruida = leida.to_overlay_index(Some(&mmap_base)).unwrap();
        let base_view = mmap_base.view().unwrap();
        assert_eq!(
            reconstruida.resolve_path(Some(&base_view), base_count, "C:\\"),
            "C:\\Musica\\intro.wav"
        );
    }
}
