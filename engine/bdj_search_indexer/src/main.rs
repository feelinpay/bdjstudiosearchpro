use bdj_search_core::index::{IndexBuilder, MmapIndex, OverlayIndex};
#[cfg(windows)]
use bdj_search_ipc::{PipeServer, DEFAULT_PIPE_NAME, IpcCommand, IpcEvent};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[cfg(windows)]
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x02;
#[cfg(windows)]
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x04;

/// Bandera global para que el manejador de Stop del servicio pueda detener el
/// bucle principal sin necesidad de referencias cruzadas.
static GLOBAL_RUNNING: AtomicBool = AtomicBool::new(true);

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
        // Un agente de usuario arranca con el directorio de trabajo en "/", asi
        // que una ruta relativa dejaria el indice donde el cliente no lo busca.
        let p = PathBuf::from("/Library/Application Support/BDJ Studio/Search Pro");
        if std::fs::create_dir_all(&p).is_ok() {
            return p.join("index.bdjx");
        }
        if let Ok(home) = std::env::var("HOME") {
            let p = PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("BDJ Studio")
                .join("Search Pro");
            let _ = std::fs::create_dir_all(&p);
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

/// Lo que hace falta para traducir un cambio del diario en una operacion sobre
/// el indice: de que entrada habla y de que carpeta cuelga.
#[cfg(windows)]
#[derive(Default)]
struct WatchState {
    volumes: Vec<VolumeWatch>,
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
    #[cfg(windows)]
    watch: Arc<Mutex<WatchState>>,
}

impl IndexerService {
    pub fn new(index_path: PathBuf) -> Self {
        Self {
            running: Arc::new(AtomicBool::new(true)),
            overlay: Arc::new(Mutex::new(OverlayIndex::new())),
            index_path,
            #[cfg(windows)]
            watch: Arc::new(Mutex::new(WatchState::default())),
        }
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        GLOBAL_RUNNING.store(false, Ordering::SeqCst);
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
                let (needs_phase2, watcher) =
                    scan_windows_volume(&mut builder, vol, &mut watch.frn_to_id);
                if let Some(vol_id) = needs_phase2 {
                    phase2_vols.push(vol_id);
                }
                if let Some(w) = watcher {
                    watch.volumes.push(w);
                }
            }

            tracing::info!(
                "Vigilando el diario USN de {} volumenes",
                watch.volumes.len()
            );
        }

        #[cfg(target_os = "macos")]
        {
            use bdj_search_fs::macos::{list_volumes, scan_subtree};

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

    /// Sondea el diario USN y aplica los cambios a la capa. Devuelve cuantos.
    #[cfg(windows)]
    fn poll_usn_changes(&self) -> usize {
        use bdj_search_fs::windows::{UsnRecord, UsnScanner};

        let mut applied = 0usize;
        let mut watch = self.watch.lock().unwrap();
        let WatchState { volumes, frn_to_id } = &mut *watch;

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
                        // (es circular). Se reengancha al final y se avisa: lo
                        // ocurrido en medio requiere reindexar ese volumen.
                        tracing::warn!(
                            "{}: diario USN ilegible ({e}). Se reengancha al final.",
                            watcher.mount_prefix
                        );
                        if let Ok(cursor) = scanner.query_journal() {
                            watcher.journal_id = cursor.journal_id;
                            watcher.next_usn = cursor.next_usn;
                        }
                        continue;
                    }
                };

            for rec in &pending {
                apply_usn_change(
                    &mut overlay,
                    base_view.as_ref(),
                    frn_to_id,
                    rec,
                    watcher,
                );
                applied += 1;
            }

            watcher.next_usn = new_usn;
        }

        applied
    }

    /// Sondea volúmenes Windows para detectar memorias USB conectadas o extraídas.
    #[cfg(windows)]
    fn poll_volume_changes(&self) {
        use bdj_search_fs::windows::list_volumes;
        let current_vols = list_volumes();
        let mut watch = self.watch.lock().unwrap();

        // 1. Detectar volúmenes recién conectados
        for vol in &current_vols {
            if !vol.is_ready {
                continue;
            }
            let drive_char = vol.path.chars().next().unwrap_or(' ');
            let already_watched = watch.volumes.iter().any(|w| w.drive_letter == drive_char);
            if !already_watched && vol.is_ntfs {
                tracing::info!("Nuevo volumen detectado (USB/NTFS): {}", vol.path);
                let scanner = match bdj_search_fs::windows::UsnScanner::open(drive_char) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                if let Ok(cursor) = scanner.query_journal() {
                    let vol_id = {
                        let mut overlay = self.overlay.lock().unwrap();
                        overlay.builder.vol_table.add_or_update(
                            &vol.path, &vol.label, &vol.fs_type, vol.is_removable,
                        )
                    };
                    let root_id = {
                        let mut overlay = self.overlay.lock().unwrap();
                        overlay.add_entry(u32::MAX, "", true, false, false, vol_id, 0, 0, 0)
                    };
                    watch.volumes.push(VolumeWatch {
                        drive_letter: drive_char,
                        vol_id,
                        mount_prefix: vol.path.clone(),
                        root_id,
                        journal_id: cursor.journal_id,
                        next_usn: cursor.next_usn,
                    });
                }
            }
        }

        // 2. Detectar volúmenes desconectados (extraídos)
        watch.volumes.retain(|w| {
            let still_present = current_vols.iter().any(|v| {
                v.is_ready && v.path.chars().next() == Some(w.drive_letter)
            });
            if !still_present {
                tracing::info!("Volumen desconectado (USB): {}:", w.drive_letter);
            }
            still_present
        });
    }

    /// Bucle de mantenimiento: atiende la tuberia, sigue el diario de cambios y
    /// publica un indice nuevo cuando la actividad se calma.
    pub fn run_daemon(&self) {
        let running = self.running.clone();

        // Bandera que controla si el indexado está activo (EnableIndexing/DisableIndexing).
        let indexing_enabled = Arc::new(AtomicBool::new(true));

        // 1. Servidor de la tuberia nombrada, en su propio hilo.
        #[cfg(windows)]
        let _ipc_thread = {
            let ipc_running = running.clone();
            let ipc_indexing = indexing_enabled.clone();
            thread::spawn(move || {
                tracing::info!("Tuberia de control en {}", DEFAULT_PIPE_NAME);
                while ipc_running.load(Ordering::SeqCst) && GLOBAL_RUNNING.load(Ordering::SeqCst) {
                    if let Ok(server) = PipeServer::create(DEFAULT_PIPE_NAME)
                        && server.wait_for_client().is_ok()
                    {
                        while let Ok(cmd) = server.read_command() {
                            tracing::info!("Orden recibida: {:?}", cmd);
                            match cmd {
                                IpcCommand::EnableIndexing => {
                                    ipc_indexing.store(true, Ordering::SeqCst);
                                    tracing::info!("Indexado habilitado por IPC");
                                }
                                IpcCommand::DisableIndexing => {
                                    ipc_indexing.store(false, Ordering::SeqCst);
                                    tracing::info!("Indexado deshabilitado por IPC");
                                }
                                IpcCommand::RescanVolume { volume_id } => {
                                    let _ = server.send_event(&IpcEvent::IndexingProgress {
                                        volume_id,
                                        entries_scanned: 0,
                                        is_done: true,
                                    });
                                }
                                IpcCommand::SetVolumeIndexed { volume_id, indexed } => {
                                    tracing::info!(
                                        "Volumen {} indexado: {}",
                                        volume_id,
                                        indexed
                                    );
                                    // TODO: almacenar la configuración por volumen
                                }
                                IpcCommand::AddFolder { ref path } => {
                                    tracing::info!("Carpeta añadida: {}", path);
                                    // TODO: añadir carpeta al escaneo
                                }
                            }
                        }
                        server.disconnect();
                    }
                    thread::sleep(Duration::from_millis(500));
                }
            })
        };

        // 2. Bucle principal.
        //
        // El indice se republica cuando la actividad de disco se calma, no en
        // cada cambio: copiar una carpeta de 500 pistas debe producir una sola
        // reescritura y no quinientas. Tambien se publica si la capa crece
        // demasiado, para que una copia larga no deje la busqueda desfasada.
        const POLL_INTERVAL: Duration = Duration::from_millis(500);

        let base_count = {
            let overlay = self.overlay.lock().unwrap();
            overlay.base_count as usize
        };
        // Reescribir un indice de 10 millones cuesta segundos; uno de 100.000,
        // milisegundos. El retardo se ajusta al coste real de publicar.
        let quiet_period = if base_count > 2_000_000 {
            Duration::from_secs(2)
        } else {
            Duration::from_millis(500)
        };

        let mut generation = 1u64;
        let mut last_change: Option<Instant> = None;

        tracing::info!(
            "Servicio activo. Sondeo cada {:?}, publicacion tras {:?} de calma.",
            POLL_INTERVAL,
            quiet_period
        );

        #[cfg(windows)]
        let mut usb_poll_counter = 0u8;

        while running.load(Ordering::SeqCst) && GLOBAL_RUNNING.load(Ordering::SeqCst) {
            thread::sleep(POLL_INTERVAL);

            if !indexing_enabled.load(Ordering::SeqCst) {
                continue;
            }

            #[cfg(windows)]
            {
                usb_poll_counter = (usb_poll_counter + 1) % 4;
                if usb_poll_counter == 0 {
                    self.poll_volume_changes();
                }
            }

            #[cfg(windows)]
            let found = self.poll_usn_changes();
            #[cfg(not(windows))]
            let found = 0usize;

            if found > 0 {
                tracing::debug!("{} cambios aplicados a la capa", found);
                last_change = Some(Instant::now());
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
            let mut overlay = self.overlay.lock().unwrap();
            match overlay.compact(base.as_ref(), &self.index_path, generation) {
                Ok(bytes) => tracing::info!(
                    "Indice republicado (generacion {}, {} bytes)",
                    generation,
                    bytes
                ),
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

    if vol.is_ntfs {
        // --- Camino rápido: lectura directa de la MFT ---
        let scanner = match bdj_search_fs::windows::UsnScanner::open(drive_letter) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("{}: no se pudo abrir el volumen NTFS ({e})", vol.path);
                return (None, None);
            }
        };

        // El punto de partida antes del escaneo servirá como base del id
        let base_idx = builder.count() as u32;

        // Mapa temporal: FRN -> parent FRN (para resolución posterior)
        let mut frn_parent: Vec<(u64, u64)> = Vec::with_capacity(500_000);

        let cursor = match scanner.enumerate_all(|rec| {
            // Saltar los nombres de sistema NTFS internos ($MFT, $Extend, etc.)
            if rec.name.starts_with('$') {
                return;
            }

            let idx = builder.add_entry(
                u32::MAX, // padre provisional, se corrige abajo
                &rec.name,
                rec.is_dir,
                (rec.attributes & FILE_ATTRIBUTE_HIDDEN) != 0,
                (rec.attributes & FILE_ATTRIBUTE_SYSTEM) != 0,
                vol_id,
                0, // tamaño: se completa en fill_metadata
                0, // mtime: se completa en fill_metadata
                0, // ctime: se completa en fill_metadata
            );

            frn_to_id.insert(rec.file_ref, idx);
            frn_parent.push((rec.file_ref, rec.parent_file_ref));
        }) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("{}: fallo la enumeración MFT ({e})", vol.path);
                // Deshacer las entradas parciales
                builder.truncate_to(base_idx);
                return (None, None);
            }
        };

        // Segunda pasada: resolver los padres ahora que todos los FRN están en
        // el mapa. Un archivo cuyo padre no se encuentra cuelga de la raíz del
        // volumen: suele ser un archivo dentro de $Extend u otra carpeta de
        // sistema que no hemos indexado.
        for (file_frn, parent_frn) in &frn_parent {
            if let Some(&child_idx) = frn_to_id.get(file_frn) {
                let parent_idx = frn_to_id
                    .get(parent_frn)
                    .copied()
                    .unwrap_or(root_id);
                builder.set_parent(child_idx, parent_idx);
            }
        }

        // La raíz de la MFT (FRN 5 en NTFS) no aparece siempre en la
        // enumeración. La entrada raíz del volumen se asocia al FRN de la raíz.
        frn_to_id.entry(5).or_insert(root_id);

        tracing::info!(
            "{}: {} entradas leídas de la MFT",
            vol.path,
            frn_parent.len()
        );

        let watcher = VolumeWatch {
            drive_letter,
            vol_id,
            mount_prefix: vol.path.clone(),
            root_id,
            journal_id: cursor.journal_id,
            next_usn: cursor.next_usn,
        };

        (Some(vol_id), Some(watcher))
    } else {
        // --- Camino lento: recorrido de directorios para exFAT/FAT32 ---
        tracing::info!("{}: escaneando por directorios ({})", vol.path, vol.fs_type);
        let path = std::path::Path::new(&vol.path);
        let entries = bdj_search_fs::windows::scan_subtree(path, usize::MAX);

        // Necesitamos un mapa de ruta-padre para asignar padres correctamente.
        let mut path_to_id: HashMap<PathBuf, u32> = HashMap::new();
        path_to_id.insert(path.to_path_buf(), root_id);

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
            "{}: {} entradas del recorrido de directorios",
            vol.path,
            entries.len()
        );

        (None, None)
    }
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
) {
    let removed = (rec.reason & usn_reason::REMOVED) != 0;
    let added = (rec.reason & usn_reason::ADDED) != 0;
    let modified = (rec.reason & usn_reason::MODIFIED) != 0;

    if removed || modified {
        if let Some(old_id) = frn_to_id.remove(&rec.file_ref) {
            overlay.mark_deleted(old_id);
        }
    }

    if !(added || modified) {
        return;
    }

    // La carpeta contenedora puede estar en el base o haberse creado hace un
    // instante en la propia capa; el mapa cubre ambos casos.
    let parent = frn_to_id
        .get(&rec.parent_file_ref)
        .copied()
        .unwrap_or(watcher.root_id);

    // Tamano y fecha se leen del disco: el diario no los trae.
    let parent_path = overlay.resolve_path(base_view, parent, &watcher.mount_prefix);
    let mut full = PathBuf::from(&parent_path);
    full.push(&rec.name);

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
