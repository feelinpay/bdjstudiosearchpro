use std::os::raw::{c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use fsevent_sys::core_foundation::{
    kCFAllocatorDefault, kCFRunLoopDefaultMode, kCFStringEncodingUTF8, kCFTypeArrayCallBacks,
    CFArrayAppendValue, CFArrayCreateMutable, CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef,
    CFIndex, CFRelease, CFStringCreateWithCString, CFStringGetCString, CFStringRef,
    CFRunLoopGetCurrent, CFRunLoopRef, CFRunLoopRun, CFRunLoopStop,
};
use fsevent_sys::{
    kFSEventStreamCreateFlagFileEvents, kFSEventStreamCreateFlagNoDefer,
    kFSEventStreamCreateFlagUseCFTypes, kFSEventStreamCreateFlagWatchRoot,
    kFSEventStreamEventIdSinceNow, FSEventStreamContext, FSEventStreamCreate,
    FSEventStreamEventFlags, FSEventStreamEventId, FSEventStreamInvalidate, FSEventStreamRef,
    FSEventStreamRelease, FSEventStreamScheduleWithRunLoop, FSEventStreamStart, FSEventStreamStop,
    FSEventsGetCurrentEventId,
};

#[derive(Debug, Clone)]
pub struct MacFsChangeEvent {
    pub path: PathBuf,
    pub event_id: u64,
    pub is_created: bool,
    pub is_removed: bool,
    pub is_renamed: bool,
    pub is_modified: bool,
    pub is_dir: bool,
    pub must_rescan_subdirs: bool,
}

// Flags definidas por Darwin FSEvents.h
pub const K_FSEVENT_STREAM_EVENT_FLAG_NONE: u32 = 0x00000000;
pub const K_FSEVENT_STREAM_EVENT_FLAG_MUST_SCAN_SUBDIRS: u32 = 0x00000001;
pub const K_FSEVENT_STREAM_EVENT_FLAG_USER_DROPPED: u32 = 0x00000002;
pub const K_FSEVENT_STREAM_EVENT_FLAG_KERNEL_DROPPED: u32 = 0x00000004;
pub const K_FSEVENT_STREAM_EVENT_FLAG_EVENT_IDS_WRAPPED: u32 = 0x00000008;
pub const K_FSEVENT_STREAM_EVENT_FLAG_HISTORY_DONE: u32 = 0x00000010;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ROOT_CHANGED: u32 = 0x00000020;
pub const K_FSEVENT_STREAM_EVENT_FLAG_MOUNT: u32 = 0x00000040;
pub const K_FSEVENT_STREAM_EVENT_FLAG_UNMOUNT: u32 = 0x00000080;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_CREATED: u32 = 0x00000100;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_REMOVED: u32 = 0x00000200;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_INODE_META_MOD: u32 = 0x00000400;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_RENAMED: u32 = 0x00000800;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_MODIFIED: u32 = 0x00001000;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_FINDER_INFO_MOD: u32 = 0x00002000;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_CHANGE_OWNER: u32 = 0x00004000;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_XATTR_MOD: u32 = 0x00008000;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_IS_FILE: u32 = 0x00010000;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_IS_DIR: u32 = 0x00020000;
pub const K_FSEVENT_STREAM_EVENT_FLAG_ITEM_IS_SYMLINK: u32 = 0x00040000;

/// Decodifica la máscara de flags cruda de Darwin en un evento de alto nivel.
pub fn parse_event(path: PathBuf, event_id: u64, flags: u32) -> MacFsChangeEvent {
    let is_created = (flags & K_FSEVENT_STREAM_EVENT_FLAG_ITEM_CREATED) != 0;
    let is_removed = (flags & K_FSEVENT_STREAM_EVENT_FLAG_ITEM_REMOVED) != 0;
    let is_renamed = (flags & K_FSEVENT_STREAM_EVENT_FLAG_ITEM_RENAMED) != 0;
    let is_modified = (flags & K_FSEVENT_STREAM_EVENT_FLAG_ITEM_MODIFIED) != 0;
    let is_dir = (flags & K_FSEVENT_STREAM_EVENT_FLAG_ITEM_IS_DIR) != 0;
    let must_rescan = (flags & (K_FSEVENT_STREAM_EVENT_FLAG_MUST_SCAN_SUBDIRS
        | K_FSEVENT_STREAM_EVENT_FLAG_USER_DROPPED
        | K_FSEVENT_STREAM_EVENT_FLAG_KERNEL_DROPPED)) != 0;

    MacFsChangeEvent {
        path,
        event_id,
        is_created,
        is_removed,
        is_renamed,
        is_modified,
        is_dir,
        must_rescan_subdirs: must_rescan,
    }
}

/// Cola compartida entre el hilo del run loop de FSEvents y quien consume.
struct CallbackCtx {
    tx: Sender<MacFsChangeEvent>,
}

/// Callback externo que Darwin invoca desde su propio run loop.
///
/// Con `kFSEventStreamCreateFlagUseCFTypes`, `eventPaths` apunta a un
/// `CFArrayRef` cuyos elementos son `CFStringRef`. Se releen los tres arrays
/// (rutas, flags e ids) con el mismo índice.
extern "C" fn fsevents_callback(
    _stream: FSEventStreamRef,
    info: *mut c_void,
    _num: usize,
    paths: *mut c_void,
    flags: *const FSEventStreamEventFlags,
    ids: *const FSEventStreamEventId,
) {
    let ctx = unsafe { &*(info as *const CallbackCtx) };
    let cf_array = unsafe { *(paths as *const CFArrayRef) };
    if cf_array.is_null() {
        return;
    }
    let count = unsafe { CFArrayGetCount(cf_array) };
    for i in 0..count {
        let cf_path = unsafe { CFArrayGetValueAtIndex(cf_array, i) };
        let path = unsafe { cf_string_to_string(cf_path) };
        if path.is_empty() {
            continue;
        }
        let f = unsafe { *flags.add(i as usize) };
        let id = unsafe { *ids.add(i as usize) };
        let ev = parse_event(PathBuf::from(path), id, f);
        if ev.is_created
            || ev.is_removed
            || ev.is_renamed
            || ev.is_modified
            || ev.must_rescan_subdirs
        {
            // `try_send`: si el consumidor va con retraso tras una copia masiva,
            // se descartan eventos viejos en lugar de crecer la memoria sin
            // limite. El bucle principal reescanea si detecta `must_rescan`.
            let _ = ctx.tx.try_send(ev);
        }
    }
}

/// Convierte una `CFStringRef` UTF-8 en un `String`. Dado que FSEvents entrega
/// rutas absolutas reales, la conversión debe aguantar nombres largos.
unsafe fn cf_string_to_string(cf: CFStringRef) -> String {
    let mut buf = [0u8; 8192];
    let ok = unsafe {
        CFStringGetCString(
            cf,
            buf.as_mut_ptr() as *mut c_char,
            buf.len() as CFIndex,
            kCFStringEncodingUTF8,
        )
    };
    if ok {
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).to_string()
    } else {
        String::new()
    }
}

/// Puntos de control compartidos entre el hilo del run loop y el consumidor.
#[derive(Default)]
struct WatcherControl {
    stop: AtomicBool,
    run_loop_ref: AtomicUsize,
}

/// Vigilante de cambios de directorio de macOS basado en FSEvents.
///
/// Cada instancia crea un `FSEventStream` sobre una ruta, lo programa en el
/// run loop de un hilo propio y entrega los eventos por un canal. `drain`
/// saca todo lo pendiente sin bloquear más de lo indicado, lo que encaja con
/// el sondeo del servicio de indexado. Al soltar el vigilante se despierta y
/// detiene el hilo y se libera el stream.
pub struct FsEventWatcher {
    watch_path: PathBuf,
    rx: Receiver<MacFsChangeEvent>,
    /// Se conserva un `Sender` aquí para que el canal no se cierre si alguien
    /// soltara el contexto del callback antes.
    _tx: Sender<MacFsChangeEvent>,
    control: Arc<WatcherControl>,
    ctx_info: *mut CallbackCtx,
    /// El stream no copia el array de rutas que se le pasa: hay que mantenerlo
    /// vivo mientras el stream exista. Se liberan juntos en `Drop`.
    cf_path: CFStringRef,
    cf_arr: CFArrayRef,
    thread: Option<thread::JoinHandle<()>>,
}

// Seguridad: el `*mut CallbackCtx` solo se desreferencia en el hilo del run
// loop (callback) o, para liberarlo, justo después de `join` del hilo en
// `Drop`. Nunca hay acceso simultáneo, así que es seguro mover el manejador a
// otro hilo (el servicio lo guarda en su estado y lo recoge desde su bucle).
unsafe impl Send for FsEventWatcher {}

impl FsEventWatcher {
    /// Crea y arranca la vigilancia de `path`.
    ///
    /// `since` es el último id de evento conocido: con él el stream repite los
    /// cambios que sucedieron entre el cierre y la reapertura. `None` significa
    /// «desde ahora».
    pub fn watch(path: &Path, since: Option<u64>) -> std::io::Result<Self> {
        let c_path = std::ffi::CString::new(path.to_string_lossy().as_bytes())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        let (tx, rx) = crossbeam_channel::unbounded();
        let ctx = Box::new(CallbackCtx { tx: tx.clone() });
        let ctx_info = Box::into_raw(ctx);

        // El array de rutas a vigilar: una sola, la raíz del volumen.
        let (stream, cf_path, cf_arr) = unsafe {
            let cf_path = CFStringCreateWithCString(
                kCFAllocatorDefault,
                c_path.as_ptr(),
                kCFStringEncodingUTF8,
            );
            if cf_path.is_null() {
                let _ = Box::from_raw(ctx_info);
                return Err(std::io::Error::other(
                    "CFStringCreateWithCString devolvió null",
                ));
            }
            let cf_arr = CFArrayCreateMutable(kCFAllocatorDefault, 1, &kCFTypeArrayCallBacks);
            CFArrayAppendValue(cf_arr, cf_path);

            let context = FSEventStreamContext {
                version: 0,
                info: ctx_info as *mut c_void,
                retain: None,
                release: None,
                copy_description: None,
            };
            let stream = FSEventStreamCreate(
                kCFAllocatorDefault,
                fsevents_callback,
                &context,
                cf_arr,
                since.unwrap_or(kFSEventStreamEventIdSinceNow),
                0.35,
                kFSEventStreamCreateFlagFileEvents
                    | kFSEventStreamCreateFlagNoDefer
                    | kFSEventStreamCreateFlagUseCFTypes
                    | kFSEventStreamCreateFlagWatchRoot,
            );
            (stream, cf_path, cf_arr)
        };

        if stream.is_null() {
            unsafe {
                CFRelease(cf_arr);
                CFRelease(cf_path);
            }
            let _ = unsafe { Box::from_raw(ctx_info) };
            return Err(std::io::Error::other("FSEventStreamCreate devolvió null"));
        }

        let control = Arc::new(WatcherControl::default());
        let ctrl = control.clone();
        // El stream solo se usa dentro del hilo del run loop. Un puntero crudo
        // no es `Send`, así que se transporta como `usize` (reconstruido al
        // entrar en el hilo); la gestión de su ciclo de vida sigue en `Drop`.
        let stream_slot = stream as usize;
        let thread = thread::Builder::new()
            .name("bdj-fsevents".into())
            .spawn(move || {
                // El stream se programa sobre el run loop del hilo en el que
                // vive: CFRunLoopGetCurrent debe ejecutarse aquí dentro.
                let stream = stream_slot as FSEventStreamRef;
                let rl = unsafe { CFRunLoopGetCurrent() };
                ctrl.run_loop_ref.store(rl as usize, Ordering::Release);
                unsafe {
                    FSEventStreamScheduleWithRunLoop(stream, rl, kCFRunLoopDefaultMode);
                    if FSEventStreamStart(stream) == 0 {
                        FSEventStreamInvalidate(stream);
                        FSEventStreamRelease(stream);
                        return;
                    }
                    // Si `Drop` ya pidió la parada, no se entra al run loop: se
                    // desmonta el stream directamente. De lo contrario
                    // `CFRunLoopRun` se queda dentro hasta que `CFRunLoopStop`
                    // (llamado desde `Drop`) lo despierte y haga que regrese.
                    if !ctrl.stop.load(Ordering::Acquire) {
                        CFRunLoopRun();
                    }
                    FSEventStreamStop(stream);
                    FSEventStreamInvalidate(stream);
                    FSEventStreamRelease(stream);
                }
            })
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        Ok(FsEventWatcher {
            watch_path: path.to_path_buf(),
            rx,
            _tx: tx,
            control,
            ctx_info,
            cf_path,
            cf_arr,
            thread: Some(thread),
        })
    }

    /// Último id de evento conocido por el sistema para poder reengancharse.
    pub fn current_event_id() -> u64 {
        unsafe { FSEventsGetCurrentEventId() }
    }

    /// Ruta que se está vigilando.
    pub fn watch_path(&self) -> &Path {
        &self.watch_path
    }

    /// Saca todos los eventos pendientes, esperando como mucho `timeout`.
    pub fn drain(&self, timeout: Duration) -> Vec<MacFsChangeEvent> {
        let mut out = Vec::new();
        if let Ok(first) = self.rx.recv_timeout(timeout) {
            out.push(first);
            // El primer lote de FSEvents tras arrancar puede traer cientos de
            // eventos; se drena todo lo que ya está en el canal de una vez.
            while let Ok(ev) = self.rx.try_recv() {
                out.push(ev);
                if out.len() >= 2048 {
                    break;
                }
            }
        }
        out
    }
}

impl Drop for FsEventWatcher {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::Release);
        // Despertar el run loop desde fuera: CFRunLoopStop es seguro entre
        // hilos y hará que CFRunLoopRun regrese y el hilo mire la bandera. Si
        // el hilo aún no ha guardado su run loop (ref == 0), él mismo revisa la
        // bandera antes de entrar y no se queda bloqueado.
        let rl = self.control.run_loop_ref.load(Ordering::Acquire);
        if rl != 0 {
            unsafe { CFRunLoopStop(rl as CFRunLoopRef) };
        }
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
        unsafe {
            if !self.cf_arr.is_null() {
                CFRelease(self.cf_arr);
            }
            if !self.cf_path.is_null() {
                CFRelease(self.cf_path);
            }
            if !self.ctx_info.is_null() {
                // El stream ya se liberó en el hilo: el contexto solo lo usaba
                // el callback, que no volverá a invocarse.
                drop(Box::from_raw(self.ctx_info));
            }
        }
    }
}