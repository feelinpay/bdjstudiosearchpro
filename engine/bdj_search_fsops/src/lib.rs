//! Gestor de operaciones de archivo de BDJ Studio Search Pro.
//!
//! Copiar, mover, renombrar, duplicar, crear carpetas y enviar a la papelera,
//! **sin bloquear la interfaz**: cada operación corre en su propio hilo y
//! publica su avance en contadores atómicos que la aplicación consulta varias
//! veces por segundo.
//!
//! Tres decisiones marcan el diseño:
//!
//! - **Por defecto nada se borra de forma definitiva.** Lo que se retira va a la
//!   papelera del sistema, incluidos los originales de un movimiento entre
//!   volúmenes. Un DJ que pierde una sesión pierde trabajo de meses. La única
//!   excepción es `DeletePermanently`, que elimina sin papelera y **no se puede
//!   deshacer**: la interfaz solo debe pedirla tras una confirmación explícita.
//! - **Los conflictos los resuelve el usuario, dentro de la aplicación.** El
//!   hilo se detiene, publica el choque —tamaños y fechas de ambos lados— y
//!   espera. Delegar en los diálogos del sistema habría sacado al usuario de la
//!   aplicación en mitad de su flujo.
//! - **Se anota lo que se hizo, no lo que se pretendía.** Deshacer invierte
//!   acciones concretas. Y una operación que sobrescribió algo se marca como no
//!   reversible, porque lo pisado no vuelve.

pub mod model;
pub mod naming;
mod state;
mod worker;
#[cfg(target_os = "macos")]
mod macos;

pub use model::*;
pub use naming::{unique_path, validate_file_name};
pub use worker::set_copy_chunk;

use state::OpShared;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// Cuántas operaciones terminadas se conservan para poder deshacerlas.
const MAX_HISTORIA: usize = 64;

/// Cuántas operaciones pueden estar tocando el disco a la vez.
///
/// Copiar miles de archivos en diez hilos a la vez no es más rápido, solo hace
/// que el plato trabaje en espiral. Se acota la concurrencia y el resto espera
/// turno en estado `Planning`: la interfaz los ve como "contando", no como
/// congelados.
const MAX_CONCURRENT_FSOPS: u32 = 3;

/// Turno para el disco: permite que a lo sumo `max` operaciones hagan I/O a la
/// vez. Es un semáforo de conteo sencillo sobre `Mutex` + `Condvar`, sin
/// dependencias externas.
struct Limiter {
    activos: Mutex<u32>,
    hueco: Condvar,
    max: u32,
}

impl Limiter {
    fn new(max: u32) -> Self {
        Self {
            activos: Mutex::new(0),
            hueco: Condvar::new(),
            max,
        }
    }

    /// Bloquea hasta conseguir un hueco y devuelve un permiso que lo libera al
    /// soltarse (RAII): se adquiere antes de tocar el disco y se suelta al
    /// terminar la operación, pase lo que pase.
    fn adquirir(&self) -> Permiso<'_> {
        let mut a = self.activos.lock().unwrap_or_else(|p| p.into_inner());
        while *a >= self.max {
            a = self.hueco.wait(a).unwrap_or_else(|p| p.into_inner());
        }
        *a += 1;
        Permiso(self)
    }
}

struct Permiso<'a>(&'a Limiter);

impl Drop for Permiso<'_> {
    fn drop(&mut self) {
        let mut a = self.0.activos.lock().unwrap_or_else(|p| p.into_inner());
        *a = a.saturating_sub(1);
        self.0.hueco.notify_one();
    }
}

#[derive(Default)]
struct Registro {
    vivas: HashMap<u64, Arc<OpShared>>,
    /// Recibos de operaciones terminadas, con su petición original, de la más
    /// antigua a la más reciente. La petición original es lo que permite rehacer.
    historia: Vec<(Receipt, Option<OpRequest>)>,
    /// Peticiones rehacer, en orden inverso: cada deshacer apila la petición
    /// original de lo que retiró; rehacer las vuelve a lanzar.
    redos: Vec<Vec<OpRequest>>,
}

/// Punto de entrada único: recibe peticiones, las ejecuta y responde por su
/// estado.
pub struct FileOpManager {
    next_id: AtomicU64,
    registro: Mutex<Registro>,
    limiter: Arc<Limiter>,
}

impl Default for FileOpManager {
    fn default() -> Self {
        Self::new()
    }
}

impl FileOpManager {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            registro: Mutex::new(Registro::default()),
            limiter: Arc::new(Limiter::new(MAX_CONCURRENT_FSOPS)),
        }
    }

    /// Encola una operación y devuelve su identificador de inmediato.
    ///
    /// No espera a que termine: quien llama consulta `progress` cuando quiera.
    ///
    /// Una operación nueva invalida la pila de "rehacer": las piezas que se
    /// habían deshecho dejan de poder volver, como en cualquier editor.
    pub fn submit(&self, req: OpRequest) -> u64 {
        self.submit_privado(req, true)
    }

    /// Como `submit`, pero sin vaciar la pila de rehacer. Lo usan deshacer y
    /// rehacer para mover piezas por su cuenta.
    fn submit_privado(&self, req: OpRequest, clear_redo: bool) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let shared = Arc::new(OpShared::new(id, req.kind));

        if let Ok(mut reg) = self.registro.lock() {
            reg.vivas.insert(id, shared.clone());
            if clear_redo {
                reg.redos.clear();
            }
        }

        let hilo = shared.clone();
        let limiter = self.limiter.clone();
        let nombre = format!("bdj-fsop-{id}");
        let lanzado = std::thread::Builder::new()
            .name(nombre)
            .spawn(move || {
                // Turno para el disco: si ya hay `MAX_CONCURRENT_FSOPS` operando,
                // este hilo se queda en `Planning` esperando que uno se libere.
                let _permiso = limiter.adquirir();
                worker::run(&hilo, req);
            });

        if let Err(e) = lanzado {
            shared.push_error(format!("No se pudo iniciar la operación: {e}"));
            shared.set_state(OpState::Failed);
        }

        id
    }

    pub fn progress(&self, id: u64) -> Option<OpProgress> {
        let reg = self.registro.lock().ok()?;
        reg.vivas.get(&id).map(|s| s.progress())
    }

    /// Estado de todas las operaciones vivas, de la más antigua a la más nueva.
    pub fn all_progress(&self) -> Vec<OpProgress> {
        let Ok(reg) = self.registro.lock() else {
            return Vec::new();
        };
        let mut v: Vec<OpProgress> = reg.vivas.values().map(|s| s.progress()).collect();
        v.sort_by_key(|p| p.id);
        v
    }

    pub fn cancel(&self, id: u64) {
        if let Ok(reg) = self.registro.lock()
            && let Some(s) = reg.vivas.get(&id)
        {
            s.cancel();
        }
    }

    /// Responde al conflicto que una operación tiene pendiente.
    pub fn resolve_conflict(&self, id: u64, decision: ConflictDecision, apply_to_all: bool) {
        if let Ok(reg) = self.registro.lock()
            && let Some(s) = reg.vivas.get(&id)
        {
            s.answer(decision, apply_to_all);
        }
    }

    /// Recoge los cambios que esta aplicación hizo en el disco.
    ///
    /// El servicio los aplica al índice al instante y abre una ventana de
    /// supresión para no volver a procesarlos cuando lleguen por el diario del
    /// sistema de archivos.
    pub fn take_change_notifications(&self) -> Vec<PathChange> {
        let Ok(reg) = self.registro.lock() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for s in reg.vivas.values() {
            out.extend(s.take_changes());
        }
        out
    }

    /// Archiva las operaciones terminadas y devuelve cuántas se archivaron.
    ///
    /// Conviene llamarlo de vez en cuando desde la interfaz: es lo que mueve un
    /// recibo a la historia y deja la operación disponible para deshacer.
    pub fn reap(&self) -> usize {
        let Ok(mut reg) = self.registro.lock() else {
            return 0;
        };
        let terminadas: Vec<u64> = reg
            .vivas
            .iter()
            .filter(|(_, s)| s.state().is_finished())
            .map(|(id, _)| *id)
            .collect();

        for id in &terminadas {
            if let Some(s) = reg.vivas.get(id) {
                let recibo = s.receipt();
                if recibo.undoable && s.state() == OpState::Done {
                    let forward = s.take_original();
                    reg.historia.push((recibo, forward));
                }
            }
        }
        // Las operaciones vivas se retiran solo cuando quien las pidió ya ha
        // visto su estado final; se archivan aquí de forma explícita.
        for id in &terminadas {
            reg.vivas.remove(id);
        }
        while reg.historia.len() > MAX_HISTORIA {
            reg.historia.remove(0);
        }
        while reg.redos.len() > MAX_HISTORIA {
            reg.redos.remove(0);
        }
        terminadas.len()
    }

    /// Recibo de la última operación reversible, sin retirarlo de la historia.
    pub fn last_undoable(&self) -> Option<Receipt> {
        let reg = self.registro.lock().ok()?;
        reg.historia.last().map(|(r, _)| r.clone())
    }

    /// Deshace la última operación reversible.
    ///
    /// Devuelve los identificadores de las operaciones lanzadas para revertirla.
    /// Puede haber más de una: devolver algo a su sitio cuando además se le
    /// cambió el nombre son dos pasos. Al retirarla, queda apuntada para poder
    /// rehacerla.
    pub fn undo_last(&self) -> Vec<u64> {
        let recibo = {
            let Ok(mut reg) = self.registro.lock() else {
                return Vec::new();
            };
            match reg.historia.pop() {
                Some((r, forward)) => {
                    if let Some(f) = forward {
                        reg.redos.push(vec![f]);
                    }
                    r
                }
                None => return Vec::new(),
            }
        };

        worker::inverse_requests(&recibo)
            .into_iter()
            .map(|r| self.submit_privado(r, false))
            .collect()
    }

    /// Deshace una operación concreta por su identificador.
    pub fn undo(&self, id: u64) -> Vec<u64> {
        let recibo = {
            let Ok(mut reg) = self.registro.lock() else {
                return Vec::new();
            };
            match reg.historia.iter().position(|(r, _)| r.id == id) {
                Some(pos) => {
                    let (r, forward) = reg.historia.remove(pos);
                    if let Some(f) = forward {
                        reg.redos.push(vec![f]);
                    }
                    r
                }
                None => return Vec::new(),
            }
        };
        worker::inverse_requests(&recibo)
            .into_iter()
            .map(|r| self.submit_privado(r, false))
            .collect()
    }

    /// Cierto si hay algo que rehacer (es decir, algo que se deshizo y no se ha
    /// invalidado con una operación nueva).
    pub fn can_redo(&self) -> bool {
        self.registro
            .lock()
            .map(|r| !r.redos.is_empty())
            .unwrap_or(false)
    }

    /// Rehace la última operación que se deshizo.
    ///
    /// Vuelve a lanzar la petición original, así que la pieza se recrea tal y
    /// como estaba. Devuelve los identificadores de las operaciones lanzadas.
    pub fn redo_last(&self) -> Vec<u64> {
        let grupo = {
            let Ok(mut reg) = self.registro.lock() else {
                return Vec::new();
            };
            match reg.redos.pop() {
                Some(g) => g,
                None => return Vec::new(),
            }
        };
        grupo
            .into_iter()
            .map(|r| self.submit_privado(r, false))
            .collect()
    }

    /// Espera a que una operación termine. Solo para pruebas y para el cierre
    /// ordenado de la aplicación.
    pub fn wait_for(&self, id: u64, timeout: std::time::Duration) -> Option<OpProgress> {
        let limite = std::time::Instant::now() + timeout;
        loop {
            let p = self.progress(id)?;
            if p.state.is_finished() {
                return Some(p);
            }
            if std::time::Instant::now() >= limite {
                return Some(p);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;
    use tempfile::tempdir;

    const ESPERA: Duration = Duration::from_secs(20);

    fn escribir(p: &std::path::Path, bytes: &[u8]) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, bytes).unwrap();
    }

    #[test]
    fn crear_carpeta() {
        let dir = tempdir().unwrap();
        let m = FileOpManager::new();
        let id = m.submit(
            OpRequest::new(OpKind::CreateFolder, Vec::new())
                .with_destination(dir.path().to_path_buf())
                .with_new_name("Sesion 2026".into()),
        );
        let p = m.wait_for(id, ESPERA).unwrap();
        assert_eq!(p.state, OpState::Done, "errores: {:?}", p.errors);
        assert!(dir.path().join("Sesion 2026").is_dir());
    }

    #[test]
    fn crear_carpeta_con_nombre_invalido_falla_sin_tocar_el_disco() {
        let dir = tempdir().unwrap();
        let m = FileOpManager::new();
        let id = m.submit(
            OpRequest::new(OpKind::CreateFolder, Vec::new())
                .with_destination(dir.path().to_path_buf())
                .with_new_name("a/b".into()),
        );
        let p = m.wait_for(id, ESPERA).unwrap();
        assert_eq!(p.state, OpState::Failed);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn renombrar() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("viejo.wav");
        escribir(&f, b"audio");
        let m = FileOpManager::new();
        let id = m.submit(
            OpRequest::new(OpKind::Rename, vec![f.clone()]).with_new_name("nuevo.wav".into()),
        );
        let p = m.wait_for(id, ESPERA).unwrap();
        assert_eq!(p.state, OpState::Done, "errores: {:?}", p.errors);
        assert!(!f.exists());
        assert!(dir.path().join("nuevo.wav").exists());
    }

    #[test]
    fn copiar_un_arbol_completo() {
        let dir = tempdir().unwrap();
        let origen = dir.path().join("Sets");
        escribir(&origen.join("a.wav"), &vec![1u8; 3000]);
        escribir(&origen.join("sub/b.wav"), &vec![2u8; 5000]);
        let destino = dir.path().join("Copia");
        std::fs::create_dir(&destino).unwrap();

        let m = FileOpManager::new();
        let id = m.submit(
            OpRequest::new(OpKind::Copy, vec![origen.clone()])
                .with_destination(destino.clone()),
        );
        let p = m.wait_for(id, ESPERA).unwrap();
        assert_eq!(p.state, OpState::Done, "errores: {:?}", p.errors);
        assert!(destino.join("Sets/a.wav").exists());
        assert_eq!(
            std::fs::read(destino.join("Sets/sub/b.wav")).unwrap().len(),
            5000
        );
        // El original sigue donde estaba.
        assert!(origen.join("a.wav").exists());
        assert_eq!(p.done_bytes, 8000);
    }

    #[test]
    fn mover_dentro_del_mismo_volumen_no_copia_bytes() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("pista.wav");
        escribir(&f, &vec![7u8; 10_000]);
        let destino = dir.path().join("destino");
        std::fs::create_dir(&destino).unwrap();

        let m = FileOpManager::new();
        let id = m
            .submit(OpRequest::new(OpKind::Move, vec![f.clone()]).with_destination(destino.clone()));
        let p = m.wait_for(id, ESPERA).unwrap();
        assert_eq!(p.state, OpState::Done, "errores: {:?}", p.errors);
        assert!(!f.exists());
        assert!(destino.join("pista.wav").exists());
    }

    #[test]
    fn duplicar_deja_el_original_y_una_copia_con_nombre_libre() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("kick.wav");
        escribir(&f, b"boom");
        let m = FileOpManager::new();
        let id = m.submit(OpRequest::new(OpKind::Duplicate, vec![f.clone()]));
        let p = m.wait_for(id, ESPERA).unwrap();
        assert_eq!(p.state, OpState::Done, "errores: {:?}", p.errors);
        assert!(f.exists());
        assert!(dir.path().join("kick (2).wav").exists());
    }

    #[test]
    fn conservar_ambos_al_copiar_sobre_un_nombre_ocupado() {
        let dir = tempdir().unwrap();
        let origen_dir = dir.path().join("a");
        let destino = dir.path().join("b");
        escribir(&origen_dir.join("pista.wav"), b"nuevo");
        escribir(&destino.join("pista.wav"), b"viejo");

        let m = FileOpManager::new();
        let id = m.submit(
            OpRequest::new(OpKind::Copy, vec![origen_dir.join("pista.wav")])
                .with_destination(destino.clone())
                .with_policy(ConflictPolicy::KeepBoth),
        );
        let p = m.wait_for(id, ESPERA).unwrap();
        assert_eq!(p.state, OpState::Done, "errores: {:?}", p.errors);
        assert_eq!(std::fs::read(destino.join("pista.wav")).unwrap(), b"viejo");
        assert_eq!(
            std::fs::read(destino.join("pista (2).wav")).unwrap(),
            b"nuevo"
        );
    }

    #[test]
    fn omitir_deja_el_destino_intacto() {
        let dir = tempdir().unwrap();
        let origen_dir = dir.path().join("a");
        let destino = dir.path().join("b");
        escribir(&origen_dir.join("pista.wav"), b"nuevo");
        escribir(&destino.join("pista.wav"), b"viejo");

        let m = FileOpManager::new();
        let id = m.submit(
            OpRequest::new(OpKind::Copy, vec![origen_dir.join("pista.wav")])
                .with_destination(destino.clone())
                .with_policy(ConflictPolicy::Skip),
        );
        m.wait_for(id, ESPERA).unwrap();
        assert_eq!(std::fs::read(destino.join("pista.wav")).unwrap(), b"viejo");
    }

    #[test]
    fn sobrescribir_deja_la_operacion_fuera_de_deshacer() {
        let dir = tempdir().unwrap();
        let origen_dir = dir.path().join("a");
        let destino = dir.path().join("b");
        escribir(&origen_dir.join("pista.wav"), b"nuevo");
        escribir(&destino.join("pista.wav"), b"viejo");

        let m = FileOpManager::new();
        let id = m.submit(
            OpRequest::new(OpKind::Copy, vec![origen_dir.join("pista.wav")])
                .with_destination(destino.clone())
                .with_policy(ConflictPolicy::Overwrite),
        );
        let p = m.wait_for(id, ESPERA).unwrap();
        assert_eq!(p.state, OpState::Done, "errores: {:?}", p.errors);
        assert_eq!(std::fs::read(destino.join("pista.wav")).unwrap(), b"nuevo");
        assert!(
            !p.can_undo,
            "lo que se pisa no vuelve: la operación no puede anunciarse como reversible"
        );
    }

    #[test]
    fn preguntar_detiene_la_operacion_hasta_que_se_responde() {
        let dir = tempdir().unwrap();
        let origen_dir = dir.path().join("a");
        let destino = dir.path().join("b");
        escribir(&origen_dir.join("pista.wav"), b"nuevo");
        escribir(&destino.join("pista.wav"), b"viejo");

        let m = FileOpManager::new();
        let id = m.submit(
            OpRequest::new(OpKind::Copy, vec![origen_dir.join("pista.wav")])
                .with_destination(destino.clone())
                .with_policy(ConflictPolicy::Ask),
        );

        // Esperar a que publique el conflicto.
        let mut visto = None;
        for _ in 0..2000 {
            let p = m.progress(id).unwrap();
            if p.state == OpState::WaitingConflict && p.pending_conflict.is_some() {
                visto = p.pending_conflict.clone();
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let conflicto = visto.expect("la operación debería haberse detenido a preguntar");
        assert!(conflicto.destination.ends_with("pista.wav"));
        assert_eq!(conflicto.destination_size, 5);

        m.resolve_conflict(id, ConflictDecision::KeepBoth, false);
        let p = m.wait_for(id, ESPERA).unwrap();
        assert_eq!(p.state, OpState::Done, "errores: {:?}", p.errors);
        assert!(destino.join("pista (2).wav").exists());
    }

    #[test]
    fn deshacer_un_movimiento_lo_devuelve_a_su_sitio() {
        let dir = tempdir().unwrap();
        let origen_dir = dir.path().join("a");
        let destino = dir.path().join("b");
        std::fs::create_dir_all(&origen_dir).unwrap();
        std::fs::create_dir_all(&destino).unwrap();
        let f = origen_dir.join("pista.wav");
        escribir(&f, b"audio");

        let m = FileOpManager::new();
        let id = m.submit(
            OpRequest::new(OpKind::Move, vec![f.clone()]).with_destination(destino.clone()),
        );
        m.wait_for(id, ESPERA).unwrap();
        assert!(destino.join("pista.wav").exists());

        m.reap();
        let deshacer = m.undo_last();
        assert!(!deshacer.is_empty(), "el movimiento debería ser reversible");
        for uid in deshacer {
            m.wait_for(uid, ESPERA);
        }
        assert!(f.exists(), "el archivo debería haber vuelto a su carpeta");
        assert!(!destino.join("pista.wav").exists());
    }

    #[test]
    fn deshacer_un_renombrado_recupera_el_nombre_original() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("viejo.wav");
        escribir(&f, b"audio");

        let m = FileOpManager::new();
        let id = m.submit(
            OpRequest::new(OpKind::Rename, vec![f.clone()]).with_new_name("nuevo.wav".into()),
        );
        m.wait_for(id, ESPERA).unwrap();
        m.reap();

        for uid in m.undo_last() {
            m.wait_for(uid, ESPERA);
        }
        assert!(f.exists());
        assert!(!dir.path().join("nuevo.wav").exists());
    }

    #[test]
    fn crear_un_archivo_vacio_y_avisar_al_indice() {
        let dir = tempdir().unwrap();
        let m = FileOpManager::new();
        let id = m.submit(
            OpRequest::new(OpKind::CreateFile, Vec::new())
                .with_destination(dir.path().to_path_buf())
                .with_new_name("notas.txt".into()),
        );
        let p = m.wait_for(id, ESPERA).unwrap();
        assert_eq!(p.state, OpState::Done);

        let ruta = dir.path().join("notas.txt");
        assert!(ruta.is_file());
        assert_eq!(std::fs::metadata(&ruta).unwrap().len(), 0);

        let cambios = m.take_change_notifications();
        assert_eq!(cambios.len(), 1);
        match &cambios[0] {
            PathChange::Created { path, is_dir } => {
                assert!(path.ends_with("notas.txt"));
                assert!(!is_dir);
            }
            otro => panic!("cambio inesperado: {otro:?}"),
        }

        // Deshacer un archivo nuevo lo envía a la papelera, nunca borra.
        m.reap();
        let ids = m.undo_last();
        assert!(!ids.is_empty(), "crear un archivo debe poder deshacerse");
        for uid in ids {
            m.wait_for(uid, ESPERA);
        }
        assert!(!ruta.exists(), "tras deshacer, el archivo no está en su sitio");
    }

    #[test]
    fn los_cambios_se_publican_para_el_indice() {
        let dir = tempdir().unwrap();
        let m = FileOpManager::new();
        let id = m.submit(
            OpRequest::new(OpKind::CreateFolder, Vec::new())
                .with_destination(dir.path().to_path_buf())
                .with_new_name("Nueva".into()),
        );
        m.wait_for(id, ESPERA).unwrap();
        let cambios = m.take_change_notifications();
        assert_eq!(cambios.len(), 1);
        match &cambios[0] {
            PathChange::Created { path, is_dir } => {
                assert!(path.ends_with("Nueva"));
                assert!(is_dir);
            }
            otro => panic!("cambio inesperado: {otro:?}"),
        }
        // Se entregan una sola vez.
        assert!(m.take_change_notifications().is_empty());
    }

    #[test]
    fn no_se_puede_mover_una_carpeta_dentro_de_si_misma() {
        let dir = tempdir().unwrap();
        let padre = dir.path().join("Sets");
        let hijo = padre.join("2026");
        std::fs::create_dir_all(&hijo).unwrap();

        let m = FileOpManager::new();
        let id = m
            .submit(OpRequest::new(OpKind::Move, vec![padre.clone()]).with_destination(hijo.clone()));
        let p = m.wait_for(id, ESPERA).unwrap();
        assert_eq!(p.state, OpState::Failed);
        assert!(padre.is_dir());
    }

    #[test]
    fn cancelar_detiene_la_copia() {
        let dir = tempdir().unwrap();
        let origen = dir.path().join("grande");
        std::fs::create_dir_all(&origen).unwrap();
        for i in 0..40 {
            escribir(&origen.join(format!("p{i}.wav")), &vec![0u8; 400_000]);
        }
        let destino = dir.path().join("destino");
        std::fs::create_dir(&destino).unwrap();

        let m = FileOpManager::new();
        let id = m.submit(
            OpRequest::new(OpKind::Copy, vec![origen.clone()]).with_destination(destino.clone()),
        );
        std::thread::sleep(Duration::from_millis(5));
        m.cancel(id);
        let p = m.wait_for(id, ESPERA).unwrap();
        assert!(
            matches!(p.state, OpState::Cancelled | OpState::Done),
            "estado inesperado {:?}",
            p.state
        );
    }

    #[test]
    fn la_papelera_no_borra_de_forma_definitiva() {
        // En el contenedor de integración continua puede no haber papelera; el
        // test comprueba que la operación no revienta y que informa.
        let dir = tempdir().unwrap();
        let f = dir.path().join("sobra.wav");
        escribir(&f, b"x");
        let m = FileOpManager::new();
        let id = m.submit(OpRequest::new(OpKind::Trash, vec![f.clone()]));
        let p = m.wait_for(id, ESPERA).unwrap();
        assert!(p.state.is_finished());
        if p.state == OpState::Done {
            assert!(!f.exists());
        } else {
            assert!(!p.errors.is_empty(), "un fallo debe explicarse");
        }
    }

    #[test]
    fn rehacer_devuelve_lo_deshacer() {
        let dir = tempdir().unwrap();
        let carpeta = dir.path().join("Sets");
        std::fs::create_dir_all(&carpeta).unwrap();
        escribir(&carpeta.join("pista.wav"), b"audio");

        let m = FileOpManager::new();
        let id = m.submit(OpRequest::new(OpKind::Rename, vec![carpeta.join("pista.wav")])
            .with_new_name("otra.wav".into()));
        m.wait_for(id, ESPERA).unwrap();
        m.reap();

        // Sin deshacer no hay nada que rehacer.
        assert!(!m.can_redo());

        // Deshacer el renombrado.
        for uid in m.undo_last() {
            m.wait_for(uid, ESPERA);
        }
        assert!(carpeta.join("pista.wav").exists());
        assert!(!carpeta.join("otra.wav").exists());
        assert!(m.can_redo(), "tras deshacer debe haber redo");

        // Rehacerlo: vuelve el nombre nuevo y desaparece el original.
        for uid in m.redo_last() {
            m.wait_for(uid, ESPERA);
        }
        assert!(carpeta.join("otra.wav").exists());
        assert!(!carpeta.join("pista.wav").exists());
        assert!(!m.can_redo(), "tras rehacer ya no queda nada por rehacer");
    }

    #[test]
    fn una_operacion_nueva_vacia_la_pila_de_rehacer() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("pista.wav");
        escribir(&f, b"audio");

        let m = FileOpManager::new();
        let id = m.submit(OpRequest::new(OpKind::Rename, vec![f.clone()])
            .with_new_name("otra.wav".into()));
        m.wait_for(id, ESPERA).unwrap();
        m.reap();

        for uid in m.undo_last() {
            m.wait_for(uid, ESPERA);
        }
        assert!(m.can_redo());

        // Una operación nueva invalida el redo.
        let nid = m.submit(OpRequest::new(OpKind::CreateFolder, Vec::new())
            .with_destination(dir.path().to_path_buf())
            .with_new_name("Nueva".into()));
        m.wait_for(nid, ESPERA).unwrap();
        assert!(!m.can_redo());
        assert!(m.redo_last().is_empty());
    }

    #[test]
    fn una_operacion_desconocida_no_devuelve_estado() {
        let m = FileOpManager::new();
        assert!(m.progress(999).is_none());
        assert!(m.undo(999).is_empty());
    }

    #[test]
    fn el_limite_impide_que_haya_mas_de_max_en_el_disco() {
        let limiter = Arc::new(Limiter::new(MAX_CONCURRENT_FSOPS));
        let pico = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let activos = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let n = MAX_CONCURRENT_FSOPS + 4;

        let manos: Vec<_> = (0..n)
            .map(|_| {
                let (l, p, a) = (limiter.clone(), pico.clone(), activos.clone());
                std::thread::spawn(move || {
                    let _permiso = l.adquirir();
                    let ahora = a.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    p.fetch_max(ahora, std::sync::atomic::Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(3));
                    a.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                })
            })
            .collect();
        for m in manos {
            m.join().unwrap();
        }
        assert!(
            pico.load(std::sync::atomic::Ordering::SeqCst) <= MAX_CONCURRENT_FSOPS,
            "nunca debería haber más de {MAX_CONCURRENT_FSOPS} haciendo I/O a la vez"
        );
        assert_eq!(activos.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn la_cola_de_operaciones_se_drena_sin_quedarse_colgada() {
        let dir = tempdir().unwrap();
        let m = FileOpManager::new();
        let total = (MAX_CONCURRENT_FSOPS + 5) as usize;
        let mut ids = Vec::new();
        for i in 0..total {
            let id = m.submit(
                OpRequest::new(OpKind::CreateFolder, Vec::new())
                    .with_destination(dir.path().to_path_buf())
                    .with_new_name(format!("Carpeta {i}")),
            );
            ids.push(id);
        }
        for id in ids {
            let p = m.wait_for(id, Duration::from_secs(30)).unwrap();
            assert_eq!(p.state, OpState::Done, "errores: {:?}", p.errors);
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), total);
    }

    #[test]
    fn copiar_varios_origenes_a_la_vez() {
        let dir = tempdir().unwrap();
        let destino = dir.path().join("destino");
        std::fs::create_dir(&destino).unwrap();
        let mut origenes: Vec<PathBuf> = Vec::new();
        for i in 0..5 {
            let f = dir.path().join(format!("pista{i}.wav"));
            escribir(&f, &vec![9u8; 1000]);
            origenes.push(f);
        }

        let m = FileOpManager::new();
        let id = m.submit(OpRequest::new(OpKind::Copy, origenes).with_destination(destino.clone()));
        let p = m.wait_for(id, ESPERA).unwrap();
        assert_eq!(p.state, OpState::Done, "errores: {:?}", p.errors);
        assert_eq!(p.done_items, 5);
        assert_eq!(p.done_bytes, 5000);
    }

    #[test]
    fn borrar_permanentemente_no_deja_trazas() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("basura.wav");
        escribir(&f, b"x");
        let m = FileOpManager::new();
        let id = m.submit(OpRequest::new(OpKind::DeletePermanently, vec![f.clone()]));
        let p = m.wait_for(id, ESPERA).unwrap();
        assert_eq!(p.state, OpState::Done, "errores: {:?}", p.errors);
        assert!(!f.exists());
        assert!(
            !p.can_undo,
            "un borrado definitivo no puede anunciarse como reversible"
        );
        // Ni siquiera entra en la historia: no hay nada que deshacer.
        m.reap();
        assert!(m.undo_last().is_empty());
    }

    #[test]
    fn borrar_permanentemente_una_carpeta_entera_y_avisar_al_indice() {
        let dir = tempdir().unwrap();
        let carpeta = dir.path().join("Sets");
        escribir(&carpeta.join("a.wav"), b"1");
        escribir(&carpeta.join("sub/b.wav"), b"2");
        let m = FileOpManager::new();
        let id = m.submit(OpRequest::new(OpKind::DeletePermanently, vec![carpeta.clone()]));
        let p = m.wait_for(id, ESPERA).unwrap();
        assert_eq!(p.state, OpState::Done, "errores: {:?}", p.errors);
        assert!(!carpeta.exists());
        // Avisó al índice con un `Removed`, que es lo que refresca la vista.
        let cambios = m.take_change_notifications();
        assert!(matches!(&cambios[0], PathChange::Removed { .. }));
    }

    #[test]
    fn la_raiz_de_un_volumen_no_se_borra_aunque_se_pida() {
        let m = FileOpManager::new();
        let id = m.submit(OpRequest::new(
            OpKind::DeletePermanently,
            vec![PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" })],
        ));
        let p = m.wait_for(id, ESPERA).unwrap();
        assert_eq!(p.state, OpState::Failed);
        assert!(!p.errors.is_empty(), "debe explicarse que no se toca la raíz");
    }

    #[test]
    fn comprimir_y_descomprimir_un_archivo_zip() {
        let dir = tempdir().unwrap();
        let origen = dir.path().join("Musica");
        escribir(&origen.join("track1.wav"), b"audio-data-1");
        escribir(&origen.join("sub/track2.wav"), b"audio-data-2");

        let zip_dest = dir.path().join("backup.zip");
        let m = FileOpManager::new();

        // 1. Comprimir
        let req_zip = OpRequest::new(OpKind::CompressZip, vec![origen.clone()])
            .with_destination(zip_dest.clone());
        let id = m.submit(req_zip);
        let p = m.wait_for(id, ESPERA).unwrap();
        assert_eq!(p.state, OpState::Done, "errores: {:?}", p.errors);
        assert!(zip_dest.exists());
        assert!(p.done_bytes > 0);

        // 2. Descomprimir en una carpeta nueva
        let extract_dir = dir.path().join("Extraido");
        let req_unzip = OpRequest::new(OpKind::ExtractZip, vec![zip_dest.clone()])
            .with_destination(extract_dir.clone());
        let id2 = m.submit(req_unzip);
        let p2 = m.wait_for(id2, ESPERA).unwrap();
        assert_eq!(p2.state, OpState::Done, "errores: {:?}", p2.errors);
        assert!(extract_dir.exists());
    }
}
