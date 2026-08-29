use bdj_search_core::index::{IndexBuilder, MmapIndex, OverlayIndex};
use bdj_search_ipc::{PipeServer, DEFAULT_PIPE_NAME, IpcCommand, IpcEvent};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

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

const SERVICE_NAME: &str = "BDJSearchProIndexer";

fn get_index_path() -> PathBuf {
    if let Ok(program_data) = std::env::var("ProgramData") {
        let p = PathBuf::from(program_data).join("BDJ Studio").join("Search Pro");
        let _ = std::fs::create_dir_all(&p);
        p.join("index.bdjx")
    } else if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(local_app_data).join("BDJ Studio").join("Search Pro");
        let _ = std::fs::create_dir_all(&p);
        p.join("index.bdjx")
    } else {
        PathBuf::from("index.bdjx")
    }
}

pub struct IndexerService {
    running: Arc<AtomicBool>,
    overlay: Arc<Mutex<OverlayIndex>>,
    index_path: PathBuf,
}

impl IndexerService {
    pub fn new(index_path: PathBuf) -> Self {
        Self {
            running: Arc::new(AtomicBool::new(true)),
            overlay: Arc::new(Mutex::new(OverlayIndex::new())),
            index_path,
        }
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Perform initial volume scan and generate primary index
    pub fn initial_scan(&self) -> (u64, usize) {
        tracing::info!("Starting initial storage scan...");
        let mut builder = IndexBuilder::new();

        #[cfg(windows)]
        {
            use bdj_search_fs::windows::{list_volumes, scan_subtree, UsnScanner};

            let volumes = list_volumes();
            tracing::info!("Discovered {} logical volumes", volumes.len());

            for vol in &volumes {
                if !vol.is_ready {
                    continue;
                }

                let vol_id = builder.vol_table.add_or_update(
                    &vol.path,
                    &vol.label,
                    &vol.fs_type,
                    vol.is_ready,
                );

                // Add Volume Root Directory
                let root_id = builder.add_entry(
                    u32::MAX,
                    &vol.path,
                    true,
                    false,
                    false,
                    vol_id,
                    0,
                    1700000000,
                    1700000000,
                );

                // Try NTFS USN Journal scan first if elevated
                let mut usn_success = false;
                if vol.is_ntfs
                    && let Some(letter) = vol.path.chars().next()
                    && let Ok(scanner) = UsnScanner::open(letter)
                {
                    tracing::info!("Accessing NTFS USN journal on {}:...", letter);
                    let scan_res = scanner.enumerate_all(|rec| {
                        builder.add_entry(
                            root_id,
                            &rec.name,
                            rec.is_dir,
                            (rec.attributes & 0x2) != 0,
                            (rec.attributes & 0x4) != 0,
                            vol_id,
                            0, // Size populated in Phase 2
                            1700000000,
                            1700000000,
                        );
                    });
                    if scan_res.is_ok() {
                        usn_success = true;
                    }
                }

                // Fallback for non-NTFS or non-elevated user mode
                if !usn_success {
                    tracing::info!("Scanning directory tree on {} (Large Fetch fallback)...", vol.path);
                    let entries = scan_subtree(Path::new(&vol.path), 3);
                    for e in entries {
                        builder.add_entry(
                            root_id,
                            &e.name,
                            e.is_dir,
                            e.is_hidden,
                            e.is_system,
                            vol_id,
                            e.size,
                            e.mtime,
                            e.ctime,
                        );
                    }
                }
            }
        }

        let count = builder.count();
        tracing::info!("Index populated with {} entries. Serializing to disk...", count);

        let parent_dir = self.index_path.parent().unwrap_or(Path::new("."));
        let _ = std::fs::create_dir_all(parent_dir);

        if let Ok(file) = std::fs::File::create(&self.index_path) {
            let mut writer = std::io::BufWriter::with_capacity(4 * 1024 * 1024, file);
            let _ = builder.write_to(&mut writer);
        }

        (builder.generation, count)
    }

    /// Background runner maintaining IPC named pipe server and periodic compaction
    pub fn run_daemon(&self) {
        let running = self.running.clone();
        let index_path = self.index_path.clone();
        let overlay = self.overlay.clone();

        // 1. Spawn IPC Named Pipe Server Thread
        let ipc_running = running.clone();
        let _ipc_thread = thread::spawn(move || {
            #[cfg(windows)]
            {
                tracing::info!("Initializing Named Pipe server on {}", DEFAULT_PIPE_NAME);
                while ipc_running.load(Ordering::SeqCst) {
                    if let Ok(server) = PipeServer::create(DEFAULT_PIPE_NAME)
                        && server.wait_for_client().is_ok()
                    {
                        tracing::info!("Client connected to Named Pipe");
                        while let Ok(cmd) = server.read_command() {
                            tracing::info!("Received IPC command: {:?}", cmd);
                            if let IpcCommand::RescanVolume { volume_id } = cmd {
                                let _ = server.send_event(&IpcEvent::IndexingProgress {
                                    volume_id,
                                    entries_scanned: 100,
                                    is_done: true,
                                });
                            }
                        }
                        server.disconnect();
                    }
                    thread::sleep(Duration::from_millis(500));
                }
            }
        });

        // 2. Main Maintenance & Compaction Loop
        tracing::info!("BDJSearchProIndexer service active. Entering low-power standby...");
        let mut generation = 1u64;

        while running.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_secs(5));

            let should_compact = {
                let ov = overlay.lock().unwrap();
                ov.should_compact(1000)
            };

            if should_compact {
                generation += 1;
                tracing::info!("Compacting index to generation {}...", generation);
                let base = MmapIndex::open(&index_path).ok();
                let mut ov = overlay.lock().unwrap();
                let _ = ov.compact(base.as_ref(), &index_path, generation);
            }
        }

        tracing::info!("BDJSearchProIndexer service gracefully stopped.");
    }
}

#[cfg(windows)]
define_windows_service!(ffi_service_main, my_service_main);

#[cfg(windows)]
fn my_service_main(_arguments: Vec<std::ffi::OsString>) {
    if let Ok(status_handle) = service_control_handler::register(SERVICE_NAME, move |control_event| {
        match control_event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
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
