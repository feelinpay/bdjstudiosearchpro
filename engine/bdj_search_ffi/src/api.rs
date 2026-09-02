//! Superficie que la aplicación Flutter ve del motor.
//!
//! Tres bloques, en este orden: consultar el índice, navegarlo, y modificar el
//! disco. Los dos primeros no escriben nada; el tercero delega en
//! `bdj_search_fsops`, que corre en sus propios hilos y nunca bloquea a quien
//! llama.
//!
//! Todo lo que cruza esta frontera usa tipos simples —enteros, cadenas y
//! vectores— porque son los que el generador traduce sin ambigüedad. Las
//! decisiones se pasan como códigos numéricos y no como enumerados, para que un
//! cambio de nombre en el motor no rompa la interfaz en silencio.

use crate::diagnostics::{IndexProblem, classify};
use crate::service;
use crate::rowsrc::RowSource;
use bdj_search_core::index::{MmapIndex, OverlayIndex, OverlaySnapshot, overlay_path_for};
use bdj_search_core::search::engine::DEFAULT_LIMIT;
use bdj_search_core::search::{Engine, SearchResult, search_merged};
use bdj_search_core::tuning::Tuning;
use bdj_search_fsops::{
    ConflictDecision, ConflictPolicy, FileOpManager, OpKind, OpRequest, PathChange,
};
use bdj_search_ipc::{IpcCommand, IpcEvent, LocalChange};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

// ───────────────────────────── Tipos de salida ─────────────────────────────

pub struct SearchStatusFfi {
    pub ready_count: u32,
    pub total_count: u32,
    pub is_complete: bool,
    pub generation: u64,
    pub elapsed_ms: u64,
}

pub struct RowBatchFfi {
    pub generation: u64,
    pub offset: u32,
    pub count: u32,
    pub names: Vec<String>,
    pub paths: Vec<String>,
    pub extensions: Vec<String>,
    pub sizes: Vec<u64>,
    pub mtimes: Vec<u32>,
    pub flags: Vec<u8>,
}

/// Estado real del motor, para que la interfaz pueda decir la verdad.
pub struct EngineStatusFfi {
    /// Ruta del índice que se está mirando.
    pub index_path: String,
    pub file_exists: bool,
    pub file_size: u64,
    /// Cierto solo si el índice está mapeado y es utilizable.
    pub is_open: bool,
    pub generation: u64,
    pub entry_count: u64,
    /// Código de [`IndexProblem`]: 0 correcto, 1 no existe, 2 formato antiguo,
    /// 3 dañado, 4 sin permiso, 5 otro.
    pub problem: u8,
    /// Texto listo para mostrar.
    pub message: String,
}

/// Un tramo de la miga de pan del explorador.
pub struct CrumbFfi {
    pub name: String,
    pub path: String,
}

/// Estado de una operación de archivo en curso.
pub struct FileOpFfi {
    pub id: u64,
    /// 0 crear carpeta, 1 renombrar, 2 copiar, 3 mover, 4 duplicar, 5 papelera.
    pub kind: u8,
    /// 0 planificando, 1 en marcha, 2 esperando respuesta a un conflicto,
    /// 3 cancelando, 4 hecho, 5 cancelado, 6 fallido.
    pub state: u8,
    pub total_items: u64,
    pub done_items: u64,
    pub total_bytes: u64,
    pub done_bytes: u64,
    pub current: String,
    pub errors: Vec<String>,
    pub can_undo: bool,
    pub elapsed_ms: u64,
    /// Cierto cuando `state == 2` y hay que preguntar al usuario.
    pub has_conflict: bool,
    pub conflict_source: String,
    pub conflict_destination: String,
    pub conflict_source_size: u64,
    pub conflict_destination_size: u64,
    pub conflict_source_mtime: u32,
    pub conflict_destination_mtime: u32,
}

/// Ajustes derivados de la máquina, para que la interfaz también se adapte.
pub struct TuningFfi {
    /// 0 baja, 1 media, 2 alta.
    pub tier: u8,
    pub tier_name: String,
    pub cores: u32,
    pub memory_mb: u64,
    pub search_threads: u32,
    /// Filas que conviene pedir de entrada.
    pub initial_limit: u32,
    /// Páginas de filas que conviene mantener en memoria.
    pub cached_pages: u32,
}

/// Lo que el servicio dice de sí mismo.
pub struct ServiceStatusFfi {
    pub reachable: bool,
    pub indexing_enabled: bool,
    pub license_active: bool,
    pub generation: u64,
    pub entry_count: u64,
    pub volume_prefixes: Vec<String>,
    pub volume_labels: Vec<String>,
    pub volume_fs_types: Vec<String>,
    pub volume_connected: Vec<bool>,
    pub volume_indexed: Vec<bool>,
    pub volume_entry_counts: Vec<u64>,
}

// ───────────────────────────── Estado del motor ─────────────────────────────

/// La última consulta resuelta, con lo necesario para poder repetirla.
struct LastQuery {
    result: SearchResult,
    /// Texto de la consulta, o la ruta de la carpeta si se está navegando.
    query: String,
    /// Carpeta a la que se acotó la búsqueda. Vacío significa todo el equipo.
    scope: String,
    sort_col: u8,
    ascending: bool,
    /// Cierto cuando el resultado son los hijos de una carpeta y no una búsqueda.
    is_browse: bool,
}

static ENGINE: LazyLock<Engine> = LazyLock::new(Engine::new);
static FSOPS: LazyLock<FileOpManager> = LazyLock::new(|| {
    // Aquí es donde se compone la aplicación: el gestor de archivos no sabe
    // nada del motor, así que es esta capa la que le pasa el tamaño de bloque
    // que corresponde a la máquina.
    bdj_search_fsops::set_copy_chunk(Tuning::current().copy_chunk);
    FileOpManager::new()
});
pub(crate) static MMAP_INDEX: Mutex<Option<MmapIndex>> = Mutex::new(None);
static LAST: Mutex<Option<LastQuery>> = Mutex::new(None);
static INDEX_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
static INDEX_GENERATION: AtomicU64 = AtomicU64::new(0);
/// Último fallo al abrir el índice, para poder explicarlo.
static LAST_PROBLEM: Mutex<Option<(u8, String)>> = Mutex::new(None);

/// Capa de cambios publicada por el servicio, mapeada.
///
/// Es lo que hace que un archivo copiado hace un segundo aparezca en la lista.
/// Antes la aplicación solo abría `index.bdjx`, así que el único camino por el
/// que un cambio llegaba a la pantalla era una reescritura completa del índice:
/// unos veinticinco segundos sobre diez millones de entradas.
static OVERLAY: Mutex<Option<OverlaySnapshot>> = Mutex::new(None);
/// La misma capa como índice unificado, para reconstruir rutas que suben de la
/// capa a una carpeta que ya estaba en el base.
static OVERLAY_IDS: Mutex<Option<OverlayIndex>> = Mutex::new(None);
/// Número de publicación de la capa que hay mapeada ahora mismo.
static OVERLAY_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Lee el número de publicación de la capa sin mapear el archivo entero.
///
/// Se consulta cada pocas decenas de milisegundos mientras la interfaz espera
/// cambios, así que tiene que costar lo que leer treinta y dos bytes y no lo que
/// mapear y validar un índice.
fn overlay_generation_on_disk(index_path: &Path) -> Option<u64> {
    use std::io::Read;
    let mut f = std::fs::File::open(overlay_path_for(index_path)).ok()?;
    let mut cabecera = [0u8; 32];
    f.read_exact(&mut cabecera).ok()?;
    if &cabecera[0..8] != b"BDJOVL\0\0" {
        return None;
    }
    Some(u64::from_le_bytes(cabecera[24..32].try_into().ok()?))
}

/// Vuelve a mapear la capa si el servicio ha publicado una versión nueva.
///
/// `base_generation` es la del índice que hay abierto. Una capa de otra
/// generación **se descarta**: justo después de compactar existe una ventana en
/// la que el archivo de capa todavía es el anterior, y sus identificadores por
/// debajo de `base_count` apuntan a posiciones que la compactación acaba de
/// renumerar. Aplicarla no daría un resultado incompleto sino uno incorrecto,
/// con archivos colgando de carpetas equivocadas.
fn reload_overlay(index_path: &Path, base_generation: u64, forzar: bool) -> bool {
    let en_disco = overlay_generation_on_disk(index_path);
    let actual = OVERLAY_GENERATION.load(Ordering::SeqCst);

    match en_disco {
        Some(g) if !forzar && g == actual => return false,
        None => {
            // No hay capa publicada: si teníamos una, se suelta.
            let habia = OVERLAY.lock().take().is_some();
            *OVERLAY_IDS.lock() = None;
            OVERLAY_GENERATION.store(0, Ordering::SeqCst);
            return habia;
        }
        _ => {}
    }

    let ruta = overlay_path_for(index_path);
    match OverlaySnapshot::open(&ruta, base_generation) {
        Ok(snap) => {
            let generacion = snap.overlay_generation;
            let ids = {
                let base = MMAP_INDEX.lock();
                snap.to_overlay_index(base.as_ref())
            };
            *OVERLAY_IDS.lock() = ids;
            *OVERLAY.lock() = Some(snap);
            OVERLAY_GENERATION.store(generacion, Ordering::SeqCst);
            true
        }
        Err(e) => {
            // Lo normal aquí es un desajuste de generación durante la ventana de
            // una compactación; se resuelve solo en la siguiente publicación.
            tracing_no_op(&e);
            let habia = OVERLAY.lock().take().is_some();
            *OVERLAY_IDS.lock() = None;
            OVERLAY_GENERATION.store(0, Ordering::SeqCst);
            habia
        }
    }
}

/// La capa no encajaba. No es un error que deba llegar al usuario: se corrige
/// sola en cuanto el servicio publique la siguiente.
fn tracing_no_op(_e: &bdj_search_core::index::OverlayFileError) {}

pub fn ping() -> String {
    "pong from BDJ Search Pro Rust Engine".to_string()
}

/// Dónde vive el índice.
/// Ruta por defecto del índice en macOS: la carpeta de soporte por usuario,
/// igual que la que usa el servicio (y que Sample Pad ya probó en producción).
/// `/Library` queda secundario, solo para instalaciones de sistema.
#[cfg(target_os = "macos")]
fn default_macos_index_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("BDJ Studio")
            .join("Search Pro")
            .join("index.bdjx");
    }
    PathBuf::from("/Library/Application Support/BDJ Studio/Search Pro/index.bdjx")
}

///
/// Debe coincidir exactamente con lo que escribe el servicio: si no, la
/// aplicación mira en un sitio y el servicio indexa en otro, y el síntoma es
/// una aplicación que nunca encuentra nada.
fn resolve_default_index_path() -> PathBuf {
    if let Ok(program_data) = std::env::var("ProgramData") {
        let p = PathBuf::from(program_data)
            .join("BDJ Studio")
            .join("Search Pro")
            .join("index.bdjx");
        if p.exists() {
            return p;
        }
    }
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(local_app_data)
            .join("BDJ Studio")
            .join("Search Pro")
            .join("index.bdjx");
        if p.exists() {
            return p;
        }
    }
    #[cfg(target_os = "macos")]
    {
        let user = default_macos_index_path();
        if user.exists() {
            return user;
        }
        let system = PathBuf::from("/Library/Application Support/BDJ Studio/Search Pro/index.bdjx");
        if system.exists() {
            return system;
        }
    }
    // Ninguna existe todavía: se devuelve la que el servicio usará, para poder
    // nombrarla en el mensaje de error.
    if let Ok(program_data) = std::env::var("ProgramData") {
        return PathBuf::from(program_data)
            .join("BDJ Studio")
            .join("Search Pro")
            .join("index.bdjx");
    }
    #[cfg(target_os = "macos")]
    return default_macos_index_path();
    PathBuf::from("index.bdjx")
}

pub fn engine_open(index_path_str: String) -> Result<(), String> {
    let path = if index_path_str.trim().is_empty() {
        resolve_default_index_path()
    } else {
        let p = PathBuf::from(index_path_str);
        if p.is_dir() { p.join("index.bdjx") } else { p }
    };

    *INDEX_PATH.lock() = Some(path.clone());

    let fallo = |problema: IndexProblem, detalle: String| -> String {
        *LAST_PROBLEM.lock() = Some((problema.code(), problema.message(&path)));
        detalle
    };

    if !path.exists() {
        return Err(fallo(
            IndexProblem::NotFound,
            format!("El archivo de índice no existe en la ruta: {}", path.display()),
        ));
    }

    let mmap = match MmapIndex::open(&path) {
        Ok(m) => m,
        Err(e) => {
            let detalle = format!("Error al abrir el índice: {e}");
            return Err(fallo(classify(&path, &detalle), detalle));
        }
    };

    let generation = match mmap.view() {
        Ok(v) => v.header.generation,
        Err(e) => {
            let detalle = format!("Formato de índice inválido: {e:?}");
            return Err(fallo(classify(&path, &detalle), detalle));
        }
    };

    *MMAP_INDEX.lock() = Some(mmap);
    INDEX_GENERATION.store(generation, Ordering::SeqCst);
    *LAST_PROBLEM.lock() = None;

    // Y la capa que haya publicada, para no arrancar ciego a todo lo ocurrido
    // desde la última compactación.
    reload_overlay(&path, generation, true);
    Ok(())
}

/// Estado del motor, con la causa concreta cuando algo falla.
///
/// Antes la interfaz solo sabía si el índice estaba abierto o no, y pintaba
/// «Cargando motor…» para cualquier fallo. Con esto puede decir si falta el
/// servicio, si el índice es de un formato anterior o si es un problema de
/// permisos.
pub fn engine_status() -> EngineStatusFfi {
    let path = INDEX_PATH
        .lock()
        .clone()
        .unwrap_or_else(resolve_default_index_path);
    let file_exists = path.exists();
    let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    let guard = MMAP_INDEX.lock();
    if let Some(mmap) = guard.as_ref()
        && let Ok(view) = mmap.view()
    {
        return EngineStatusFfi {
            index_path: path.to_string_lossy().to_string(),
            file_exists,
            file_size,
            is_open: true,
            generation: view.header.generation,
            entry_count: view.header.entry_count,
            problem: IndexProblem::None.code(),
            message: IndexProblem::None.message(&path),
        };
    }
    drop(guard);

    let (problem, message) = LAST_PROBLEM.lock().clone().unwrap_or_else(|| {
        let p = if file_exists {
            IndexProblem::Unknown
        } else {
            IndexProblem::NotFound
        };
        (p.code(), p.message(&path))
    });

    EngineStatusFfi {
        index_path: path.to_string_lossy().to_string(),
        file_exists,
        file_size,
        is_open: false,
        generation: 0,
        entry_count: 0,
        problem,
        message,
    }
}

/// Vuelve a mapear el índice si el servicio ha publicado una versión nueva.
///
/// Devuelve `true` cuando algo cambió, para que la interfaz repita la consulta
/// en curso. Sin esto el cliente mapeaba el índice una sola vez al arrancar y no
/// volvía a enterarse de nada.
pub fn reload_if_changed() -> bool {
    let path = INDEX_PATH
        .lock()
        .clone()
        .unwrap_or_else(resolve_default_index_path);

    // Leer la cabecera cuesta microsegundos; se puede sondear sin coste.
    let Ok(mmap) = MmapIndex::open(&path) else {
        return false;
    };
    let Ok(view) = mmap.view() else {
        return false;
    };
    let generation = view.header.generation;
    let base_cambio = generation != INDEX_GENERATION.load(Ordering::SeqCst);

    if base_cambio {
        INDEX_GENERATION.store(generation, Ordering::SeqCst);
        *MMAP_INDEX.lock() = Some(mmap);
        *INDEX_PATH.lock() = Some(path.clone());
        *LAST_PROBLEM.lock() = None;

        // La pila de refinamiento y el último resultado guardan identificadores
        // del índice anterior. Tras un renumerado apuntarían a archivos
        // equivocados.
        ENGINE.cancel_all();
        *LAST.lock() = None;
    }

    // La capa se recarga siempre, y a la fuerza si el base cambió: sus
    // identificadores solo significan algo respecto a una generación concreta.
    let capa_cambio = reload_overlay(&path, generation, base_cambio);

    if capa_cambio && !base_cambio {
        // El base sigue siendo el mismo, así que la pila de refinamiento —que
        // solo guarda resultados del base— sigue siendo válida y no se tira. Lo
        // que sí caduca es el último resultado, porque se calculó fundiendo una
        // capa que ya no es la de ahora.
        *LAST.lock() = None;
    }

    base_cambio || capa_cambio
}

/// Espera a que el índice publique una generación nueva.
///
/// Es la mitad "de empuje" de un socket: la interfaz no sondea con un
/// temporizador, sino que se queda esperando esta llamada; en cuanto el servicio
/// reescribe el índice (rename atómico), devuelve `true` en decenas de
/// milisegundos. Corre en el hilo de trabajo nativo, nunca en el de la UI.
///
/// Devuelve `false` si el plazo se agota sin publicaciones, para que la interfaz
/// pueda reintentar sin esperar para siempre.
pub async fn wait_index_changed(timeout_ms: u64) -> bool {
    let limite = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if reload_if_changed() {
            return true;
        }
        if Instant::now() >= limite {
            return false;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

pub fn index_generation() -> u64 {
    INDEX_GENERATION.load(Ordering::SeqCst)
}

pub fn engine_close() {
    *MMAP_INDEX.lock() = None;
    *LAST.lock() = None;
    *INDEX_PATH.lock() = None;
    *LAST_PROBLEM.lock() = None;
    INDEX_GENERATION.store(0, Ordering::SeqCst);
}

// ───────────────────────────────── Búsqueda ─────────────────────────────────

pub fn search(query: String, sort_col: u8, ascending: bool) -> u64 {
    search_with_limit(query, String::new(), sort_col, ascending, DEFAULT_LIMIT as u32)
}

/// Busca devolviendo, como mucho, `limit` filas ya ordenadas.
///
/// El conteo total que devuelve `search_status` es exacto aunque la ventana esté
/// recortada: contar es parte del recorrido, ordenar el conjunto entero no.
pub fn search_with_limit(
    query: String,
    scope: String,
    sort_col: u8,
    ascending: bool,
    limit: u32,
) -> u64 {
    let guard = MMAP_INDEX.lock();
    let capa = OVERLAY.lock();
    let capa_ids = OVERLAY_IDS.lock();
    if let Some(mmap) = guard.as_ref()
        && let Ok(view) = mmap.view()
    {
        // Se consultan las dos capas a la vez: el índice base y lo que el
        // servicio haya publicado desde la última compactación. Sin esto un
        // archivo copiado hace un segundo no existe para la búsqueda hasta que
        // se reescriba el índice entero.
        let (gen_id, result) = search_merged(
            &ENGINE,
            &view,
            capa.as_ref(),
            capa_ids.as_ref(),
            &query,
            &scope,
            sort_col,
            ascending,
            limit as usize,
        );
        *LAST.lock() = Some(LastQuery {
            result,
            query,
            scope,
            sort_col,
            ascending,
            is_browse: false,
        });
        return gen_id;
    }
    ENGINE.current_generation()
}

/// Amplía la ventana de la última consulta sin cambiarla.
///
/// La interfaz la usa cuando el usuario baja más allá de las filas que ya tiene.
/// Devuelve la nueva generación, o la misma si no hacía falta ampliar.
pub fn extend_results(generation: u64, limit: u32) -> u64 {
    let (query, scope, sort_col, ascending, is_browse, ya_tiene) = {
        let guard = LAST.lock();
        match guard.as_ref() {
            Some(l) if l.result.generation == generation => (
                l.query.clone(),
                l.scope.clone(),
                l.sort_col,
                l.ascending,
                l.is_browse,
                l.result.entry_ids.len(),
            ),
            _ => return generation,
        }
    };

    if ya_tiene >= limit as usize {
        return generation;
    }
    if is_browse {
        return browse_path(query, sort_col, ascending, limit);
    }
    search_with_limit(query, scope, sort_col, ascending, limit)
}

pub fn search_status(generation: u64) -> SearchStatusFfi {
    let guard = LAST.lock();
    if let Some(l) = guard.as_ref()
        && l.result.generation == generation
    {
        return SearchStatusFfi {
            ready_count: l.result.entry_ids.len() as u32,
            total_count: l.result.total_count,
            // `is_complete` deja de significar «el motor terminó» —siempre
            // termina— y pasa a significar «tienes todas las filas»: es lo que
            // la interfaz necesita para saber si puede pedir más.
            is_complete: !l.result.truncated,
            generation,
            elapsed_ms: l.result.elapsed_ms,
        };
    }

    SearchStatusFfi {
        ready_count: 0,
        total_count: 0,
        is_complete: true,
        generation,
        elapsed_ms: 0,
    }
}

pub fn rows(generation: u64, offset: u32, count: u32) -> RowBatchFfi {
    let last = LAST.lock();
    let index_guard = MMAP_INDEX.lock();

    if let (Some(l), Some(mmap)) = (last.as_ref(), index_guard.as_ref())
        && l.result.generation == generation
        && let Ok(view) = mmap.view()
    {
        let ids = &l.result.entry_ids;
        let start = (offset as usize).min(ids.len());
        let end = (start + count as usize).min(ids.len());
        let slice = &ids[start..end];
        let actual = slice.len();

        let mut names = Vec::with_capacity(actual);
        let mut paths = Vec::with_capacity(actual);
        let mut extensions = Vec::with_capacity(actual);
        let mut sizes = Vec::with_capacity(actual);
        let mut mtimes = Vec::with_capacity(actual);
        let mut flags = Vec::with_capacity(actual);

        // Los identificadores pueden venir de las dos capas, así que se leen
        // por el resolvedor unificado y no indexando la vista del base: un
        // identificador por encima de `base_count` caería fuera de rango.
        let capa = OVERLAY.lock();
        let capa_ids = OVERLAY_IDS.lock();
        let capa_view = capa.as_ref().and_then(|s| s.view().ok());
        let src = match (capa.as_ref(), capa_view.as_ref()) {
            (Some(s), Some(cv)) => RowSource::with_overlay(&view, s, cv, capa_ids.as_ref()),
            _ => RowSource::plain(&view),
        };

        for &id in slice {
            names.push(src.name(id));
            // Columna «Ruta»: la carpeta contenedora, no la ruta del propio
            // archivo, que ya se ve en la columna «Nombre».
            paths.push(src.parent_path(id));
            extensions.push(src.extension(id));
            sizes.push(src.size(id));
            mtimes.push(src.mtime(id));
            flags.push(src.flags(id));
        }

        return RowBatchFfi {
            generation,
            offset: start as u32,
            count: actual as u32,
            names,
            paths,
            extensions,
            sizes,
            mtimes,
            flags,
        };
    }

    RowBatchFfi {
        generation,
        offset,
        count: 0,
        names: Vec::new(),
        paths: Vec::new(),
        extensions: Vec::new(),
        sizes: Vec::new(),
        mtimes: Vec::new(),
        flags: Vec::new(),
    }
}

pub fn full_path(generation: u64, row: u32) -> String {
    let last = LAST.lock();
    let index_guard = MMAP_INDEX.lock();

    if let (Some(l), Some(mmap)) = (last.as_ref(), index_guard.as_ref())
        && l.result.generation == generation
        && let Some(&id) = l.result.entry_ids.get(row as usize)
        && let Ok(view) = mmap.view()
    {
        let capa = OVERLAY.lock();
        let capa_ids = OVERLAY_IDS.lock();
        let capa_view = capa.as_ref().and_then(|s| s.view().ok());
        let src = match (capa.as_ref(), capa_view.as_ref()) {
            (Some(s), Some(cv)) => RowSource::with_overlay(&view, s, cv, capa_ids.as_ref()),
            _ => RowSource::plain(&view),
        };
        return src.full_path(id);
    }
    String::new()
}

/// Rutas completas de varias filas a la vez.
///
/// Es lo que necesita el arrastre: el sistema operativo espera **la lista** de
/// referencias de lo seleccionado. Pedirlas una a una obligaría a bloquear y
/// soltar el índice tantas veces como archivos haya seleccionados.
pub fn paths_for_rows(generation: u64, rows: Vec<u32>) -> Vec<String> {
    let last = LAST.lock();
    let index_guard = MMAP_INDEX.lock();

    let mut out = Vec::with_capacity(rows.len());
    if let (Some(l), Some(mmap)) = (last.as_ref(), index_guard.as_ref())
        && l.result.generation == generation
        && let Ok(view) = mmap.view()
    {
        let capa = OVERLAY.lock();
        let capa_ids = OVERLAY_IDS.lock();
        let capa_view = capa.as_ref().and_then(|s| s.view().ok());
        let src = match (capa.as_ref(), capa_view.as_ref()) {
            (Some(s), Some(cv)) => RowSource::with_overlay(&view, s, cv, capa_ids.as_ref()),
            _ => RowSource::plain(&view),
        };
        for r in rows {
            if let Some(&id) = l.result.entry_ids.get(r as usize) {
                out.push(src.full_path(id));
            }
        }
    }
    out
}

// ──────────────────────────────── Navegación ────────────────────────────────

/// Lista el contenido de una carpeta **desde el índice**, sin tocar el disco.
///
/// El resultado se comporta igual que el de una búsqueda: se lee con `rows`, se
/// ordena con las mismas columnas y se arrastra con `paths_for_rows`. Así el
/// explorador y el buscador comparten toda la maquinaria en lugar de duplicarla.
pub fn browse_path(path: String, sort_col: u8, ascending: bool, limit: u32) -> u64 {
    let guard = MMAP_INDEX.lock();
    let Some(mmap) = guard.as_ref() else {
        return ENGINE.current_generation();
    };
    let Ok(view) = mmap.view() else {
        return ENGINE.current_generation();
    };

    // Localizar la carpeta por su ruta, bajando por la columna de hijos.
    //
    // Se usa la capa **de verdad**, la que el servicio ha publicado, y no una
    // vacía de usar y tirar. Con la capa vacía una carpeta creada hace un
    // segundo no se podía ni localizar ni listar: el usuario pulsaba «nueva
    // carpeta» y no aparecía nada hasta la siguiente compactación.
    let capa = OVERLAY.lock();
    let capa_ids = OVERLAY_IDS.lock();
    let vacia;
    let overlay: &OverlayIndex = match capa_ids.as_ref() {
        Some(oi) => oi,
        None => {
            vacia = OverlayIndex::with_base_count(view.entry_count() as u32);
            &vacia
        }
    };
    let Some(folder) = overlay.resolve_id_by_path(Some(&view), &path) else {
        return ENGINE.current_generation();
    };

    let base_count = capa
        .as_ref()
        .map(|s| s.base_count)
        .unwrap_or(view.entry_count() as u32);
    let hijos: Vec<u32> = overlay
        .children_of(Some(&view), folder)
        .into_iter()
        .filter(|&c| c >= base_count || view.is_alive(c as usize))
        .collect();

    let gen_id = ENGINE.next_generation();
    let result = bdj_search_core::search::rank_merged(
        &view,
        capa.as_ref(),
        capa_ids.as_ref(),
        hijos,
        sort_col,
        ascending,
        limit as usize,
        gen_id,
    );
    *LAST.lock() = Some(LastQuery {
        result,
        query: path,
        scope: String::new(),
        sort_col,
        ascending,
        is_browse: true,
    });
    gen_id
}

/// Lista los subdirectorios inmediatos de `path`, desde el índice.
///
/// Es lo que alimenta el árbol jerárquico del panel lateral: dado que se lee
/// de la memoria mapeada no toca el disco, y devolver solo los hijos directos
/// permite expandir/contraer ramas sin enumerar el árbol entero.
pub fn browse_subdirs(path: String) -> Vec<String> {
    let guard = MMAP_INDEX.lock();
    let Some(mmap) = guard.as_ref() else {
        return Vec::new();
    };
    let Ok(view) = mmap.view() else {
        return Vec::new();
    };

    let overlay = bdj_search_core::index::OverlayIndex::with_base_count(view.entry_count() as u32);
    let Some(folder) = overlay.resolve_id_by_path(Some(&view), &path) else {
        return Vec::new();
    };

    let mut salida: Vec<String> = view
        .children(folder as usize)
        .iter()
        .copied()
        .filter(|&c| view.is_alive(c as usize) && view.is_dir(c as usize))
        .map(|c| view.resolve_full_path(c as usize))
        .collect();
    salida.sort_by_key(|a| a.to_lowercase());
    salida
}

/// Lista las raíces de los volúmenes indexados: el punto de partida del árbol.
pub fn browse_roots(sort_col: u8, ascending: bool) -> u64 {
    let guard = MMAP_INDEX.lock();
    let Some(mmap) = guard.as_ref() else {
        return ENGINE.current_generation();
    };
    let Ok(view) = mmap.view() else {
        return ENGINE.current_generation();
    };

    let raices = view.volume_roots();
    let (gen_id, result) = ENGINE.rank(&view, raices, sort_col, ascending, 256);
    *LAST.lock() = Some(LastQuery {
        result,
        query: String::new(),
        scope: String::new(),
        sort_col,
        ascending,
        is_browse: true,
    });
    gen_id
}

/// Miga de pan de una ruta: cada tramo con su nombre y su ruta completa.
pub fn breadcrumb(path: String) -> Vec<CrumbFfi> {
    let mut out = Vec::new();
    if path.trim().is_empty() {
        return out;
    }

    let unificada = path.replace('/', "\\");
    let mut acumulada = String::new();
    let mut primero = true;

    for parte in unificada.split('\\') {
        if parte.is_empty() {
            if primero {
                // Ruta absoluta de estilo Unix.
                acumulada.push('/');
            }
            continue;
        }
        if primero && parte.ends_with(':') {
            // Letra de unidad de Windows: el tramo es «C:\».
            acumulada.push_str(parte);
            acumulada.push('\\');
            out.push(CrumbFfi {
                name: parte.to_string(),
                path: acumulada.clone(),
            });
            primero = false;
            continue;
        }
        if !acumulada.is_empty() && !acumulada.ends_with('\\') && !acumulada.ends_with('/') {
            acumulada.push('\\');
        }
        acumulada.push_str(parte);
        out.push(CrumbFfi {
            name: parte.to_string(),
            path: acumulada.clone(),
        });
        primero = false;
    }
    out
}

/// Carpeta contenedora de una ruta. Cadena vacía si ya es una raíz.
pub fn parent_path(path: String) -> String {
    let p = Path::new(&path);
    match p.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_string_lossy().to_string(),
        _ => String::new(),
    }
}

// ───────────────────────────── Acciones rápidas ─────────────────────────────

pub fn reveal_in_explorer(generation: u64, row: u32) -> Result<(), String> {
    let path_str = full_path(generation, row);
    reveal_path(path_str)
}

pub fn reveal_path(path_str: String) -> Result<(), String> {
    if path_str.is_empty() {
        return Err("Ruta no encontrada".to_string());
    }
    let p = Path::new(&path_str);
    if !p.exists() {
        return Err(format!("El archivo ya no existe: {path_str}"));
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{path_str}\""))
            .spawn()
            .map_err(|e| format!("Error al abrir el Explorador: {e}"))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(&path_str)
            .spawn()
            .map_err(|e| format!("Error al abrir el Finder: {e}"))?;
    }

    Ok(())
}

pub fn open_file(generation: u64, row: u32) -> Result<(), String> {
    open_path(full_path(generation, row))
}

pub fn open_path(path_str: String) -> Result<(), String> {
    if path_str.is_empty() {
        return Err("Ruta no encontrada".to_string());
    }
    let p = Path::new(&path_str);
    if !p.exists() {
        return Err(format!("El archivo ya no existe: {path_str}"));
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("cmd.exe")
            .raw_arg(format!("/c start \"\" \"{path_str}\""))
            .spawn()
            .map_err(|e| format!("Error al abrir el archivo: {e}"))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path_str)
            .spawn()
            .map_err(|e| format!("Error al abrir el archivo: {e}"))?;
    }

    Ok(())
}

/// «Abrir con…»: deja que el sistema pregunte con qué programa.
pub fn open_with(path_str: String) -> Result<(), String> {
    if path_str.is_empty() {
        return Err("Ruta no encontrada".to_string());
    }
    let p = Path::new(&path_str);
    if !p.exists() {
        return Err(format!("El archivo ya no existe: {path_str}"));
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("rundll32.exe")
            .raw_arg(format!("shell32.dll,OpenAs_RunDLL \"{path_str}\""))
            .spawn()
            .map_err(|e| format!("Error al abrir el diálogo: {e}"))?;
    }

    #[cfg(target_os = "macos")]
    {
        // macOS no tiene un «abrir con» genérico desde consola; mostrarlo en el
        // Finder es lo más cercano y deja al usuario elegir desde ahí.
        return reveal_path(path_str);
    }

    Ok(())
}

/// Menú «Compartir…» delegando en la API nativa de cada sistema:
/// - Windows: invoca el verbo de compartir de shell de Windows o Explorer.
/// - macOS: invoca el menú/servicio de compartir del sistema.
pub fn share_file(path_str: String) -> Result<(), String> {
    if path_str.is_empty() {
        return Err("Ruta no encontrada".to_string());
    }
    let p = Path::new(&path_str);
    if !p.exists() {
        return Err(format!("El archivo ya no existe: {path_str}"));
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let escaped = path_str.replace('\'', "''");
        let ps_cmd = format!(
            "$sh = New-Object -ComObject Shell.Application; \
             $folder = $sh.NameSpace((Split-Path -Parent '{escaped}')); \
             $item = $folder.ParseName((Split-Path -Leaf '{escaped}')); \
             if ($item) {{ \
                 $verb = $item.Verbs() | Where-Object {{ $_.Name -like '*compartir*' -or $_.Name -like '*share*' }} | Select-Object -First 1; \
                 if ($verb) {{ $verb.DoIt() }} else {{ Start-Process explorer.exe \"/select,`\"{escaped}`\"\" }} \
             }}"
        );
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps_cmd])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }

    #[cfg(target_os = "macos")]
    {
        let escaped = path_str.replace('"', "\\\"");
        let script = format!(
            "tell application \"Finder\" to set theSelection to (POSIX file \"{escaped}\" as alias)\n\
             tell application \"Finder\" to activate"
        );
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .spawn();
    }

    Ok(())
}

pub fn show_properties(generation: u64, row: u32) -> Result<(), String> {
    show_properties_path(full_path(generation, row))
}

pub fn show_properties_path(path_str: String) -> Result<(), String> {
    if path_str.is_empty() {
        return Err("Ruta no encontrada".to_string());
    }
    let p = Path::new(&path_str);
    if !p.exists() {
        return Err(format!("El archivo ya no existe: {path_str}"));
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // La ruta va embebida en comillas simples de PowerShell; una comilla
        // dentro del nombre se escapa duplicandola. NameSpace/ParseName
        // devuelven $null si la ruta falla (p.ej. archivo borrado en disco), y
        // sin estos guardas `$item.InvokeVerb` lanza un error de PowerShell en
        // la consola.
        let ps_path = path_str.replace('\'', "''");
        let ps_script = format!(
            "$shell = New-Object -ComObject Shell.Application; \
             $folder = $shell.NameSpace([System.IO.Path]::GetDirectoryName('{ps_path}')); \
             if ($null -eq $folder) {{ exit 2 }}; \
             $item = $folder.ParseName([System.IO.Path]::GetFileName('{ps_path}')); \
             if ($null -eq $item) {{ exit 3 }}; \
             $item.InvokeVerb('properties')"
        );
        std::process::Command::new("powershell.exe")
            .raw_arg(format!(
                "-NoProfile -WindowStyle Hidden -Command \"{}\"",
                ps_script.replace('"', "\\\"")
            ))
            .spawn()
            .map_err(|e| format!("Error al mostrar las propiedades: {e}"))?;
    }

    #[cfg(target_os = "macos")]
    {
        let as_path = path_str.replace('"', "\\\"");
        let as_script = format!(
            "tell application \"Finder\" to open information window of (POSIX file \"{as_path}\" as alias)"
        );
        std::process::Command::new("osascript")
            .args(["-e", &as_script])
            .spawn()
            .map_err(|e| format!("Error al mostrar las propiedades: {e}"))?;
    }

    Ok(())
}

// ──────────────────────── Operaciones sobre archivos ────────────────────────

fn policy_from(code: u8) -> ConflictPolicy {
    match code {
        1 => ConflictPolicy::KeepBoth,
        2 => ConflictPolicy::Skip,
        3 => ConflictPolicy::Overwrite,
        _ => ConflictPolicy::Ask,
    }
}

fn kind_code(kind: OpKind) -> u8 {
    match kind {
        OpKind::CreateFolder => 0,
        OpKind::Rename => 1,
        OpKind::Copy => 2,
        OpKind::Move => 3,
        OpKind::Duplicate => 4,
        OpKind::Trash => 5,
        OpKind::Restore => 6,
        OpKind::DeletePermanently => 7,
        OpKind::CreateFile => 8,
        OpKind::CompressZip => 9,
        OpKind::ExtractZip => 10,
    }
}

fn state_code(state: bdj_search_fsops::OpState) -> u8 {
    use bdj_search_fsops::OpState::*;
    match state {
        Planning => 0,
        Running => 1,
        WaitingConflict => 2,
        Cancelling => 3,
        Done => 4,
        Cancelled => 5,
        Failed => 6,
    }
}

fn to_paths(v: Vec<String>) -> Vec<PathBuf> {
    v.into_iter().map(PathBuf::from).collect()
}

pub fn fsop_create_folder(parent: String, name: String) -> u64 {
    FSOPS.submit(
        OpRequest::new(OpKind::CreateFolder, Vec::new())
            .with_destination(PathBuf::from(parent))
            .with_new_name(name),
    )
}

/// Crea un archivo vacío dentro de `parent`. Equivale a «Nuevo documento».
pub fn fsop_create_file(parent: String, name: String) -> u64 {
    FSOPS.submit(
        OpRequest::new(OpKind::CreateFile, Vec::new())
            .with_destination(PathBuf::from(parent))
            .with_new_name(name),
    )
}

pub fn fsop_rename(path: String, new_name: String) -> u64 {
    FSOPS.submit(OpRequest::new(OpKind::Rename, vec![PathBuf::from(path)]).with_new_name(new_name))
}

pub fn fsop_copy(sources: Vec<String>, destination: String, policy: u8) -> u64 {
    FSOPS.submit(
        OpRequest::new(OpKind::Copy, to_paths(sources))
            .with_destination(PathBuf::from(destination))
            .with_policy(policy_from(policy)),
    )
}

pub fn fsop_move(sources: Vec<String>, destination: String, policy: u8) -> u64 {
    FSOPS.submit(
        OpRequest::new(OpKind::Move, to_paths(sources))
            .with_destination(PathBuf::from(destination))
            .with_policy(policy_from(policy)),
    )
}

pub fn fsop_duplicate(sources: Vec<String>) -> u64 {
    FSOPS.submit(OpRequest::new(OpKind::Duplicate, to_paths(sources)))
}

/// Envía a la papelera. **Nunca borra de forma definitiva.**
pub fn fsop_trash(sources: Vec<String>) -> u64 {
    FSOPS.submit(OpRequest::new(OpKind::Trash, to_paths(sources)))
}

/// Restaura de la papelera a su ubicación original. Windows y Linux únicamente.
pub fn fsop_restore(paths: Vec<String>) -> u64 {
    FSOPS.submit(OpRequest::new(OpKind::Restore, to_paths(paths)))
}

/// Borra del disco de forma definitiva, sin pasar por la papelera.
///
/// **No se puede deshacer.** La interfaz solo debe llamarla tras una
/// confirmación explícita.
pub fn fsop_delete_permanently(sources: Vec<String>) -> u64 {
    FSOPS.submit(OpRequest::new(OpKind::DeletePermanently, to_paths(sources)))
}

pub fn fsop_compress_zip(sources: Vec<String>, destination: String) -> u64 {
    FSOPS.submit(
        OpRequest::new(OpKind::CompressZip, to_paths(sources))
            .with_destination(PathBuf::from(destination)),
    )
}

pub fn fsop_extract_zip(sources: Vec<String>, destination: String) -> u64 {
    FSOPS.submit(
        OpRequest::new(OpKind::ExtractZip, to_paths(sources))
            .with_destination(PathBuf::from(destination)),
    )
}

fn to_ffi(p: bdj_search_fsops::OpProgress) -> FileOpFfi {
    let c = p.pending_conflict.clone();
    FileOpFfi {
        id: p.id,
        kind: kind_code(p.kind),
        state: state_code(p.state),
        total_items: p.total_items,
        done_items: p.done_items,
        total_bytes: p.total_bytes,
        done_bytes: p.done_bytes,
        current: p.current,
        errors: p.errors,
        can_undo: p.can_undo,
        elapsed_ms: p.elapsed_ms,
        has_conflict: c.is_some(),
        conflict_source: c.as_ref().map(|x| x.source.clone()).unwrap_or_default(),
        conflict_destination: c
            .as_ref()
            .map(|x| x.destination.clone())
            .unwrap_or_default(),
        conflict_source_size: c.as_ref().map(|x| x.source_size).unwrap_or(0),
        conflict_destination_size: c.as_ref().map(|x| x.destination_size).unwrap_or(0),
        conflict_source_mtime: c.as_ref().map(|x| x.source_mtime).unwrap_or(0),
        conflict_destination_mtime: c.as_ref().map(|x| x.destination_mtime).unwrap_or(0),
    }
}

/// Estado de todas las operaciones vivas.
///
/// La interfaz lo consulta unas pocas veces por segundo: los contadores son
/// atómicos y leerlos no interrumpe a los hilos que están copiando.
pub fn fsop_progress_all() -> Vec<FileOpFfi> {
    FSOPS.all_progress().into_iter().map(to_ffi).collect()
}

pub fn fsop_cancel(op_id: u64) {
    FSOPS.cancel(op_id);
}

/// Responde a un conflicto. `decision`: 0 conservar ambos, 1 omitir,
/// 2 sobrescribir, 3 cancelar.
pub fn fsop_resolve_conflict(op_id: u64, decision: u8, apply_to_all: bool) {
    let d = match decision {
        0 => ConflictDecision::KeepBoth,
        1 => ConflictDecision::Skip,
        2 => ConflictDecision::Overwrite,
        _ => ConflictDecision::Cancel,
    };
    FSOPS.resolve_conflict(op_id, d, apply_to_all);
}

/// Archiva las operaciones terminadas y avisa al servicio de lo que cambió.
///
/// Devuelve cuántas se archivaron. Conviene llamarlo desde el mismo temporizador
/// que consulta el progreso: es lo que abre la ventana de supresión en el
/// servicio, y por tanto lo que evita que el cambio se procese dos veces.
pub fn fsop_reap() -> u32 {
    let archivadas = FSOPS.reap() as u32;
    flush_changes();
    archivadas
}

/// ¿Hay algo que deshacer?
pub fn fsop_can_undo() -> bool {
    FSOPS.last_undoable().is_some()
}

/// Deshace la última operación reversible. Devuelve los identificadores de las
/// operaciones lanzadas para revertirla.
pub fn fsop_undo_last() -> Vec<u64> {
    FSOPS.undo_last()
}

/// ¿Hay algo que rehacer? (Algo que se deshizo y no se ha invalidado después.)
pub fn fsop_can_redo() -> bool {
    FSOPS.can_redo()
}

/// Rehace la última operación que se deshizo. Devuelve los identificadores de
/// las operaciones lanzadas para recrearla.
pub fn fsop_redo_last() -> Vec<u64> {
    FSOPS.redo_last()
}

/// Comunica al servicio lo que la aplicación acaba de hacer en el disco.
fn flush_changes() {
    let cambios = FSOPS.take_change_notifications();
    if cambios.is_empty() {
        return;
    }
    let traducidos: Vec<LocalChange> = cambios
        .into_iter()
        .map(|c| match c {
            PathChange::Created { path, is_dir } => LocalChange::Created { path, is_dir },
            PathChange::Removed { path } => LocalChange::Removed { path },
            PathChange::Renamed { from, to } => LocalChange::Renamed { from, to },
        })
        .collect();
    service::send_ok(&IpcCommand::LocalChanges {
        changes: traducidos,
    });
    // Recarga la capa de inmediato: el servicio ya la publicó en disco antes de
    // responder con Ack, así que este proceso ya puede ver el archivo nuevo en el
    // mismo fotograma.
    reload_if_changed();
}

/// Ajustes que la interfaz debe usar en esta máquina.
///
/// La interfaz pedía dos mil filas de entrada y guardaba diez páginas en
/// memoria, viniera de un sobremesa o de un portátil de dos núcleos. Pedir dos
/// mil filas para pintar treinta es trabajo tirado justo en el equipo que menos
/// puede permitírselo.
pub fn machine_tuning() -> TuningFfi {
    let t = Tuning::current();
    TuningFfi {
        tier: match t.tier {
            bdj_search_core::tuning::Tier::Low => 0,
            bdj_search_core::tuning::Tier::Mid => 1,
            bdj_search_core::tuning::Tier::High => 2,
        },
        tier_name: t.tier.name().to_string(),
        cores: t.cores as u32,
        memory_mb: t.memory_mb,
        search_threads: t.search_threads as u32,
        initial_limit: t.initial_limit as u32,
        cached_pages: t.cached_pages as u32,
    }
}

// ─────────────────────────── Control del servicio ───────────────────────────

/// Comunica al servicio si hay licencia activa y hasta cuándo.
///
/// Sin esto el servicio no indexa: era el agujero por el que un servicio elevado
/// recorría el disco entero sin comprobar nada.
pub fn service_set_license(active: bool, expires_at: u64) -> bool {
    service::send_ok(&IpcCommand::SetLicenseState { active, expires_at })
}

pub fn service_set_volume_indexed(mount_prefix: String, indexed: bool) -> bool {
    service::send_ok(&IpcCommand::SetVolumeIndexed {
        mount_prefix,
        indexed,
    })
}

pub fn service_add_folder(path: String) -> bool {
    service::send_ok(&IpcCommand::AddFolder { path })
}

pub fn service_rescan(mount_prefix: String) -> bool {
    service::send_ok(&IpcCommand::RescanVolume { mount_prefix })
}

pub fn service_set_indexing(enabled: bool) -> bool {
    service::send_ok(&if enabled {
        IpcCommand::EnableIndexing
    } else {
        IpcCommand::DisableIndexing
    })
}

pub fn service_status() -> ServiceStatusFfi {
    let vacio = || ServiceStatusFfi {
        reachable: false,
        indexing_enabled: false,
        license_active: false,
        generation: 0,
        entry_count: 0,
        volume_prefixes: Vec::new(),
        volume_labels: Vec::new(),
        volume_fs_types: Vec::new(),
        volume_connected: Vec::new(),
        volume_indexed: Vec::new(),
        volume_entry_counts: Vec::new(),
    };

    match service::send(&IpcCommand::GetStatus) {
        Some(IpcEvent::Status {
            indexing_enabled,
            license_active,
            generation,
            entry_count,
            volumes,
        }) => ServiceStatusFfi {
            reachable: true,
            indexing_enabled,
            license_active,
            generation,
            entry_count,
            volume_prefixes: volumes.iter().map(|v| v.mount_prefix.clone()).collect(),
            volume_labels: volumes.iter().map(|v| v.label.clone()).collect(),
            volume_fs_types: volumes.iter().map(|v| v.fs_type.clone()).collect(),
            volume_connected: volumes.iter().map(|v| v.is_connected).collect(),
            volume_indexed: volumes.iter().map(|v| v.is_indexed).collect(),
            volume_entry_counts: volumes.iter().map(|v| v.entry_count).collect(),
        },
        _ => vacio(),
    }
}

// ──────── Adaptadores legacy / FRB exportados directamente ────────
// Este bloque de operaciones de archivo quedó superado por `bdj_search_fsops`
// (las funciones `fsop_*`): copiaban de forma bloqueante y sin deshacer. Al no
// haber un módulo `file_ops` en `bdj_search_core`, además, ni siquiera compilaba.
// Se elimina junto con los archivos huérfanos del núcleo (tarea 10).
