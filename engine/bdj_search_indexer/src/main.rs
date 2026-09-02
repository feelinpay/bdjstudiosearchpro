use bdj_search_core::tuning::Tuning;
use bdj_search_core::index::{
    IndexBuilder, IndexerSettings, MmapIndex, OverlayIndex, SuppressionWindow, overlay_path_for,
    write_overlay,
};
#[cfg(any(windows, unix))]
use bdj_search_ipc::{IpcCommand, IpcEvent, LocalChange, VolumeStatus};
#[cfg(windows)]
use bdj_search_ipc::{DEFAULT_PIPE_NAME, PipeServer};
#[cfg(unix)]
use bdj_search_ipc::{default_unix_socket_path, UnixSocketServer};
#[allow(unused_imports)]
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::time::Instant;

#[cfg(windows)]
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x02;
#[cfg(windows)]
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x04;

/// Bandera global para que el manejador de Stop del servicio pueda detener el
/// bucle principal sin necesidad de referencias cruzadas.
static GLOBAL_RUNNING: AtomicBool = AtomicBool::new(true);

#[cfg(windows)]
fn is_on_battery() -> bool {
    #[repr(C)]
    struct SystemPowerStatus {
        ac_line_status: u8,
        battery_flag: u8,
        battery_life_percent: u8,
        system_status_flag: u8,
        battery_life_time: u32,
        battery_full_life_time: u32,
    }
    unsafe extern "system" {
        fn GetSystemPowerStatus(status: *mut SystemPowerStatus) -> i32;
    }
    let mut status = std::mem::MaybeUninit::<SystemPowerStatus>::uninit();
    if unsafe { GetSystemPowerStatus(status.as_mut_ptr()) } != 0 {
        let s = unsafe { status.assume_init() };
        s.ac_line_status == 0
    } else {
        false
    }
}

#[cfg(not(windows))]
fn is_on_battery() -> bool {
    false
}

#[cfg(windows)]
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};

#[cfg(windows)]
const SERVICE_NAME: &str = "BDJSearchProIndexer";

/// Ubicacion del indice. Debe coincidir exactamente con lo que busca el cliente
/// en `bdj_search_ffi::resolve_default_index_path`, o el servicio indexa en un
/// sitio y la aplicacion mira en otro.
fn get_index_path() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(program_data) = std::env::var("ProgramData") {
            let p = PathBuf::from(program_data)
                .join("BDJ Studio")
                .join("Search Pro");
            let _ = std::fs::create_dir_all(&p);
            return p.join("index.bdjx");
        }
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let p = PathBuf::from(local_app_data)
                .join("BDJ Studio")
                .join("Search Pro");
            let _ = std::fs::create_dir_all(&p);
            return p.join("index.bdjx");
        }
    }

    #[cfg(target_os = "macos")]
    {
        // Un agente de usuario corre sin privilegios: la carpeta por usuario
        // es la que se puede crear sin elevación (igual que Sample Pad).
        // `/Library` queda como secundario, para instalaciones de sistema.
        if let Ok(home) = std::env::var("HOME") {
            let p = PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("BDJ Studio")
                .join("Search Pro");
            let _ = std::fs::create_dir_all(&p);
            return p.join("index.bdjx");
        }
        let p = PathBuf::from("/Library/Application Support/BDJ Studio/Search Pro");
        if std::fs::create_dir_all(&p).is_ok() {
            return p.join("index.bdjx");
        }
    }

    PathBuf::from("index.bdjx")
}

/// Volumen NTFS bajo vigilancia del diario USN.
#[cfg(windows)]
struct VolumeWatch {
    drive_letter: char,
    vol_id: u8,
    mount_prefix: String,
    root_id: u32,
    journal_id: u64,
    next_usn: i64,
}

/// Volumen sin diario USN —exFAT, FAT32, o NTFS sin elevacion— vigilado con
/// `ReadDirectoryChangesW`.
///
/// Es el caso del USB de cabina, que es exactamente el disco que mas cambia en
/// el flujo de un DJ. Antes se indexaba una vez al conectarlo y no se volvia a
/// mirar nunca: copiar una pista al pendrive no la hacia aparecer.
#[cfg(windows)]
struct WalkWatch {
    mount_prefix: String,
    vol_id: u8,
}

/// Un volumen macOS bajo vigilancia FSEvents.
///
/// Cada uno tiene su propio `FsEventWatcher` —que corre en su hilo con su
/// run loop— y guarda el ultimo id de evento visto para poder reenganchar el
/// stream sin perder cambios si algun dia hay que recrearlo.
#[cfg(target_os = "macos")]
struct MacVolumeWatch {
    mount_prefix: PathBuf,
    last_event_id: u64,
    watcher: Option<bdj_search_fs::macos::FsEventWatcher>,
}

/// Un cambio observado por `ReadDirectoryChangesW`, ya con su ruta completa.
#[cfg(windows)]
#[derive(Debug, Clone)]
struct WalkChange {
    vol_id: u8,
    full_path: String,
    action: u32,
}

/// Acciones de `FILE_NOTIFY_INFORMATION`.
#[cfg(windows)]
mod rdcw_action {
    pub const ADDED: u32 = 1;
    pub const REMOVED: u32 = 2;
    pub const MODIFIED: u32 = 3;
    pub const RENAMED_OLD_NAME: u32 = 4;
    pub const RENAMED_NEW_NAME: u32 = 5;
}

/// Lo que hace falta para traducir un cambio del diario en una operacion sobre
/// el indice: de que entrada habla y de que carpeta cuelga.
#[cfg(windows)]
#[derive(Default)]
struct WatchState {
    volumes: Vec<VolumeWatch>,
    /// Volumenes vigilados por recorrido de directorios.
    walk_volumes: Vec<WalkWatch>,
    /// Numero de referencia de la MFT -> identificador unificado.
    ///
    /// Es el equivalente al mapa que Everything mantiene en memoria. Sin el, un
    /// registro del diario no se puede asociar a ninguna entrada del indice.
    frn_to_id: HashMap<u64, u32>,
}

pub struct IndexerService {
    running: Arc<AtomicBool>,
    overlay: Arc<Mutex<OverlayIndex>>,
    index_path: PathBuf,
    #[cfg_attr(not(any(windows, unix)), allow(dead_code))]
    settings_path: PathBuf,
    settings: Arc<Mutex<IndexerSettings>>,
    /// Rutas que la aplicacion acaba de tocar y cuyos eventos del sistema de
    /// archivos hay que descartar. Es la reconciliacion del doble evento.
    suppress: Arc<Mutex<SuppressionWindow>>,
    #[cfg(windows)]
    watch: Arc<Mutex<WatchState>>,
    /// Cambios recogidos por los vigilantes de directorio, a la espera de
    /// aplicarse en el bucle principal.
    #[cfg(windows)]
    walk_queue: Arc<Mutex<Vec<WalkChange>>>,
    /// Volumenes que hay que volver a escanear entero, por prefijo de montaje.
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    pending_rescans: Arc<Mutex<Vec<String>>>,
    /// Vigilantes FSEvents de macOS, uno por volumen montado.
    #[cfg(target_os = "macos")]
    mac_watch: Arc<Mutex<Vec<MacVolumeWatch>>>,
    /// Número de publicación de la capa de cambios.
    ///
    /// Sube en cada escritura de `overlay.bdjo`. La aplicación lo compara con el
    /// que tiene mapeado para saber si hay algo nuevo sin releer el archivo
    /// entero.
    overlay_generation: Arc<AtomicU64>,
}

/// Segundos desde 1970.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl IndexerService {
    pub fn new(index_path: PathBuf) -> Self {
        let settings_path = index_path.with_file_name("settings.json");
        let settings = IndexerSettings::load(&settings_path);
        tracing::info!(
            "Ajustes: indexado {}, licencia {}, {} volumen(es) excluido(s)",
            if settings.indexing_enabled { "activo" } else { "detenido" },
            if settings.license_active { "activa" } else { "sin activar" },
            settings.excluded_volumes.len()
        );
        Self {
            running: Arc::new(AtomicBool::new(true)),
            overlay: Arc::new(Mutex::new(OverlayIndex::new())),
            index_path,
            settings_path,
            settings: Arc::new(Mutex::new(settings)),
            suppress: Arc::new(Mutex::new(SuppressionWindow::default())),
            #[cfg(windows)]
            watch: Arc::new(Mutex::new(WatchState::default())),
            #[cfg(windows)]
            walk_queue: Arc::new(Mutex::new(Vec::new())),
            pending_rescans: Arc::new(Mutex::new(Vec::new())),
            #[cfg(target_os = "macos")]
            mac_watch: Arc::new(Mutex::new(Vec::new())),
            overlay_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Generación del `index.bdjx` que hay ahora mismo publicado.
    ///
    /// La capa se ancla a esta generación: la aplicación descarta una capa cuyo
    /// número no coincida con el del índice que tiene abierto, porque compactar
    /// renumera todas las entradas y una capa vieja colocaría los archivos en
    /// carpetas equivocadas.
    fn base_generation(&self) -> u64 {
        MmapIndex::open(&self.index_path)
            .ok()
            .and_then(|m| m.view().map(|v| v.header.generation).ok())
            .unwrap_or(0)
    }

    /// Publica la capa de cambios junto al índice.
    ///
    /// Es la operación que hace que un archivo copiado hace un segundo aparezca
    /// en la lista sin esperar a la compactación. Cuesta milisegundos porque el
    /// archivo solo contiene lo que ha cambiado desde la última compactación,
    /// mientras que reescribir el índice de diez millones de entradas cuesta
    /// unos veinticinco segundos.
    fn publish_overlay(&self, base_generation: u64) {
        publish_overlay_file(
            &self.overlay,
            &self.overlay_generation,
            &self.index_path,
            base_generation,
        );
    }

    /// ¿Puede indexarse ahora mismo?
    fn may_index(&self) -> bool {
        self.settings
            .lock()
            .map(|s| s.may_index(now_secs()))
            .unwrap_or(false)
    }

    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    fn volume_is_indexed(&self, mount_prefix: &str) -> bool {
        self.settings
            .lock()
            .map(|s| s.is_volume_indexed(mount_prefix))
            .unwrap_or(true)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        GLOBAL_RUNNING.store(false, Ordering::SeqCst);
    }

    /// Apunta un volumen para volver a escanearlo entero.
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    fn request_rescan(&self, mount_prefix: &str) {
        if let Ok(mut v) = self.pending_rescans.lock()
            && !v.iter().any(|p| p.eq_ignore_ascii_case(mount_prefix))
        {
            v.push(mount_prefix.to_string());
        }
    }

    /// Lanza un hilo de vigilancia por cada volumen sin diario USN.
    ///
    /// `ReadDirectoryChangesW` es una llamada **bloqueante**: se queda dormida
    /// hasta que algo cambia. Por eso no puede vivir en el bucle de sondeo, y
    /// por eso cada volumen tiene su propio hilo, que deja lo que ve en una cola
    /// que el bucle principal vacia.
    ///
    /// El modulo `rdcw.rs` estaba escrito y no se instanciaba en ningun sitio:
    /// los volumenes exFAT y FAT32 se indexaban al conectarlos y no se volvian a
    /// mirar nunca.
    #[cfg(windows)]
    fn spawn_walk_watchers(&self) {
        use bdj_search_fs::windows::DirectoryWatcher;

        let objetivos: Vec<(String, u8)> = match self.watch.lock() {
            Ok(w) => w
                .walk_volumes
                .iter()
                .map(|v| (v.mount_prefix.clone(), v.vol_id))
                .collect(),
            Err(_) => return,
        };

        for (mount_prefix, vol_id) in objetivos {
            let cola = self.walk_queue.clone();
            let running = self.running.clone();
            let raiz = PathBuf::from(&mount_prefix);
            thread::Builder::new()
                .name(format!("bdj-rdcw-{vol_id}"))
                .spawn(move || {
                    let watcher = match DirectoryWatcher::open(&raiz) {
                        Ok(w) => w,
                        Err(e) => {
                            tracing::warn!("{}: no se pudo vigilar ({e})", raiz.display());
                            return;
                        }
                    };
                    tracing::info!("{}: vigilancia por directorios activa", raiz.display());
                    let mut buffer = vec![0u8; 64 * 1024];

                    while running.load(Ordering::SeqCst) && GLOBAL_RUNNING.load(Ordering::SeqCst) {
                        let mut lote: Vec<WalkChange> = Vec::new();
                        let leidos = watcher.read_changes(&mut buffer, |rec| {
                            let mut full = raiz.clone();
                            full.push(&rec.relative_path);
                            lote.push(WalkChange {
                                vol_id,
                                full_path: full.to_string_lossy().to_string(),
                                action: rec.action,
                            });
                        });

                        match leidos {
                            Ok(_) => {
                                if !lote.is_empty()
                                    && let Ok(mut q) = cola.lock()
                                {
                                    // Un tope evita que una copia masiva sobre el
                                    // pendrive llene la memoria mientras el bucle
                                    // principal esta ocupado.
                                    if q.len() < 200_000 {
                                        q.extend(lote);
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "{}: la vigilancia se interrumpio ({e})",
                                    raiz.display()
                                );
                                break;
                            }
                        }
                    }
                })
                .ok();
        }
    }

    /// Aplica a la capa lo que han visto los vigilantes de directorio.
    #[cfg(windows)]
    fn drain_walk_queue(&self) -> usize {
        let lote: Vec<WalkChange> = match self.walk_queue.lock() {
            Ok(mut q) => std::mem::take(&mut *q),
            Err(_) => return 0,
        };
        if lote.is_empty() {
            return 0;
        }

        let base = MmapIndex::open(&self.index_path).ok();
        let base_view = base.as_ref().and_then(|m| m.view().ok());
        let mut overlay = match self.overlay.lock() {
            Ok(o) => o,
            Err(_) => return 0,
        };

        let mut aplicados = 0usize;
        for cambio in lote {
            // Lo que hizo la propia aplicacion ya esta en el indice.
            if self
                .suppress
                .lock()
                .map(|w| w.is_suppressed(&cambio.full_path))
                .unwrap_or(false)
            {
                continue;
            }

            let hecho = match cambio.action {
                rdcw_action::ADDED | rdcw_action::RENAMED_NEW_NAME => {
                    let es_carpeta = Path::new(&cambio.full_path).is_dir();
                    apply_local_change(
                        &mut overlay,
                        base_view.as_ref(),
                        &LocalChangeKind::Created {
                            path: cambio.full_path.clone(),
                            is_dir: es_carpeta,
                        },
                    )
                }
                rdcw_action::REMOVED | rdcw_action::RENAMED_OLD_NAME => apply_local_change(
                    &mut overlay,
                    base_view.as_ref(),
                    &LocalChangeKind::Removed {
                        path: cambio.full_path.clone(),
                    },
                ),
                rdcw_action::MODIFIED => {
                    // Tamano o fecha: se anula y se vuelve a insertar, porque la
                    // capa solo sabe anadir y anular.
                    let es_carpeta = Path::new(&cambio.full_path).is_dir();
                    let quitado = apply_local_change(
                        &mut overlay,
                        base_view.as_ref(),
                        &LocalChangeKind::Removed {
                            path: cambio.full_path.clone(),
                        },
                    );
                    let puesto = apply_local_change(
                        &mut overlay,
                        base_view.as_ref(),
                        &LocalChangeKind::Created {
                            path: cambio.full_path.clone(),
                            is_dir: es_carpeta,
                        },
                    );
                    quitado || puesto
                }
                _ => false,
            };
            let _ = cambio.vol_id;
            if hecho {
                aplicados += 1;
            }
        }
        aplicados
    }

    /// Reescanea los volumenes apuntados y publica el indice resultante.
    ///
    /// Ocurre cuando el diario USN se pierde —es circular y una copia grande
    /// puede darle la vuelta— o cuando el usuario lo pide. Antes solo se
    /// escribia un aviso en el registro y el indice se quedaba desincronizado en
    /// silencio hasta reiniciar el servicio.
    #[cfg(windows)]
    fn run_pending_rescans(&self) {
        let pendientes: Vec<String> = match self.pending_rescans.lock() {
            Ok(mut v) => std::mem::take(&mut *v),
            Err(_) => return,
        };
        if pendientes.is_empty() {
            return;
        }
        tracing::warn!(
            "Reindexado completo de {} volumen(es): {:?}",
            pendientes.len(),
            pendientes
        );
        // Reconstruir el indice entero es lo unico seguro: un reescaneo parcial
        // dejaria fuera los borrados ocurridos en el hueco del diario.
        self.initial_scan();
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[allow(dead_code)]
    fn run_pending_rescans(&self) {}

    /// Reindexa entero cuando macOS pierde eventos.
    ///
    /// FSEvents avisa con `MUST_SCAN_SUBDIRS` cuando el núcleo descartó o no
    /// pudo entregar un lote; lo que pasó en ese hueco se ha perdido para el
    /// índice incremental y lo único seguro es reconstruir.
    #[cfg(target_os = "macos")]
    fn run_pending_rescans_mac(&self) {
        let pendientes: Vec<String> = match self.pending_rescans.lock() {
            Ok(mut v) => std::mem::take(&mut *v),
            Err(_) => return,
        };
        if pendientes.is_empty() {
            return;
        }
        tracing::warn!(
            "macOS: reindexado completo de {} volumen(es): {:?}",
            pendientes.len(),
            pendientes
        );
        self.initial_scan();
    }

    /// Crea un `FsEventWatcher` para cada volumen montado que aún no tiene uno.
    ///
    /// Se llama tras el escaneo inicial y cuando aparece un volumen nuevo. Los
    /// vigilantes ya existentes se conservan intactos: cada uno guarda su `since`
    /// y no hay que recrearlos, con el riesgo de perder su cursor.
    #[cfg(target_os = "macos")]
    fn rebuild_mac_watchers(&self) {
        use bdj_search_fs::macos::list_volumes;
        let volumes = list_volumes();
        let mut watches = match self.mac_watch.lock() {
            Ok(w) => w,
            Err(_) => return,
        };
        for vol in &volumes {
            if !vol.is_ready {
                continue;
            }
            if watches.iter().any(|w| w.mount_prefix == vol.path) {
                continue;
            }
            if !self.volume_is_indexed(&vol.path.to_string_lossy()) {
                tracing::info!(
                    "macOS: {} excluido por el usuario, se omite",
                    vol.path.display()
                );
                continue;
            }
            // El escaneo ya terminó: se toma el id actual para que el stream no
            // repita eventos viejos que podrían duplicar entradas.
            let since = bdj_search_fs::macos::FsEventWatcher::current_event_id();
            match bdj_search_fs::macos::FsEventWatcher::watch(&vol.path, Some(since)) {
                Ok(w) => {
                    tracing::info!("macOS: vigilando volumen {}", vol.path.display());
                    watches.push(MacVolumeWatch {
                        mount_prefix: vol.path.clone(),
                        last_event_id: since,
                        watcher: Some(w),
                    });
                }
                Err(e) => tracing::warn!(
                    "macOS: no se pudo vigilar {} ({e})",
                    vol.path.display()
                ),
            }
        }
    }

    /// Aplica a la capa lo que han visto los vigilantes FSEvents.
    #[cfg(target_os = "macos")]
    fn drain_mac_watchers(&self) -> usize {
        let mut watches = match self.mac_watch.lock() {
            Ok(w) => w,
            Err(_) => return 0,
        };
        let base = MmapIndex::open(&self.index_path).ok();
        let base_view = base.as_ref().and_then(|m| m.view().ok());
        let mut overlay = match self.overlay.lock() {
            Ok(o) => o,
            Err(_) => return 0,
        };
        let mut aplicados = 0usize;

        for w in watches.iter_mut() {
            let Some(watcher) = w.watcher.as_ref() else {
                continue;
            };
            let eventos = watcher.drain(Duration::from_millis(50));
            for ev in &eventos {
                if ev.event_id > w.last_event_id {
                    w.last_event_id = ev.event_id;
                }
                let ruta = ev.path.to_string_lossy().to_string();

                // Lo que hizo la propia aplicación ya está en el índice.
                if self
                    .suppress
                    .lock()
                    .map(|s| s.is_suppressed(&ruta))
                    .unwrap_or(false)
                {
                    continue;
                }

                if ev.must_rescan_subdirs {
                    let prefijo = w.mount_prefix.to_string_lossy().to_string();
                    tracing::warn!(
                        "macOS: eventos perdidos cerca de {} ({})",
                        w.mount_prefix.display(),
                        ruta
                    );
                    self.request_rescan(&prefijo);
                    continue;
                }

                // El tamaño y la fecha viven en columnas del índice: para un
                // `modified` se anula y se vuelve a insertar, como en Windows.
                let hecho = if ev.is_removed {
                    apply_local_change(
                        &mut overlay,
                        base_view.as_ref(),
                        &LocalChangeKind::Removed { path: ruta.clone() },
                    )
                } else if ev.is_created || ev.is_renamed || ev.is_modified {
                    let es_carpeta = ev.is_dir || Path::new(&ruta).is_dir();
                    let quitado = apply_local_change(
                        &mut overlay,
                        base_view.as_ref(),
                        &LocalChangeKind::Removed { path: ruta.clone() },
                    );
                    let puesto = apply_local_change(
                        &mut overlay,
                        base_view.as_ref(),
                        &LocalChangeKind::Created {
                            path: ruta.clone(),
                            is_dir: es_carpeta,
                        },
                    );
                    quitado || puesto
                } else {
                    false
                };
                if hecho {
                    aplicados += 1;
                }
            }
        }
        aplicados
    }

    /// Detecta memorias USB montadas o extraídas en caliente.
    ///
    /// Como FSEvents no cubre de forma fiable la aparición de volúmenes, se
    /// compara la lista de montajes con la de vigilantes cada pocas iteraciones.
    /// Un volumen recién conectado se indexa al momento —el USB de cabina, otra
    /// vez— y se queda vigilado.
    #[cfg(target_os = "macos")]
    fn poll_volume_changes_mac(&self) {
        use bdj_search_fs::macos::list_volumes;
        let volumes = list_volumes();

        let nuevos: Vec<PathBuf> = {
            let watches = match self.mac_watch.lock() {
                Ok(w) => w,
                Err(_) => return,
            };
            volumes
                .iter()
                .filter(|v| v.is_ready)
                .filter(|v| self.volume_is_indexed(&v.path.to_string_lossy()))
                .filter(|v| !watches.iter().any(|w| w.mount_prefix == v.path))
                .map(|v| v.path.clone())
                .collect()
        };

        for vol in &nuevos {
            self.index_new_volume_mac(vol);
        }

        {
            let mut watches = match self.mac_watch.lock() {
                Ok(w) => w,
                Err(_) => return,
            };
            watches.retain(|w| {
                let sigue = volumes.iter().any(|v| v.is_ready && v.path == w.mount_prefix);
                if !sigue {
                    tracing::info!(
                        "macOS: volumen desconectado, se deja de vigilar {}",
                        w.mount_prefix.display()
                    );
                }
                sigue
            });
        }

        if !nuevos.is_empty() {
            self.rebuild_mac_watchers();
        }
    }

    /// Recorre un volumen macOS recién conectado y mete su contenido en la capa.
    #[cfg(target_os = "macos")]
    fn index_new_volume_mac(&self, root: &Path) {
        use bdj_search_fs::macos::scan_subtree;

        let (label, fs_type, is_removable, is_ready) =
            match bdj_search_fs::macos::list_volumes()
                .into_iter()
                .find(|v| v.path == root)
            {
                Some(v) => (v.label, v.fs_type, v.is_removable, v.is_ready),
                None => {
                    tracing::info!("macOS: volumen desaparecido antes de indexarse");
                    return;
                }
            };
        if !is_ready {
            return;
        }

        tracing::info!("macOS: volumen nuevo detectado: {}", root.display());
        let mount = root.to_string_lossy().to_string();

        let mut overlay = match self.overlay.lock() {
            Ok(o) => o,
            Err(_) => return,
        };
        let vol_id =
            overlay
                .builder
                .vol_table
                .add_or_update(&mount, &label, &fs_type, is_removable);
        let root_id = overlay.add_entry(u32::MAX, "", true, false, false, vol_id, 0, 0, 0);

        let entradas = scan_subtree(root, usize::MAX);
        let mut ruta_a_id: HashMap<PathBuf, u32> = HashMap::new();
        ruta_a_id.insert(root.to_path_buf(), root_id);
        for e in &entradas {
            let padre = ruta_a_id.get(&e.parent_path).copied().unwrap_or(root_id);
            let id = overlay.add_entry(
                padre,
                &e.name,
                e.is_dir,
                e.is_hidden,
                e.is_system,
                vol_id,
                e.size,
                e.mtime,
                e.ctime,
            );
            if e.is_dir {
                let mut hijo = e.parent_path.clone();
                hijo.push(&e.name);
                ruta_a_id.insert(hijo, id);
            }
        }
        tracing::info!(
            "macOS: {}: {} entradas anadidas",
            root.display(),
            entradas.len()
        );
    }

    /// Fase 1 del indexado: nombres, extensiones y jerarquia real.
    ///
    /// Deja la busqueda por nombre utilizable en segundos. Los tamanos y las
    /// fechas no vienen en la MFT, asi que los completa despues `fill_metadata`
    /// recorriendo directorios en segundo plano.
    pub fn initial_scan(&self) -> (u64, usize) {
        tracing::info!("Iniciando escaneo inicial de volumenes...");
        let mut builder = IndexBuilder::new();

        #[cfg(windows)]
        let mut phase2_vols: Vec<u8> = Vec::new();

        #[cfg(windows)]
        {
            use bdj_search_fs::windows::list_volumes;

            let volumes = list_volumes();
            tracing::info!("Detectados {} volumenes logicos", volumes.len());

            let mut watch = self.watch.lock().unwrap();
            watch.volumes.clear();
            watch.frn_to_id.clear();

            for vol in &volumes {
                if !vol.is_ready {
                    continue;
                }
                // Un volumen que el usuario ha excluido no se toca: ni se
                // recorre ni se vigila.
                if !self.volume_is_indexed(&vol.path) {
                    tracing::info!("{}: excluido por el usuario, se omite", vol.path);
                    continue;
                }
                let (needs_phase2, watcher) =
                    scan_windows_volume(&mut builder, vol, &mut watch.frn_to_id);
                if let Some(vol_id) = needs_phase2 {
                    phase2_vols.push(vol_id);
                }
                match watcher {
                    Some(w) => watch.volumes.push(w),
                    None => {
                        // Sin diario USN: se vigila por recorrido de directorios.
                        let vol_id = builder.vol_table.add_or_update(
                            &vol.path,
                            &vol.label,
                            &vol.fs_type,
                            vol.is_removable,
                        );
                        watch.walk_volumes.push(WalkWatch {
                            mount_prefix: vol.path.clone(),
                            vol_id,
                        });
                    }
                }
            }

            tracing::info!(
                "Vigilando el diario USN de {} volumen(es) y por directorios {} mas",
                watch.volumes.len(),
                watch.walk_volumes.len()
            );
        }

        #[cfg(target_os = "macos")]
        {
            use bdj_search_fs::macos::{check_disk_access, list_volumes, scan_subtree};

            // Sin acceso total al disco, macOS no da error al listar el
            // Escritorio o Documentos: devuelve una lista vacía. El índice se
            // construiría «bien» y al usuario le faltarían justo sus carpetas.
            if let Some(aviso) = check_disk_access().message() {
                tracing::warn!("{aviso}");
            }

            let volumes = list_volumes();
            tracing::info!("macOS: detectados {} volumenes logicos", volumes.len());

            for vol in &volumes {
                if !vol.is_ready {
                    continue;
                }
                let vol_id = builder.vol_table.add_or_update(
                    &vol.path.to_string_lossy(),
                    &vol.label,
                    &vol.fs_type,
                    vol.is_removable,
                );

                let root_id = builder.add_entry(
                    u32::MAX, "", true, false, false, vol_id, 0, 0, 0,
                );

                tracing::info!("macOS: escaneando volumen {}", vol.path.display());
                let entries = scan_subtree(&vol.path, usize::MAX);

                let mut path_to_id: HashMap<PathBuf, u32> = HashMap::new();
                path_to_id.insert(vol.path.clone(), root_id);

                for entry in &entries {
                    let parent_id = path_to_id
                        .get(&entry.parent_path)
                        .copied()
                        .unwrap_or(root_id);

                    let idx = builder.add_entry(
                        parent_id,
                        &entry.name,
                        entry.is_dir,
                        entry.is_hidden,
                        entry.is_system,
                        vol_id,
                        entry.size,
                        entry.mtime,
                        entry.ctime,
                    );

                    if entry.is_dir {
                        let mut child_path = entry.parent_path.clone();
                        child_path.push(&entry.name);
                        path_to_id.insert(child_path, idx);
                    }
                }

                tracing::info!(
                    "{}: {} entradas indexadas en macOS",
                    vol.path.display(),
                    entries.len()
                );
            }

            // Todo lo existente ya está en el índice: desde aquí solo importan
            // los cambios, así que se arranca la vigilancia FSEvents.
            self.rebuild_mac_watchers();
        }

        let count = builder.count();
        tracing::info!("Fase 1 completada: {} entradas. Publicando indice...", count);
        self.write_index(&mut builder);

        #[cfg(windows)]
        {
            if !phase2_vols.is_empty() {
                tracing::info!("Fase 2: completando tamanos y fechas...");
                let filled = fill_metadata(&mut builder, &phase2_vols);
                builder.generation += 1;
                self.write_index(&mut builder);
                tracing::info!(
                    "Fase 2 completada: {} entradas con metadatos reales.",
                    filled
                );
            }
        }

        // La capa de cambios se ancla al indice publicado: sus identificadores
        // solo significan algo respecto a un base concreto.
        {
            let mut overlay = self.overlay.lock().unwrap();
            overlay.set_base_count(builder.count() as u32);
            overlay.builder.vol_table = builder.vol_table.clone();
        }

        // Se publica una capa vacía anclada al índice recién escrito.
        //
        // Si hubiera quedado en disco la capa de una ejecución anterior, la
        // aplicación la aplicaría sobre un índice que acaba de renumerar todas
        // sus entradas: los archivos aparecerían en carpetas equivocadas. La
        // comprobación de generación al leer ya lo impediría, pero publicar la
        // vacía es lo que deja el estado correcto en lugar de simplemente
        // ignorado.
        self.publish_overlay(builder.generation);

        (builder.generation, builder.count())
    }

    /// Publica el indice de forma atomica.
    ///
    /// Se escribe a un temporal y se renombra encima. El cliente mapea el
    /// archivo en solo lectura, y sin esto podria llegar a mapear un indice a
    /// medio escribir.
    fn write_index(&self, builder: &mut IndexBuilder) {
        let parent_dir = self.index_path.parent().unwrap_or(Path::new("."));
        if let Err(e) = std::fs::create_dir_all(parent_dir) {
            tracing::error!("No se pudo crear el directorio del indice: {e}");
            return;
        }

        let tmp = self.index_path.with_extension("bdjx.tmp");
        let file = match std::fs::File::create(&tmp) {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("No se pudo crear el indice temporal: {e}");
                return;
            }
        };

        let written = {
            let mut writer = std::io::BufWriter::with_capacity(4 * 1024 * 1024, file);
            builder.write_to(&mut writer)
        };

        match written {
            Ok(bytes) => {
                if let Err(e) = std::fs::rename(&tmp, &self.index_path) {
                    tracing::error!("No se pudo publicar el indice: {e}");
                } else {
                    tracing::info!("Indice publicado: {} bytes", bytes);
                }
            }
            Err(e) => {
                tracing::error!("Error escribiendo el indice: {e}");
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }

    /// Sondea el diario USN y aplica los cambios a la capa.
    ///
    /// Devuelve cuantos cambios se aplicaron y que volumenes se han quedado sin
    /// diario legible y necesitan un reindexado completo.
    #[cfg(windows)]
    fn poll_usn_changes(&self) -> (usize, Vec<String>) {
        use bdj_search_fs::windows::{UsnRecord, UsnScanner};

        let mut applied = 0usize;
        let mut needs_rescan: Vec<String> = Vec::new();
        let mut watch = self.watch.lock().unwrap();
        let WatchState { volumes, frn_to_id, .. } = &mut *watch;

        // El base se abre una vez por sondeo: hace falta para reconstruir la
        // ruta de una entrada nueva y poder leer su tamano y su fecha.
        let base = MmapIndex::open(&self.index_path).ok();
        let base_view = base.as_ref().and_then(|m| m.view().ok());

        let mut overlay = self.overlay.lock().unwrap();

        for watcher in volumes.iter_mut() {
            let scanner = match UsnScanner::open(watcher.drive_letter) {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!("{}: volumen no accesible ({e})", watcher.mount_prefix);
                    continue;
                }
            };

            let mut pending: Vec<UsnRecord> = Vec::new();
            let new_usn =
                match scanner.read_changes(watcher.journal_id, watcher.next_usn, |r| {
                    pending.push(r)
                }) {
                    Ok(usn) => usn,
                    Err(e) => {
                        // El diario se recreo o el cursor quedo fuera de rango
                        // (es circular, y una copia grande puede darle la vuelta).
                        // Lo ocurrido en ese hueco se ha perdido: reengancharse
                        // al final y seguir dejaria el indice desincronizado en
                        // silencio hasta el siguiente reinicio del servicio.
                        tracing::warn!(
                            "{}: diario USN ilegible ({e}). Se marca para reindexar.",
                            watcher.mount_prefix
                        );
                        if let Ok(cursor) = scanner.query_journal() {
                            watcher.journal_id = cursor.journal_id;
                            watcher.next_usn = cursor.next_usn;
                        }
                        needs_rescan.push(watcher.mount_prefix.clone());
                        continue;
                    }
                };

            for rec in &pending {
                if apply_usn_change(
                    &mut overlay,
                    base_view.as_ref(),
                    frn_to_id,
                    rec,
                    watcher,
                    &self.suppress,
                ) {
                    applied += 1;
                }
            }

            watcher.next_usn = new_usn;
        }

        (applied, needs_rescan)
    }

    /// Sondea los volumenes para detectar memorias USB conectadas o extraidas.
    ///
    /// Antes solo entraban los NTFS: `poll_volume_changes` filtraba por
    /// `vol.is_ntfs` y solo abria un lector del diario. Un pendrive exFAT
    /// conectado en caliente —el USB de cabina— no entraba al indice bajo
    /// ninguna circunstancia. Ahora los que no tienen diario se recorren y se
    /// quedan vigilados por directorios.
    #[cfg(windows)]
    fn poll_volume_changes(&self) {
        use bdj_search_fs::windows::list_volumes;
        let current_vols = list_volumes();

        let mut nuevos_sin_diario: Vec<String> = Vec::new();

        {
            let mut watch = self.watch.lock().unwrap();

            // 1. Volumenes recien conectados.
            for vol in &current_vols {
                if !vol.is_ready {
                    continue;
                }
                if !self.volume_is_indexed(&vol.path) {
                    continue;
                }
                let drive_char = vol.path.chars().next().unwrap_or(' ');
                let ya_vigilado = watch.volumes.iter().any(|w| w.drive_letter == drive_char)
                    || watch
                        .walk_volumes
                        .iter()
                        .any(|w| w.mount_prefix.eq_ignore_ascii_case(&vol.path));
                if ya_vigilado {
                    continue;
                }

                tracing::info!("Nuevo volumen detectado: {} ({})", vol.path, vol.fs_type);

                // Camino rapido: NTFS con diario accesible.
                let con_diario = if vol.is_ntfs {
                    bdj_search_fs::windows::UsnScanner::open(drive_char)
                        .ok()
                        .and_then(|s| s.query_journal().ok())
                } else {
                    None
                };

                let vol_id = {
                    let mut overlay = self.overlay.lock().unwrap();
                    overlay.builder.vol_table.add_or_update(
                        &vol.path,
                        &vol.label,
                        &vol.fs_type,
                        vol.is_removable,
                    )
                };
                let root_id = {
                    let mut overlay = self.overlay.lock().unwrap();
                    overlay.add_entry(u32::MAX, "", true, false, false, vol_id, 0, 0, 0)
                };

                match con_diario {
                    Some(cursor) => {
                        watch.volumes.push(VolumeWatch {
                            drive_letter: drive_char,
                            vol_id,
                            mount_prefix: vol.path.clone(),
                            root_id,
                            journal_id: cursor.journal_id,
                            next_usn: cursor.next_usn,
                        });
                        // Aun sin diario previo hay que leer lo que ya contiene.
                        nuevos_sin_diario.push(vol.path.clone());
                    }
                    None => {
                        watch.walk_volumes.push(WalkWatch {
                            mount_prefix: vol.path.clone(),
                            vol_id,
                        });
                        nuevos_sin_diario.push(vol.path.clone());
                    }
                }
            }

            // 2. Volumenes desconectados.
            watch.volumes.retain(|w| {
                let sigue = current_vols
                    .iter()
                    .any(|v| v.is_ready && v.path.chars().next() == Some(w.drive_letter));
                if !sigue {
                    tracing::info!("Volumen desconectado: {}:", w.drive_letter);
                }
                sigue
            });
            watch.walk_volumes.retain(|w| {
                let sigue = current_vols
                    .iter()
                    .any(|v| v.is_ready && v.path.eq_ignore_ascii_case(&w.mount_prefix));
                if !sigue {
                    tracing::info!("Volumen desconectado: {}", w.mount_prefix);
                }
                sigue
            });
        }

        // 3. Contenido de lo recien conectado.
        //
        // Se hace fuera del cerrojo de `watch` porque recorrer un pendrive lleno
        // puede tardar segundos y el bucle principal no debe quedarse esperando.
        for prefijo in nuevos_sin_diario {
            self.index_new_volume(&prefijo);
        }
    }

    /// Recorre un volumen recien conectado y mete su contenido en la capa.
    #[cfg(windows)]
    fn index_new_volume(&self, mount_prefix: &str) {
        let (vol_id, root_id) = {
            let overlay = self.overlay.lock().unwrap();
            let vol_id = overlay
                .builder
                .vol_table
                .volumes
                .iter()
                .find(|v| v.mount_prefix.eq_ignore_ascii_case(mount_prefix))
                .map(|v| v.id);
            let Some(vol_id) = vol_id else { return };
            let raiz = overlay
                .builder
                .parents
                .iter()
                .enumerate()
                .find(|&(i, p)| *p == u32::MAX && overlay.builder.volumes[i] == vol_id)
                .map(|(i, _)| overlay.base_count + i as u32);
            let Some(raiz) = raiz else { return };
            (vol_id, raiz)
        };

        tracing::info!("{mount_prefix}: leyendo contenido...");
        let entradas = bdj_search_fs::windows::scan_subtree(Path::new(mount_prefix), usize::MAX);

        let mut overlay = self.overlay.lock().unwrap();
        let mut ruta_a_id: HashMap<PathBuf, u32> = HashMap::new();
        ruta_a_id.insert(PathBuf::from(mount_prefix), root_id);

        for e in &entradas {
            let padre = ruta_a_id.get(&e.parent_path).copied().unwrap_or(root_id);
            let id = overlay.add_entry(
                padre,
                &e.name,
                e.is_dir,
                e.is_hidden,
                e.is_system,
                vol_id,
                e.size,
                e.mtime,
                e.ctime,
            );
            if e.is_dir {
                let mut hijo = e.parent_path.clone();
                hijo.push(&e.name);
                ruta_a_id.insert(hijo, id);
            }
        }
        tracing::info!("{mount_prefix}: {} entradas anadidas", entradas.len());

        // El volumen nuevo ya esta en la capa: hay que vigilarlo desde ahora.
        self.spawn_walk_watchers();
    }

    /// Bucle de mantenimiento: atiende la tuberia, sigue el diario de cambios y
    /// publica un indice nuevo cuando la actividad se calma.
    pub fn run_daemon(&self) {
        let running = self.running.clone();

        // 1. Servidor de la tuberia nombrada, en su propio hilo.
        //
        // Ahora las ordenes hacen algo: excluir un volumen reduce el indice, y
        // sin licencia el servicio deja de indexar. Antes `SetVolumeIndexed`,
        // `AddFolder` y `RescanVolume` solo escribian en el registro.
        #[cfg(windows)]
        let _ipc_thread = {
            let ipc_running = running.clone();
            let ipc_settings = self.settings.clone();
            let ipc_settings_path = self.settings_path.clone();
            let ipc_suppress = self.suppress.clone();
            let ipc_overlay = self.overlay.clone();
            let ipc_overlay_gen = self.overlay_generation.clone();
            let ipc_index_path = self.index_path.clone();
            let ipc_rescans = self.pending_rescans.clone();
            thread::spawn(move || {
                tracing::info!("Tuberia de control en {}", DEFAULT_PIPE_NAME);
                while ipc_running.load(Ordering::SeqCst) && GLOBAL_RUNNING.load(Ordering::SeqCst) {
                    if let Ok(server) = PipeServer::create(DEFAULT_PIPE_NAME)
                        && server.wait_for_client().is_ok()
                    {
                        while let Ok(cmd) = server.read_command() {
                            tracing::info!("Orden recibida: {:?}", cmd);
                            let respuesta = handle_ipc_command(
                                &cmd,
                                &ipc_settings,
                                &ipc_settings_path,
                                &ipc_suppress,
                                &ipc_overlay,
                                &ipc_overlay_gen,
                                &ipc_index_path,
                                &ipc_rescans,
                            );
                            let _ = server.send_event(&respuesta);
                        }
                        server.disconnect();
                    }
                    thread::sleep(Duration::from_millis(500));
                }
            })
        };

        // 1b. Servidor del socket Unix, en su propio hilo.
        //
        // Misma tubería de control que en Windows, con el socket del sistema:
        // excluir un volumen, anunciar cambios locales o apuntar reescaneos
        // funcionan igual en macOS.
        #[cfg(unix)]
        let _ipc_thread = {
            let ipc_running = running.clone();
            let ipc_settings = self.settings.clone();
            let ipc_settings_path = self.settings_path.clone();
            let ipc_suppress = self.suppress.clone();
            let ipc_overlay = self.overlay.clone();
            let ipc_overlay_gen = self.overlay_generation.clone();
            let ipc_index_path = self.index_path.clone();
            let ipc_rescans = self.pending_rescans.clone();
            let ipc_socket = default_unix_socket_path();
            thread::spawn(move || {
                tracing::info!("Tuberia de control en {}", ipc_socket.display());
                while ipc_running.load(Ordering::SeqCst) && GLOBAL_RUNNING.load(Ordering::SeqCst) {
                    match UnixSocketServer::bind(&ipc_socket) {
                        Ok(server) => {
                            if let Ok(mut stream) = server.accept() {
                                while let Ok(cmd) = UnixSocketServer::read_command(&mut stream) {
                                    tracing::info!("Orden recibida: {:?}", cmd);
                                    let respuesta = handle_ipc_command(
                                        &cmd,
                                        &ipc_settings,
                                        &ipc_settings_path,
                                        &ipc_suppress,
                                        &ipc_overlay,
                                        &ipc_overlay_gen,
                                        &ipc_index_path,
                                        &ipc_rescans,
                                    );
                                    let _ = UnixSocketServer::send_event(&mut stream, &respuesta);
                                }
                            }
                            // Al soltarse el servidor se elimina el archivo del
                            // socket, listo para reabrirse con el siguiente ciclo.
                            drop(server);
                        }
                        Err(e) => tracing::warn!(
                            "No se pudo abrir la tuberia de control ({e})"
                        ),
                    }
                    thread::sleep(Duration::from_millis(500));
                }
            })
        };

        // 1c. Vigilantes de directorio para los volumenes sin diario USN.
        #[cfg(windows)]
        self.spawn_walk_watchers();

        // 2. Bucle principal.
        //
        // El indice se republica cuando la actividad de disco se calma, no en
        // cada cambio: copiar una carpeta de 500 pistas debe producir una sola
        // reescritura y no quinientas. Tambien se publica si la capa crece
        // demasiado, para que una copia larga no deje la busqueda desfasada.
        const FAST_POLL: Duration = Duration::from_millis(500);
        let mut idle_cycles: u32 = 0;

        let base_count = {
            let overlay = self.overlay.lock().unwrap();
            overlay.base_count as usize
        };
        // Reescribir un indice de 10 millones cuesta segundos; uno de 100.000,
        // milisegundos. El retardo se ajusta al coste real de publicar. En
        // indices modestos se publica casi de inmediato (0,25 s): el "tiempo
        // real" de una carpeta normal se siente como el Explorador.
        // El periodo de calma sale del perfil del equipo y del tamaño del
        // índice: publicar la capa ya no depende de esto —eso ocurre en cuanto
        // hay cambios—, así que aquí solo se decide cada cuánto conviene
        // compactar, que es la operación cara.
        let base_calma = Tuning::current().quiet_period_ms;
        let quiet_period = if base_count > 2_000_000 {
            Duration::from_millis(base_calma * 4)
        } else {
            Duration::from_millis(base_calma)
        };

        let mut generation = 1u64;
        let mut last_change: Option<Instant> = None;

        tracing::info!(
            "Servicio activo. Sondeo base 500ms (reposo hasta 2-3.5s), publicacion tras {:?} de calma.",
            quiet_period
        );

        #[cfg(windows)]
        let mut usb_poll_counter = 0u8;

        while running.load(Ordering::SeqCst) && GLOBAL_RUNNING.load(Ordering::SeqCst) {
            let max_idle = if is_on_battery() {
                Duration::from_millis(3_500)
            } else {
                Duration::from_millis(2_000)
            };
            let current_poll = if idle_cycles < 10 {
                FAST_POLL
            } else {
                let extra = ((idle_cycles - 10) as u64 * 250).min(max_idle.as_millis() as u64 - 500);
                Duration::from_millis(500 + extra)
            };
            thread::sleep(current_poll);

            // Sin licencia activa —o con el indexado detenido por el usuario—
            // el servicio no sigue ningun cambio ni republica nada.
            if !self.may_index() {
                continue;
            }

            // Rutas silenciadas que ya han caducado.
            if let Ok(mut w) = self.suppress.lock() {
                w.purge();
            }

            #[cfg(windows)]
            {
                usb_poll_counter = (usb_poll_counter + 1) % 4;
                if usb_poll_counter == 0 {
                    self.poll_volume_changes();
                }
            }

            #[cfg(windows)]
            let found = {
                let (aplicados, sin_diario) = self.poll_usn_changes();
                for prefijo in sin_diario {
                    self.request_rescan(&prefijo);
                }
                let del_recorrido = self.drain_walk_queue();
                self.run_pending_rescans();
                aplicados + del_recorrido
            };
            #[cfg(target_os = "macos")]
            let found = {
                let aplicados = self.drain_mac_watchers();
                self.poll_volume_changes_mac();
                self.run_pending_rescans_mac();
                aplicados
            };
            #[cfg(not(any(windows, target_os = "macos")))]
            let found = 0usize;

            if found > 0 {
                idle_cycles = 0;
                tracing::debug!("{} cambios aplicados a la capa", found);
                last_change = Some(Instant::now());
                // Se publica **ya**, sin esperar a la compactación.
                //
                // Este es el cambio que hace que el producto funcione en tiempo
                // real. Antes el único camino por el que un cambio del disco
                // llegaba a la aplicación era reescribir el índice entero:
                // 950 MB y unos 25 segundos sobre diez millones de entradas, y
                // encima solo cuando el disco llevaba un rato quieto. Ahora se
                // escribe un archivo pequeño con lo que ha cambiado, que cuesta
                // milisegundos, y la compactación queda para lo que debía ser:
                // mantenimiento en segundo plano que nadie espera.
                self.publish_overlay(self.base_generation());
            } else {
                idle_cycles = idle_cycles.saturating_add(1);
            }

            let (has_changes, too_big) = {
                let overlay = self.overlay.lock().unwrap();
                (overlay.has_changes(), overlay.should_compact(base_count))
            };

            if !has_changes {
                continue;
            }

            let quiet_enough = last_change.map(|t| t.elapsed() >= quiet_period).unwrap_or(true);
            if !(quiet_enough || too_big) {
                continue;
            }

            generation += 1;
            let base = MmapIndex::open(&self.index_path).ok();
            let resultado = {
                let mut overlay = self.overlay.lock().unwrap();
                overlay.compact(base.as_ref(), &self.index_path, generation)
            };
            match resultado {
                Ok(bytes) => {
                    tracing::info!(
                        "Indice republicado (generacion {}, {} bytes)",
                        generation,
                        bytes
                    );
                    // La compactación ya metió estos cambios dentro del índice y
                    // vació la capa. Hay que publicar la capa vacía anclada a la
                    // generación nueva: si se dejara en disco la anterior, la
                    // aplicación aplicaría los mismos cambios dos veces —y con
                    // identificadores que la compactación acaba de renumerar—.
                    self.publish_overlay(generation);
                }
                Err(e) => tracing::error!("Fallo la republicacion del indice: {e}"),
            }
            last_change = None;
        }

        tracing::info!("Servicio detenido correctamente.");
    }
}
/// Escanea un volumen Windows e inserta sus entradas en el builder.
///
/// Para volúmenes NTFS/ReFS usa `FSCTL_ENUM_USN_DATA` (la MFT entera en
/// segundos). Para exFAT/FAT32 cae al recorrido de directorios clásico.
///
/// Devuelve `(Some(vol_id), Some(watcher))` cuando el diario USN queda listo
/// para vigilancia, o `(None, None)` si no hay diario.
#[cfg(windows)]
fn scan_windows_volume(
    builder: &mut IndexBuilder,
    vol: &bdj_search_fs::windows::VolumeInfo,
    frn_to_id: &mut HashMap<u64, u32>,
) -> (Option<u8>, Option<VolumeWatch>) {
    let drive_letter = vol.path.chars().next().unwrap_or('C');
    let vol_id = builder.vol_table.add_or_update(
        &vol.path, &vol.label, &vol.fs_type, vol.is_removable,
    );

    // La raíz del volumen: entrada sin nombre cuyo padre es u32::MAX.
    let root_id = builder.add_entry(
        u32::MAX, "", true, false, false, vol_id, 0, 0, 0,
    );

    let mut scanned_mft = false;
    let mut watcher_opt = None;

    if vol.is_ntfs {
        // --- Camino rápido: lectura directa de la MFT ---
        match bdj_search_fs::windows::UsnScanner::open(drive_letter) {
            Ok(scanner) => {
                let base_idx = builder.count() as u32;
                let mut frn_parent: Vec<(u64, u64)> = Vec::with_capacity(500_000);

                let cursor_res = scanner.enumerate_all(|rec| {
                    if rec.name.starts_with('$') {
                        return;
                    }

                    let idx = builder.add_entry(
                        u32::MAX,
                        &rec.name,
                        rec.is_dir,
                        (rec.attributes & FILE_ATTRIBUTE_HIDDEN) != 0,
                        (rec.attributes & FILE_ATTRIBUTE_SYSTEM) != 0,
                        vol_id,
                        0,
                        0,
                        0,
                    );

                    frn_to_id.insert(rec.file_ref, idx);
                    frn_parent.push((rec.file_ref, rec.parent_file_ref));
                });

                match cursor_res {
                    Ok(cursor) => {
                        for (file_frn, parent_frn) in &frn_parent {
                            if let Some(&child_idx) = frn_to_id.get(file_frn) {
                                let parent_idx = frn_to_id
                                    .get(parent_frn)
                                    .copied()
                                    .unwrap_or(root_id);
                                builder.set_parent(child_idx, parent_idx);
                            }
                        }

                        frn_to_id.entry(5).or_insert(root_id);

                        tracing::info!(
                            "{}: {} entradas leídas de la MFT",
                            vol.path,
                            frn_parent.len()
                        );

                        watcher_opt = Some(VolumeWatch {
                            drive_letter,
                            vol_id,
                            mount_prefix: vol.path.clone(),
                            root_id,
                            journal_id: cursor.journal_id,
                            next_usn: cursor.next_usn,
                        });
                        scanned_mft = true;
                    }
                    Err(e) => {
                        tracing::warn!("{}: falló la enumeración MFT ({e}). Usando recorrido de directorios...", vol.path);
                        builder.truncate_to(base_idx);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("{}: MFT no accesible directamente sin privilegios elevados ({e}). Recorriendo sistema de archivos...", vol.path);
            }
        }
    }

    if !scanned_mft {
        // --- Camino de recorrido de directorios (exFAT/FAT32 o NTFS sin elevación) ---
        tracing::info!("{}: escaneando por directorios ({})", vol.path, vol.fs_type);
        let path = std::path::Path::new(&vol.path);
        let count = bdj_search_fs::windows::scan_subtree_into_builder(path, root_id, vol_id, builder);

        tracing::info!(
            "{}: {} entradas indexadas del recorrido de directorios",
            vol.path,
            count
        );

        // Ya tiene metadata completa, no necesita phase 2
        return (None, None);
    }

    (Some(vol_id), watcher_opt)
}

/// Fase 2 del indexado NTFS: rellena tamaños y fechas.
///
/// La MFT solo entrega nombres y jerarquía. Los tamaños y las fechas se leen
/// recorriendo cada directorio una sola vez, agrupando hijos por padre para
/// minimizar las aperturas de directorio.
#[cfg(windows)]
fn fill_metadata(builder: &mut IndexBuilder, _vol_ids: &[u8]) -> usize {
    let pairs = builder.children_by_parent();
    let mut filled = 0usize;

    // Agrupar hijos por carpeta contenedora.
    let mut current_parent = u32::MAX;
    let mut current_children: Vec<u32> = Vec::with_capacity(256);

    for &(parent_id, child_id) in &pairs {
        if parent_id != current_parent {
            if !current_children.is_empty() {
                filled += fill_parent_metadata(builder, current_parent, &current_children);
                current_children.clear();
            }
            current_parent = parent_id;
        }
        current_children.push(child_id);
    }
    if !current_children.is_empty() {
        filled += fill_parent_metadata(builder, current_parent, &current_children);
    }

    filled
}

/// Rellena metadatos para todos los hijos de un directorio dado.
///
/// Abre el directorio padre una sola vez con `scan_directory` y cruza los
/// resultados con los hijos del índice por nombre.
#[cfg(windows)]
fn fill_parent_metadata(
    builder: &mut IndexBuilder,
    parent_id: u32,
    children: &[u32],
) -> usize {
    // Reconstruir la ruta del padre.
    let vol_id = builder.volume_of(parent_id);
    let mount_prefix = builder
        .vol_table
        .get(vol_id)
        .map(|v| v.mount_prefix.as_str())
        .unwrap_or("C:\\");
    let parent_path_str = builder.resolve_path(parent_id, mount_prefix);
    let parent_path = std::path::Path::new(&parent_path_str);

    if !parent_path.exists() {
        return 0;
    }

    // Leer el directorio una sola vez.
    let fs_entries = bdj_search_fs::windows::scan_directory(parent_path);

    // Mapa de nombre -> metadatos para cruce O(1).
    let mut name_map: HashMap<String, (u64, u32, u32)> = HashMap::with_capacity(fs_entries.len());
    for e in &fs_entries {
        name_map.insert(e.name.clone(), (e.size, e.mtime, e.ctime));
    }

    let mut filled = 0usize;
    for &child_id in children {
        let name = builder.name_at(child_id);
        if let Some(&(size, mtime, ctime)) = name_map.get(name) {
            builder.set_metadata(child_id, size, mtime, ctime);
            filled += 1;
        }
    }

    filled
}

/// Motivos del diario USN que interesan.
#[cfg(windows)]
mod usn_reason {
    pub const DATA_OVERWRITE: u32 = 0x0000_0001;
    pub const DATA_EXTEND: u32 = 0x0000_0002;
    pub const DATA_TRUNCATION: u32 = 0x0000_0004;
    pub const FILE_CREATE: u32 = 0x0000_0100;
    pub const FILE_DELETE: u32 = 0x0000_0200;
    pub const RENAME_OLD_NAME: u32 = 0x0000_1000;
    pub const RENAME_NEW_NAME: u32 = 0x0000_2000;
    pub const BASIC_INFO_CHANGE: u32 = 0x0000_8000;

    pub const REMOVED: u32 = FILE_DELETE | RENAME_OLD_NAME;
    pub const ADDED: u32 = FILE_CREATE | RENAME_NEW_NAME;
    pub const MODIFIED: u32 =
        DATA_OVERWRITE | DATA_EXTEND | DATA_TRUNCATION | BASIC_INFO_CHANGE;
}

/// Traduce un registro del diario en una operacion sobre la capa de cambios.
///
/// Un renombrado llega como dos registros: primero el nombre viejo, que anula la
/// entrada anterior, y despues el nuevo, que la vuelve a crear en su sitio. Una
/// modificacion se trata igual, porque el tamano y la fecha van en columnas del
/// indice y la capa solo sabe anadir y anular.
#[cfg(windows)]
fn apply_usn_change(
    overlay: &mut OverlayIndex,
    base_view: Option<&bdj_search_core::index::IndexView<'_>>,
    frn_to_id: &mut HashMap<u64, u32>,
    rec: &bdj_search_fs::windows::UsnRecord,
    watcher: &VolumeWatch,
    suppress: &Arc<Mutex<SuppressionWindow>>,
) -> bool {
    // La carpeta contenedora puede estar en el base o haberse creado hace un
    // instante en la propia capa; el mapa cubre ambos casos.
    let parent = frn_to_id
        .get(&rec.parent_file_ref)
        .copied()
        .unwrap_or(watcher.root_id);

    let parent_path = overlay.resolve_path(base_view, parent, &watcher.mount_prefix);
    let mut full = PathBuf::from(&parent_path);
    full.push(&rec.name);
    let full_str = full.to_string_lossy().to_string();

    // Si este cambio lo hizo la propia aplicacion hace un instante, ya esta en
    // el indice: aplicarlo otra vez duplicaria la entrada, o pondria una lapida
    // sobre la que se acaba de crear.
    if suppress
        .lock()
        .map(|w| w.is_suppressed(&full_str))
        .unwrap_or(false)
    {
        return false;
    }

    let removed = (rec.reason & usn_reason::REMOVED) != 0;
    let added = (rec.reason & usn_reason::ADDED) != 0;
    let modified = (rec.reason & usn_reason::MODIFIED) != 0;

    if removed || modified {
        if let Some(old_id) = frn_to_id.remove(&rec.file_ref) {
            overlay.mark_deleted(old_id);
        }
    }

    if !(added || modified) {
        return removed;
    }

    // Tamano y fecha se leen del disco: el diario no los trae.
    let (size, mtime, ctime) = match std::fs::metadata(&full) {
        Ok(md) => {
            let secs = |t: std::io::Result<std::time::SystemTime>| -> u32 {
                t.ok()
                    .and_then(|v| v.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as u32)
                    .unwrap_or(0)
            };
            (md.len(), secs(md.modified()), secs(md.created()))
        }
        // El archivo pudo desaparecer entre el registro y este momento: se
        // indexa igualmente por nombre y la siguiente pasada lo corrige.
        Err(_) => (0, 0, 0),
    };

    let new_id = overlay.add_entry(
        parent,
        &rec.name,
        rec.is_dir,
        (rec.attributes & FILE_ATTRIBUTE_HIDDEN) != 0,
        (rec.attributes & FILE_ATTRIBUTE_SYSTEM) != 0,
        watcher.vol_id,
        size,
        mtime,
        ctime,
    );
    frn_to_id.insert(rec.file_ref, new_id);
    true
}

/// Atiende una orden llegada por la tuberia y devuelve la respuesta.
///
/// Vive fuera de `IndexerService` para que el hilo de la tuberia solo necesite
/// los `Arc` que de verdad usa, sin prestar el servicio entero.
#[cfg(any(windows, unix))]
#[allow(clippy::too_many_arguments)]
/// Escribe `overlay.bdjo` junto al índice y sube su número de publicación.
///
/// Vive suelta —y no solo como método— porque el hilo de la tubería también
/// tiene que publicar: cuando la aplicación anuncia que acaba de copiar un
/// archivo, ese cambio debe verse en el fotograma siguiente y no en la próxima
/// vuelta del bucle de sondeo.
///
/// El bloqueo de la capa se suelta antes de volver: quien llama no debe tenerlo
/// tomado.
fn publish_overlay_file(
    overlay: &Arc<Mutex<OverlayIndex>>,
    overlay_generation: &Arc<AtomicU64>,
    index_path: &Path,
    base_generation: u64,
) {
    let siguiente = overlay_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let destino = overlay_path_for(index_path);
    let mut capa = match overlay.lock() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("No se pudo tomar la capa para publicarla: {e}");
            return;
        }
    };
    let entradas = capa.overlay_count();
    match write_overlay(&mut capa, base_generation, siguiente, &destino) {
        Ok(bytes) => tracing::debug!(
            "Capa publicada (base {base_generation}, capa {siguiente}, {entradas} entradas, {bytes} bytes)"
        ),
        Err(e) => tracing::error!("No se pudo publicar la capa de cambios: {e}"),
    }
}

/// Generación del índice base que hay publicado en `index_path`.
fn base_generation_of(index_path: &Path) -> u64 {
    MmapIndex::open(index_path)
        .ok()
        .and_then(|m| m.view().map(|v| v.header.generation).ok())
        .unwrap_or(0)
}

#[allow(clippy::too_many_arguments)]
fn handle_ipc_command(
    cmd: &IpcCommand,
    settings: &Arc<Mutex<IndexerSettings>>,
    settings_path: &Path,
    suppress: &Arc<Mutex<SuppressionWindow>>,
    overlay: &Arc<Mutex<OverlayIndex>>,
    overlay_generation: &Arc<AtomicU64>,
    index_path: &Path,
    rescans: &Arc<Mutex<Vec<String>>>,
) -> IpcEvent {
    let guardar = |s: &IndexerSettings| {
        if let Err(e) = s.save(settings_path) {
            tracing::warn!("No se pudieron guardar los ajustes: {e}");
        }
    };

    match cmd {
        IpcCommand::EnableIndexing => {
            if let Ok(mut s) = settings.lock() {
                s.indexing_enabled = true;
                guardar(&s);
            }
            IpcEvent::Ack
        }
        IpcCommand::DisableIndexing => {
            if let Ok(mut s) = settings.lock() {
                s.indexing_enabled = false;
                guardar(&s);
            }
            IpcEvent::Ack
        }
        IpcCommand::SetVolumeIndexed {
            mount_prefix,
            indexed,
        } => {
            if let Ok(mut s) = settings.lock() {
                s.set_volume_indexed(mount_prefix, *indexed);
                guardar(&s);
            }
            // Cambiar que se indexa obliga a reconstruir: excluir un volumen
            // tiene que sacar sus entradas del indice, no solo dejar de seguirlo.
            if let Ok(mut v) = rescans.lock() {
                v.push(mount_prefix.clone());
            }
            IpcEvent::Ack
        }
        IpcCommand::AddFolder { path } => {
            if !Path::new(path).is_dir() {
                return IpcEvent::Error {
                    message: format!("«{path}» no es una carpeta."),
                };
            }
            if let Ok(mut s) = settings.lock() {
                s.add_folder(path);
                guardar(&s);
            }
            if let Ok(mut v) = rescans.lock() {
                v.push(path.clone());
            }
            IpcEvent::Ack
        }
        IpcCommand::RemoveFolder { path } => {
            if let Ok(mut s) = settings.lock() {
                s.remove_folder(path);
                guardar(&s);
            }
            if let Ok(mut v) = rescans.lock() {
                v.push(path.clone());
            }
            IpcEvent::Ack
        }
        IpcCommand::RescanVolume { mount_prefix } => {
            if let Ok(mut v) = rescans.lock() {
                v.push(mount_prefix.clone());
            }
            IpcEvent::Ack
        }
        IpcCommand::SetLicenseState { active, expires_at } => {
            if let Ok(mut s) = settings.lock() {
                s.license_active = *active;
                s.license_expires_at = *expires_at;
                guardar(&s);
            }
            tracing::info!(
                "Licencia {} (caduca en {})",
                if *active { "activa" } else { "inactiva" },
                expires_at
            );
            IpcEvent::Ack
        }
        IpcCommand::LocalChanges { changes } => {
            let base = MmapIndex::open(index_path).ok();
            let base_view = base.as_ref().and_then(|m| m.view().ok());
            {
                let Ok(mut ov) = overlay.lock() else {
                    return IpcEvent::Error {
                        message: "El indice esta ocupado.".into(),
                    };
                };
                for c in changes {
                    let kind = LocalChangeKind::from(c);
                    // Primero se silencia y despues se aplica: si se hiciera al
                    // reves, un evento del sistema de archivos podria colarse entre
                    // ambos pasos y aplicarse dos veces.
                    if let Ok(mut w) = suppress.lock() {
                        for p in kind.paths() {
                            w.suppress(p);
                        }
                    }
                    apply_local_change(&mut ov, base_view.as_ref(), &kind);
                }
            }

            // Se publica antes de responder.
            //
            // Esta orden la manda la aplicación justo después de terminar una
            // copia o un renombrado, y se queda esperando el `Ack`. Publicando
            // aquí, cuando la respuesta llega la capa ya está en disco y el
            // refresco que hace la aplicación a continuación ve el archivo
            // nuevo. Si se dejara para la próxima vuelta del bucle de sondeo,
            // el usuario pulsaría «pegar» y no vería nada durante medio segundo
            // largo, que es justo lo que hacía pensar que la operación falló.
            publish_overlay_file(
                overlay,
                overlay_generation,
                index_path,
                base_generation_of(index_path),
            );
            IpcEvent::Ack
        }
        IpcCommand::GetStatus => {
            let (indexing_enabled, license_active, excluidos) = match settings.lock() {
                Ok(s) => (
                    s.indexing_enabled,
                    s.may_index(now_secs()),
                    s.excluded_volumes.clone(),
                ),
                Err(_) => (false, false, Default::default()),
            };

            let base = MmapIndex::open(index_path).ok();
            let vista = base.as_ref().and_then(|m| m.view().ok());
            let (generation, entry_count) = vista
                .as_ref()
                .map(|v| (v.header.generation, v.header.entry_count))
                .unwrap_or((0, 0));

            let mut volumes = Vec::new();
            if let Some(v) = vista.as_ref() {
                for vol in &v.vol_table.volumes {
                    let cuantas = (0..v.entry_count())
                        .filter(|&i| v.volume[i] == vol.id && v.is_alive(i))
                        .count() as u64;
                    volumes.push(VolumeStatus {
                        mount_prefix: vol.mount_prefix.clone(),
                        label: vol.label.clone(),
                        fs_type: vol.fs_type.clone(),
                        is_connected: Path::new(&vol.mount_prefix).exists(),
                        is_indexed: !excluidos
                            .iter()
                            .any(|e| e.eq_ignore_ascii_case(&vol.mount_prefix)),
                        entry_count: cuantas,
                    });
                }
            }

            IpcEvent::Status {
                indexing_enabled,
                license_active,
                generation,
                entry_count,
                volumes,
            }
        }
    }
}

/// Aplica al indice un cambio que hizo la propia aplicacion.
///
/// Es lo que hace que copiar un archivo desde la aplicacion se vea en la lista
/// al instante, sin esperar al ciclo de compactacion. La ruta se traduce a una
/// entrada con la columna de hijos del indice, sin tocar el disco salvo para
/// leer tamano y fecha de lo que se acaba de crear.
///
/// Devuelve `true` si el indice cambió. Un `false` significa que la ruta no se
/// pudo situar —una carpeta que el indice aun no conoce— y que conviene dejar
/// que el vigilante del sistema de archivos lo resuelva por su cuenta.
#[cfg_attr(not(windows), allow(dead_code))]
fn apply_local_change(
    overlay: &mut OverlayIndex,
    base_view: Option<&bdj_search_core::index::IndexView<'_>>,
    change: &LocalChangeKind,
) -> bool {
    match change {
        LocalChangeKind::Removed { path } => {
            match overlay.resolve_id_by_path(base_view, path) {
                Some(id) => {
                    overlay.mark_deleted(id);
                    true
                }
                None => false,
            }
        }
        LocalChangeKind::Created { path, is_dir } => {
            add_path_to_overlay(overlay, base_view, path, *is_dir)
        }
        LocalChangeKind::Renamed { from, to } => {
            let mut cambio = false;
            if let Some(id) = overlay.resolve_id_by_path(base_view, from) {
                overlay.mark_deleted(id);
                cambio = true;
            }
            let es_carpeta = Path::new(to).is_dir();
            cambio |= add_path_to_overlay(overlay, base_view, to, es_carpeta);
            cambio
        }
    }
}

/// Inserta en la capa una ruta recien creada, colgandola de su carpeta.
#[cfg_attr(not(windows), allow(dead_code))]
fn add_path_to_overlay(
    overlay: &mut OverlayIndex,
    base_view: Option<&bdj_search_core::index::IndexView<'_>>,
    path: &str,
    is_dir: bool,
) -> bool {
    let p = Path::new(path);
    let (Some(parent_path), Some(name)) = (
        p.parent().map(|x| x.to_string_lossy().to_string()),
        p.file_name().map(|x| x.to_string_lossy().to_string()),
    ) else {
        return false;
    };

    let Some(parent_id) = overlay.resolve_id_by_path(base_view, &parent_path) else {
        return false;
    };

    // Si ya estaba —el vigilante se adelanto— no se duplica.
    if overlay.resolve_id_by_path(base_view, path).is_some() {
        return false;
    }

    let vol_id = if parent_id < overlay.base_count {
        base_view
            .map(|v| v.volume[parent_id as usize])
            .unwrap_or(0)
    } else {
        overlay.builder.volume_of(parent_id - overlay.base_count)
    };

    let (size, mtime, ctime) = match std::fs::metadata(p) {
        Ok(md) => {
            let secs = |t: std::io::Result<std::time::SystemTime>| -> u32 {
                t.ok()
                    .and_then(|v| v.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as u32)
                    .unwrap_or(0)
            };
            (md.len(), secs(md.modified()), secs(md.created()))
        }
        Err(_) => (0, 0, 0),
    };

    overlay.add_entry(parent_id, &name, is_dir, false, false, vol_id, size, mtime, ctime);

    // Una carpeta recien copiada trae dentro todo un arbol: se recorre y se
    // añade entero, porque el usuario espera encontrar su contenido.
    if is_dir {
        add_subtree_to_overlay(overlay, base_view, p, vol_id, 0);
    }
    true
}

/// Profundidad maxima al añadir un arbol recien copiado.
#[cfg_attr(not(windows), allow(dead_code))]
const MAX_SUBTREE_DEPTH: usize = 32;

#[cfg_attr(not(windows), allow(dead_code))]
fn add_subtree_to_overlay(
    overlay: &mut OverlayIndex,
    base_view: Option<&bdj_search_core::index::IndexView<'_>>,
    dir: &Path,
    vol_id: u8,
    depth: usize,
) {
    if depth >= MAX_SUBTREE_DEPTH {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let padre = dir.to_string_lossy().to_string();
    let Some(parent_id) = overlay.resolve_id_by_path(base_view, &padre) else {
        return;
    };
    for entry in rd.flatten() {
        let Ok(md) = entry.metadata() else { continue };
        let nombre = entry.file_name().to_string_lossy().to_string();
        let secs = |t: std::io::Result<std::time::SystemTime>| -> u32 {
            t.ok()
                .and_then(|v| v.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as u32)
                .unwrap_or(0)
        };
        overlay.add_entry(
            parent_id,
            &nombre,
            md.is_dir(),
            false,
            false,
            vol_id,
            md.len(),
            secs(md.modified()),
            secs(md.created()),
        );
        if md.is_dir() {
            add_subtree_to_overlay(overlay, base_view, &entry.path(), vol_id, depth + 1);
        }
    }
}

/// Espejo portable de `bdj_search_ipc::LocalChange`.
///
/// Existe para que la logica de aplicacion se pueda compilar y probar tambien
/// donde la tuberia de Windows no esta disponible.
#[derive(Debug, Clone, PartialEq)]
pub enum LocalChangeKind {
    Created { path: String, is_dir: bool },
    Removed { path: String },
    Renamed { from: String, to: String },
}

#[cfg(any(windows, unix))]
impl From<&LocalChange> for LocalChangeKind {
    fn from(c: &LocalChange) -> Self {
        match c {
            LocalChange::Created { path, is_dir } => LocalChangeKind::Created {
                path: path.clone(),
                is_dir: *is_dir,
            },
            LocalChange::Removed { path } => LocalChangeKind::Removed { path: path.clone() },
            LocalChange::Renamed { from, to } => LocalChangeKind::Renamed {
                from: from.clone(),
                to: to.clone(),
            },
        }
    }
}

impl LocalChangeKind {
    /// Rutas que hay que silenciar tras aplicar el cambio.
    pub fn paths(&self) -> Vec<&str> {
        match self {
            LocalChangeKind::Created { path, .. } | LocalChangeKind::Removed { path } => {
                vec![path.as_str()]
            }
            LocalChangeKind::Renamed { from, to } => vec![from.as_str(), to.as_str()],
        }
    }
}

#[cfg(windows)]
define_windows_service!(ffi_service_main, my_service_main);

#[cfg(windows)]
fn my_service_main(_arguments: Vec<std::ffi::OsString>) {
    if let Ok(status_handle) = service_control_handler::register(SERVICE_NAME, move |control_event| {
        match control_event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                // Sin esto, `sc stop` se cuelga: el manejador devolvía NoError
                // pero el bucle principal nunca se enteraba.
                GLOBAL_RUNNING.store(false, Ordering::SeqCst);
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    }) {
        let _ = status_handle.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        });

        let index_path = get_index_path();
        let service = IndexerService::new(index_path);
        service.initial_scan();
        service.run_daemon();

        let _ = status_handle.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::Stopped,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        });
    }
}

fn main() {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = std::env::args().collect();
    #[cfg_attr(not(windows), allow(unused_variables))]
    let is_standalone = args.iter().any(|a| a == "--standalone" || a == "run" || a == "-s");

    #[cfg(windows)]
    {
        if !is_standalone {
            // Attempt to dispatch as Windows Service
            if service_dispatcher::start(SERVICE_NAME, ffi_service_main).is_ok() {
                return;
            }
        }
    }

    // Standalone / Developer Mode execution
    println!("Running BDJ Studio Search Pro Indexer in Standalone / Console mode...");
    let index_path = get_index_path();
    let service = IndexerService::new(index_path);
    let (gen_id, count) = service.initial_scan();
    println!("Initial scan completed: {} files (Generation {})", count, gen_id);

    // Run daemon loop in foreground
    service.run_daemon();
}


#[cfg(test)]
mod tests {
    use super::*;
    use bdj_search_core::index::IndexBuilder;
    use std::io::BufWriter;
    use tempfile::{NamedTempFile, tempdir};

    /// Indice minimo cuya raiz apunta a una carpeta real del disco, para poder
    /// comprobar que los cambios anunciados por la aplicacion se situan bien.
    fn indice_sobre(dir: &Path) -> (NamedTempFile, MmapIndex) {
        let mut b = IndexBuilder::new();
        let prefijo = format!("{}{}", dir.to_string_lossy(), std::path::MAIN_SEPARATOR);
        b.vol_table.add_or_update(&prefijo, "prueba", "NTFS", false);
        let raiz = b.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
        b.add_entry(raiz, "Musica", true, false, false, 0, 0, 1, 1);

        let temp = NamedTempFile::new().unwrap();
        let file = std::fs::File::create(temp.path()).unwrap();
        let mut w = BufWriter::new(file);
        b.write_to(&mut w).unwrap();
        drop(w);
        let mmap = MmapIndex::open(temp.path()).unwrap();
        (temp, mmap)
    }

    #[test]
    fn un_archivo_creado_por_la_aplicacion_aparece_al_instante() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Musica")).unwrap();
        let archivo = dir.path().join("Musica").join("kick.wav");
        std::fs::write(&archivo, vec![0u8; 1234]).unwrap();

        let (_t, mmap) = indice_sobre(dir.path());
        let view = mmap.view().unwrap();
        let mut overlay = OverlayIndex::with_base_count(view.entry_count() as u32);
        overlay.builder.vol_table = view.vol_table.clone();

        let cambio = LocalChangeKind::Created {
            path: archivo.to_string_lossy().to_string(),
            is_dir: false,
        };
        assert!(apply_local_change(&mut overlay, Some(&view), &cambio));

        let id = overlay
            .resolve_id_by_path(Some(&view), &archivo.to_string_lossy())
            .expect("el archivo deberia estar ya en el indice");
        assert!(id >= overlay.base_count, "deberia vivir en la capa");
        assert_eq!(overlay.builder.sizes[(id - overlay.base_count) as usize], 1234);
    }

    #[test]
    fn una_carpeta_copiada_entra_con_todo_su_contenido() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Musica")).unwrap();
        let sets = dir.path().join("Musica").join("Sets");
        std::fs::create_dir(&sets).unwrap();
        std::fs::write(sets.join("a.als"), b"x").unwrap();
        std::fs::create_dir(sets.join("2026")).unwrap();
        std::fs::write(sets.join("2026").join("b.als"), b"y").unwrap();

        let (_t, mmap) = indice_sobre(dir.path());
        let view = mmap.view().unwrap();
        let mut overlay = OverlayIndex::with_base_count(view.entry_count() as u32);
        overlay.builder.vol_table = view.vol_table.clone();

        let cambio = LocalChangeKind::Created {
            path: sets.to_string_lossy().to_string(),
            is_dir: true,
        };
        assert!(apply_local_change(&mut overlay, Some(&view), &cambio));

        for esperado in [
            sets.join("a.als"),
            sets.join("2026"),
            sets.join("2026").join("b.als"),
        ] {
            assert!(
                overlay
                    .resolve_id_by_path(Some(&view), &esperado.to_string_lossy())
                    .is_some(),
                "falta {} en el indice",
                esperado.display()
            );
        }
    }

    #[test]
    fn un_borrado_anunciado_anula_la_entrada() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Musica")).unwrap();
        let (_t, mmap) = indice_sobre(dir.path());
        let view = mmap.view().unwrap();
        let mut overlay = OverlayIndex::with_base_count(view.entry_count() as u32);
        overlay.builder.vol_table = view.vol_table.clone();

        let ruta = dir.path().join("Musica").to_string_lossy().to_string();
        assert!(overlay.resolve_id_by_path(Some(&view), &ruta).is_some());

        let cambio = LocalChangeKind::Removed { path: ruta.clone() };
        assert!(apply_local_change(&mut overlay, Some(&view), &cambio));
        assert!(
            overlay.resolve_id_by_path(Some(&view), &ruta).is_none(),
            "una carpeta borrada no debe seguir encontrandose"
        );
    }

    #[test]
    fn no_se_duplica_si_el_vigilante_ya_lo_habia_anadido() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Musica")).unwrap();
        let archivo = dir.path().join("Musica").join("snare.wav");
        std::fs::write(&archivo, b"x").unwrap();

        let (_t, mmap) = indice_sobre(dir.path());
        let view = mmap.view().unwrap();
        let mut overlay = OverlayIndex::with_base_count(view.entry_count() as u32);
        overlay.builder.vol_table = view.vol_table.clone();

        let cambio = LocalChangeKind::Created {
            path: archivo.to_string_lossy().to_string(),
            is_dir: false,
        };
        assert!(apply_local_change(&mut overlay, Some(&view), &cambio));
        let tras_el_primero = overlay.overlay_count();

        // Segundo intento: el mismo cambio, llegado esta vez por el sistema de
        // archivos. No debe anadir nada.
        assert!(!apply_local_change(&mut overlay, Some(&view), &cambio));
        assert_eq!(overlay.overlay_count(), tras_el_primero);
    }

    #[test]
    fn una_ruta_que_el_indice_no_conoce_se_deja_al_vigilante() {
        let dir = tempdir().unwrap();
        let (_t, mmap) = indice_sobre(dir.path());
        let view = mmap.view().unwrap();
        let mut overlay = OverlayIndex::with_base_count(view.entry_count() as u32);
        overlay.builder.vol_table = view.vol_table.clone();

        let cambio = LocalChangeKind::Created {
            path: "Z:\\otro disco\\x.wav".into(),
            is_dir: false,
        };
        assert!(
            !apply_local_change(&mut overlay, Some(&view), &cambio),
            "sin poder situar la ruta, hay que devolver falso y no inventarse nada"
        );
    }

    #[test]
    fn un_renombrado_anunciado_mueve_la_entrada() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Musica")).unwrap();
        let viejo = dir.path().join("Musica").join("viejo.wav");
        let nuevo = dir.path().join("Musica").join("nuevo.wav");
        std::fs::write(&viejo, b"x").unwrap();

        let (_t, mmap) = indice_sobre(dir.path());
        let view = mmap.view().unwrap();
        let mut overlay = OverlayIndex::with_base_count(view.entry_count() as u32);
        overlay.builder.vol_table = view.vol_table.clone();

        apply_local_change(
            &mut overlay,
            Some(&view),
            &LocalChangeKind::Created {
                path: viejo.to_string_lossy().to_string(),
                is_dir: false,
            },
        );
        std::fs::rename(&viejo, &nuevo).unwrap();

        assert!(apply_local_change(
            &mut overlay,
            Some(&view),
            &LocalChangeKind::Renamed {
                from: viejo.to_string_lossy().to_string(),
                to: nuevo.to_string_lossy().to_string(),
            }
        ));

        assert!(overlay.resolve_id_by_path(Some(&view), &nuevo.to_string_lossy()).is_some());
        assert!(overlay.resolve_id_by_path(Some(&view), &viejo.to_string_lossy()).is_none());
    }
}
