use bdj_search_core::index::MmapIndex;
use bdj_search_core::search::{Engine, SearchResult};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

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

static ENGINE: LazyLock<Engine> = LazyLock::new(Engine::new);
static MMAP_INDEX: Mutex<Option<MmapIndex>> = Mutex::new(None);
static LAST_SEARCH: Mutex<Option<SearchResult>> = Mutex::new(None);
/// Ruta y generacion del indice mapeado, para detectar republicaciones.
static INDEX_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
static INDEX_GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn ping() -> String {
    "pong from BDJ Search Pro Rust Engine".to_string()
}

fn resolve_default_index_path() -> PathBuf {
    if let Ok(program_data) = std::env::var("ProgramData") {
        let p = PathBuf::from(program_data).join("BDJ Studio").join("Search Pro").join("index.bdjx");
        if p.exists() {
            return p;
        }
    }
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(local_app_data).join("BDJ Studio").join("Search Pro").join("index.bdjx");
        if p.exists() {
            return p;
        }
    }
    if cfg!(target_os = "macos") {
        let p = PathBuf::from("/Library/Application Support/BDJ Studio/Search Pro/index.bdjx");
        if p.exists() {
            return p;
        }
    }
    PathBuf::from("index.bdjx")
}

pub fn engine_open(index_path_str: String) -> Result<(), String> {
    let path = if index_path_str.trim().is_empty() {
        resolve_default_index_path()
    } else {
        let p = PathBuf::from(index_path_str);
        if p.is_dir() {
            p.join("index.bdjx")
        } else {
            p
        }
    };

    if !path.exists() {
        return Err(format!("El archivo de índice no existe en la ruta: {}", path.display()));
    }

    let mmap = MmapIndex::open(&path).map_err(|e| format!("Error al abrir mmap: {}", e))?;
    // Validate header and sections
    let _ = mmap.view().map_err(|e| format!("Formato de índice inválido: {:?}", e))?;

    let generation = mmap.view().map(|v| v.header.generation).unwrap_or(0);

    let mut guard = MMAP_INDEX.lock();
    *guard = Some(mmap);
    drop(guard);

    *INDEX_PATH.lock() = Some(path);
    INDEX_GENERATION.store(generation, Ordering::SeqCst);

    Ok(())
}

/// Vuelve a mapear el indice si el servicio ha publicado una version nueva.
///
/// Devuelve `true` cuando algo cambio, para que la interfaz repita la consulta
/// en curso. Sin esto el cliente mapeaba el indice una sola vez al arrancar y
/// no volvia a enterarse de nada: por muy al dia que estuviera el servicio, la
/// aplicacion seguia mostrando la foto del arranque.
pub fn reload_if_changed() -> bool {
    let path = match INDEX_PATH.lock().clone() {
        Some(p) => p,
        None => return false,
    };

    // Leer la cabecera cuesta microsegundos; se puede sondear sin coste.
    let Ok(mmap) = MmapIndex::open(&path) else {
        return false;
    };
    let Ok(view) = mmap.view() else {
        return false;
    };
    let generation = view.header.generation;

    if generation == INDEX_GENERATION.load(Ordering::SeqCst) {
        return false;
    }

    INDEX_GENERATION.store(generation, Ordering::SeqCst);
    *MMAP_INDEX.lock() = Some(mmap);

    // La pila de refinamiento y el ultimo resultado guardan identificadores del
    // indice anterior. Tras un renumerado apuntarian a archivos equivocados.
    ENGINE.cancel_all();
    *LAST_SEARCH.lock() = None;

    true
}

/// Generacion del indice actualmente mapeado, para diagnostico.
pub fn index_generation() -> u64 {
    INDEX_GENERATION.load(Ordering::SeqCst)
}

pub fn engine_close() {
    *MMAP_INDEX.lock() = None;
    *LAST_SEARCH.lock() = None;
    *INDEX_PATH.lock() = None;
    INDEX_GENERATION.store(0, Ordering::SeqCst);
}

pub fn search(query: String, sort_col: u8, ascending: bool) -> u64 {
    let guard = MMAP_INDEX.lock();
    if let Some(mmap) = guard.as_ref()
        && let Ok(view) = mmap.view()
    {
        let (gen_id, result) = ENGINE.search(&view, &query, sort_col, ascending);
        let mut search_guard = LAST_SEARCH.lock();
        *search_guard = Some(result);
        return gen_id;
    }

    ENGINE.current_generation()
}

pub fn search_status(generation: u64) -> SearchStatusFfi {
    let search_guard = LAST_SEARCH.lock();
    if let Some(res) = search_guard.as_ref()
        && res.generation == generation
    {
        return SearchStatusFfi {
            ready_count: res.entry_ids.len() as u32,
            total_count: res.total_count,
            is_complete: true,
            generation,
            elapsed_ms: res.elapsed_ms,
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
    let search_guard = LAST_SEARCH.lock();
    let index_guard = MMAP_INDEX.lock();

    if let (Some(res), Some(mmap)) = (search_guard.as_ref(), index_guard.as_ref())
        && res.generation == generation
        && let Ok(view) = mmap.view()
    {
        let start = (offset as usize).min(res.entry_ids.len());
        let end = (start + count as usize).min(res.entry_ids.len());
        let slice = &res.entry_ids[start..end];
        let actual_count = slice.len();

        let mut names = Vec::with_capacity(actual_count);
        let mut paths = Vec::with_capacity(actual_count);
                let mut extensions = Vec::with_capacity(actual_count);
                let mut sizes = Vec::with_capacity(actual_count);
                let mut mtimes = Vec::with_capacity(actual_count);
                let mut flags = Vec::with_capacity(actual_count);

                for &id in slice {
                    let u_idx = id as usize;
                    names.push(view.get_name(u_idx).unwrap_or("").to_string());
                    // Columna "Ruta": la carpeta contenedora, no la ruta del
                    // propio archivo, que ya se ve en la columna "Nombre".
                    paths.push(view.resolve_parent_path(u_idx));
                    extensions.push(view.get_extension(u_idx).to_string());
                    sizes.push(view.size[u_idx]);
                    mtimes.push(view.mtime[u_idx]);
                    flags.push(view.flags[u_idx]);
                }

                return RowBatchFfi {
                    generation,
                    offset: start as u32,
                    count: actual_count as u32,
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
    let search_guard = LAST_SEARCH.lock();
    let index_guard = MMAP_INDEX.lock();

    if let (Some(res), Some(mmap)) = (search_guard.as_ref(), index_guard.as_ref())
        && res.generation == generation
        && let Some(&id) = res.entry_ids.get(row as usize)
        && let Ok(view) = mmap.view()
    {
        return view.resolve_full_path(id as usize);
    }

    String::new()
}

pub fn reveal_in_explorer(generation: u64, row: u32) -> Result<(), String> {
    let path_str = full_path(generation, row);
    if path_str.is_empty() {
        return Err("Ruta no encontrada".to_string());
    }

    let p = Path::new(&path_str);
    if !p.exists() {
        return Err(format!("El archivo no existe: {}", path_str));
    }

    #[cfg(windows)]
    {
        std::process::Command::new("explorer.exe")
            .arg(format!("/select,\"{}\"", path_str))
            .spawn()
            .map_err(|e| format!("Error al abrir Explorer: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(&path_str)
            .spawn()
            .map_err(|e| format!("Error al abrir Finder: {}", e))?;
    }

    Ok(())
}
