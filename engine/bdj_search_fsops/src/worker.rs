//! Ejecución de una operación en su propio hilo.
//!
//! Todo lo que aquí se hace se anota en un recibo: no se deshace reconstruyendo
//! la intención, se invierte cada acción que llegó a ejecutarse.

use crate::model::*;
use crate::naming::{is_inside, unique_path};
use crate::state::{OpShared, Resolution};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Tamaño del bloque de copia.
///
/// Un megabyte es el punto donde el progreso sigue siendo fino —una pista de
/// 40 MB da cuarenta actualizaciones— sin que la llamada al sistema domine.
/// Bloque de copia, en bytes.
///
/// En un disco mecánico, bloques de un mega dan un progreso a saltos y el
/// usuario cree que la copia se ha colgado; más pequeños, la barra avanza de
/// forma continua. Además es el punto donde se comprueba la cancelación, así que
/// un bloque más pequeño hace que «Cancelar» responda antes.
///
/// Es un valor **inyectado**, no leído del perfil de la máquina: este crate no
/// sabe nada del índice ni del motor, y hacerle depender del núcleo para leer
/// una constante rompería justo la separación que lo hace probable por su
/// cuenta. Quien compone la aplicación —la capa FFI— lo fija una vez al
/// arrancar; mientras no lo haga, vale un mega, que es lo que valía antes.
static COPY_CHUNK: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(1 << 20);

/// Fija el tamaño del bloque de copia. Se llama una vez, al arrancar.
pub fn set_copy_chunk(bytes: usize) {
    COPY_CHUNK.store(
        bytes.clamp(64 * 1024, 16 * 1024 * 1024),
        std::sync::atomic::Ordering::Relaxed,
    );
}

fn chunk() -> usize {
    COPY_CHUNK.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn run(shared: &OpShared, req: OpRequest) {
    shared.set_state(OpState::Planning);

    // Nada destructivo empieza sin comprobar sobre qué va a actuar.
    //
    // Un usuario llegó a ver «No se pudo enviar C:\ a la papelera»: la
    // aplicación había pedido de verdad que la raíz del disco fuera a la
    // papelera, y quien la salvó fue la biblioteca de papelera negándose por su
    // cuenta, con un error interno que además no significa nada para quien lo
    // lee. Entre la fila que se marca y el archivo que se borra hay una tabla
    // virtualizada, una caché de páginas, dos espacios de identificadores y un
    // índice que se renumera solo; cualquiera de esas piezas puede fallar algún
    // día, y ninguna debería poder fallar hasta el punto de apuntar a un disco
    // entero.
    if matches!(
        req.kind,
        OpKind::Trash | OpKind::Move | OpKind::Rename | OpKind::DeletePermanently
    ) && let Err(motivo) = crate::guards::check_all(&req.sources)
    {
        shared.push_error(motivo);
        shared.set_state(OpState::Failed);
        return;
    }

    // Se guarda la petición original para que rehacer pueda volver a ejecutarla.
    shared.set_original(req.clone());

    let plan = match plan(shared, &req) {
        Ok(p) => p,
        Err(e) => {
            shared.push_error(e);
            shared.set_state(OpState::Failed);
            return;
        }
    };

    shared.set_totals(plan.total_items, plan.total_bytes);
    shared.set_state(OpState::Running);

    let result = match req.kind {
        OpKind::CreateFolder => create_folder(shared, &req),
        OpKind::CreateFile => create_file(shared, &req),
        OpKind::Rename => rename(shared, &req),
        OpKind::Trash => trash(shared, &req),
        OpKind::Restore => restore(shared, &req),
        OpKind::DeletePermanently => delete_permanently(shared, &req),
        OpKind::Copy | OpKind::Duplicate | OpKind::Move => transfer(shared, &req, &plan),
        OpKind::CompressZip => compress_zip(shared, &req),
        OpKind::ExtractZip => extract_zip(shared, &req),
    };

    match result {
        Ok(()) if shared.is_cancelled() => shared.set_state(OpState::Cancelled),
        Ok(()) if shared.has_errors() => shared.set_state(OpState::Failed),
        Ok(()) => shared.set_state(OpState::Done),
        Err(e) => {
            shared.push_error(e);
            shared.set_state(OpState::Failed);
        }
    }
}

struct Plan {
    total_items: u64,
    total_bytes: u64,
}

/// Cuenta archivos y bytes antes de empezar, para que la barra de progreso
/// signifique algo desde el primer instante.
fn plan(shared: &OpShared, req: &OpRequest) -> Result<Plan, String> {
    match req.kind {
        OpKind::CreateFolder | OpKind::CreateFile | OpKind::Rename => Ok(Plan {
            total_items: 1,
            total_bytes: 0,
        }),
        OpKind::Trash => Ok(Plan {
            total_items: req.sources.len() as u64,
            total_bytes: 0,
        }),
        OpKind::Restore => Ok(Plan {
            total_items: req.sources.len() as u64,
            total_bytes: 0,
        }),
OpKind::Copy | OpKind::Duplicate | OpKind::Move => {
            let mut items = 0u64;
            let mut bytes = 0u64;
            for src in &req.sources {
                if shared.is_cancelled() {
                    break;
                }
                count_tree(src, &mut items, &mut bytes);
            }
            Ok(Plan {
                total_items: items,
                total_bytes: bytes,
            })
        }
        OpKind::DeletePermanently => Ok(Plan {
            total_items: req.sources.len() as u64,
            total_bytes: 0,
        }),
        OpKind::CompressZip => {
            let mut items = 0u64;
            let mut bytes = 0u64;
            for src in &req.sources {
                if shared.is_cancelled() {
                    break;
                }
                count_tree(Path::new(src), &mut items, &mut bytes);
            }
            Ok(Plan {
                total_items: items,
                total_bytes: bytes,
            })
        }
        OpKind::ExtractZip => {
            let mut items = 0u64;
            let mut bytes = 0u64;
            for src in &req.sources {
                if let Ok(file) = std::fs::File::open(src)
                    && let Ok(mut archive) = zip::ZipArchive::new(file)
                {
                    items += archive.len() as u64;
                    for i in 0..archive.len() {
                        if let Ok(f) = archive.by_index(i) {
                            bytes += f.size();
                        }
                    }
                }
            }
            Ok(Plan {
                total_items: items,
                total_bytes: bytes,
            })
        }
    }
}

fn count_tree(path: &Path, items: &mut u64, bytes: &mut u64) {
    let Ok(md) = fs::symlink_metadata(path) else {
        return;
    };
    *items += 1;
    if md.is_file() {
        *bytes += md.len();
        return;
    }
    if !md.is_dir() {
        return;
    }
    let Ok(rd) = fs::read_dir(path) else {
        return;
    };
    for entry in rd.flatten() {
        count_tree(&entry.path(), items, bytes);
    }
}

fn create_folder(shared: &OpShared, req: &OpRequest) -> Result<(), String> {
    let parent = req
        .destination
        .clone()
        .ok_or("Falta la carpeta contenedora.")?;
    let name = req.new_name.clone().ok_or("Falta el nombre.")?;
    crate::naming::validate_file_name(&name)?;

    let mut target = parent.join(&name);
    if target.exists() {
        match resolve(shared, req, &target, &target)? {
            Resolution::Skip => return Ok(()),
            Resolution::KeepBoth => {
                target = unique_path(&target).ok_or("No hay un nombre libre disponible.")?;
            }
            Resolution::Overwrite => {
                return Err(format!(
                    "«{}» ya existe. Una carpeta no se sobrescribe.",
                    target.display()
                ));
            }
            Resolution::Cancel => {
                shared.cancel();
                return Ok(());
            }
        }
    }

    fs::create_dir(&target).map_err(|e| format!("No se pudo crear la carpeta: {e}"))?;
    shared.record(Action::Created {
        path: target.to_string_lossy().to_string(),
        is_dir: true,
    });
    shared.notify(PathChange::Created {
        path: target.to_string_lossy().to_string(),
        is_dir: true,
    });
    shared.advance_item(&target);
    Ok(())
}

fn create_file(shared: &OpShared, req: &OpRequest) -> Result<(), String> {
    let parent = req
        .destination
        .clone()
        .ok_or("Falta la carpeta contenedora.")?;
    let name = req.new_name.clone().ok_or("Falta el nombre.")?;
    crate::naming::validate_file_name(&name)?;

    let mut target = parent.join(&name);
    if target.exists() {
        match resolve(shared, req, &target, &target)? {
            Resolution::Skip => return Ok(()),
            Resolution::KeepBoth => {
                target = unique_path(&target).ok_or("No hay un nombre libre disponible.")?;
            }
            Resolution::Overwrite => {
                shared.record(Action::Overwrote {
                    path: target.to_string_lossy().to_string(),
                });
                shared.mark_not_undoable();
            }
            Resolution::Cancel => {
                shared.cancel();
                return Ok(());
            }
        }
    }

    fs::write(&target, b"")
        .map_err(|e| format!("No se pudo crear el archivo: {e}"))?;
    shared.record(Action::Created {
        path: target.to_string_lossy().to_string(),
        is_dir: false,
    });
    shared.notify(PathChange::Created {
        path: target.to_string_lossy().to_string(),
        is_dir: false,
    });
    shared.advance_item(&target);
    Ok(())
}

fn rename(shared: &OpShared, req: &OpRequest) -> Result<(), String> {
    let source = req.sources.first().ok_or("Falta el archivo a renombrar.")?;
    let name = req.new_name.clone().ok_or("Falta el nombre nuevo.")?;
    crate::naming::validate_file_name(&name)?;

    let parent = source
        .parent()
        .ok_or("El origen no tiene carpeta contenedora.")?;
    let mut target = parent.join(&name);

    if target == *source {
        return Ok(());
    }

    // En Windows y macOS el sistema de archivos no distingue mayúsculas: pasar
    // de «pista.wav» a «Pista.wav» es un renombrado legítimo, no un conflicto.
    let solo_cambia_caja = target
        .to_string_lossy()
        .eq_ignore_ascii_case(&source.to_string_lossy());

    if target.exists() && !solo_cambia_caja {
        match resolve(shared, req, source, &target)? {
            Resolution::Skip => return Ok(()),
            Resolution::KeepBoth => {
                target = unique_path(&target).ok_or("No hay un nombre libre disponible.")?;
            }
            Resolution::Overwrite => {
                shared.record(Action::Overwrote {
                    path: target.to_string_lossy().to_string(),
                });
                shared.mark_not_undoable();
            }
            Resolution::Cancel => {
                shared.cancel();
                return Ok(());
            }
        }
    }

    fs::rename(source, &target).map_err(|e| format!("No se pudo renombrar: {e}"))?;
    shared.record(Action::Moved {
        from: source.to_string_lossy().to_string(),
        to: target.to_string_lossy().to_string(),
    });
    shared.notify(PathChange::Renamed {
        from: source.to_string_lossy().to_string(),
        to: target.to_string_lossy().to_string(),
    });
    shared.advance_item(&target);
    Ok(())
}

/// Envía una ruta a la papelera. En macOS se usa la implementación propia
/// (`macos.rs`): la crate `trash` solo sabe borrar ahí, no restaurar, y para
/// que la restauración funcione hay que llevar la ruta original.
fn do_trash(source: &Path) -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        trash::delete(source).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos::trash_path(source).map(|_| ())
    }
}

fn trash(shared: &OpShared, req: &OpRequest) -> Result<(), String> {
    for source in &req.sources {
        if shared.is_cancelled() {
            break;
        }
        shared.set_current(source);
        match do_trash(source) {
            Ok(()) => {
                shared.record(Action::Trashed {
                    original: source.to_string_lossy().to_string(),
                });
                shared.notify(PathChange::Removed {
                    path: source.to_string_lossy().to_string(),
                });
            }
            Err(e) => shared.push_error(format!(
                "No se pudo enviar «{}» a la papelera: {e}",
                source.display()
            )),
        }
        shared.advance_item(source);
    }
    Ok(())
}

/// Borra del disco de forma definitiva, sin pasar por la papelera.
///
/// Es la única operación que **nunca** entra en la historia de deshacer: no hay
/// papelera, no hay red de seguridad. Se marca como no reversible al empezar, no
/// al terminar, para que ni siquiera por un instante parezca que se puede
/// deshacer.
fn delete_permanently(shared: &OpShared, req: &OpRequest) -> Result<(), String> {
    shared.mark_not_undoable();
    for source in &req.sources {
        if shared.is_cancelled() {
            break;
        }
        shared.set_current(source);

        // La raíz de un volumen no se borra: no hay carpeta contenedora y el
        // resultado sería perder todo el disco. Es una red de seguridad ante
        // rutas sueltas que llegaran por la interfaz.
        if source.parent().is_none() {
            shared.push_error(format!(
                "«{}» parece la raíz de un volumen y no se borra.",
                source.display()
            ));
            shared.advance_item(source);
            continue;
        }

        match delete_path(source) {
            Ok(()) => {
                shared.record(Action::Deleted {
                    path: source.to_string_lossy().to_string(),
                });
                shared.notify(PathChange::Removed {
                    path: source.to_string_lossy().to_string(),
                });
            }
            Err(e) => shared.push_error(format!(
                "No se pudo eliminar «{}»: {e}",
                source.display()
            )),
        }
        shared.advance_item(source);
    }
    Ok(())
}

/// Archivo a `fs::remove_file`, carpeta (con su árbol) a `fs::remove_dir_all`.
fn delete_path(path: &Path) -> Result<(), String> {
    let md = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if md.is_dir() {
        fs::remove_dir_all(path).map_err(|e| e.to_string())
    } else {
        fs::remove_file(path).map_err(|e| e.to_string())
    }
}

/// Restaura elementos de la papelera a su ubicación original.
///
/// En macOS la crate `trash` no expone listar ni restaurar, así que la
/// restauración es propia: se localiza el elemento —por el atributo extendido
/// que graba `trash_path` o por nombre— y se le devuelve al moverlo en el mismo
/// volumen. En Windows y Linux se delega en `trash::os_limited`.
fn restore(shared: &OpShared, req: &OpRequest) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        for source in &req.sources {
            if shared.is_cancelled() {
                break;
            }
            shared.set_current(source);

            let trashed = match crate::macos::find_trashed_item(source) {
                Some(t) => t,
                None => {
                    shared.push_error(format!(
                        "No hay ningún elemento de «{}» en la papelera.",
                        source.display()
                    ));
                    shared.advance_item(source);
                    continue;
                }
            };

            // La carpeta original puede haber desaparecido; restaurar la recrea.
            if let Some(parent) = source.parent()
                && !parent.as_os_str().is_empty()
                && let Err(e) = std::fs::create_dir_all(parent)
            {
                shared.push_error(format!(
                    "No se pudo recrear la carpeta de «{}»: {e}",
                    source.display()
                ));
                shared.advance_item(source);
                continue;
            }

            match std::fs::rename(&trashed, source) {
                Ok(()) => {
                    shared.record(Action::Restored {
                        path: source.to_string_lossy().to_string(),
                    });
                    shared.notify(PathChange::Created {
                        path: source.to_string_lossy().to_string(),
                        is_dir: source.is_dir(),
                    });
                }
                Err(e) => shared.push_error(format!(
                    "No se pudo restaurar «{}»: {e}",
                    source.display()
                )),
            }
            shared.advance_item(source);
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        for source in &req.sources {
            if shared.is_cancelled() {
                break;
            }
            shared.set_current(source);

            let origen = match trash::os_limited::list() {
                Ok(items) => {
                    // Un mismo elemento puede haberse enviado a la papelera
                    // varias veces. Se restaura el más reciente (time_deleted
                    // mayor).
                    let mut candidatos: Vec<&trash::TrashItem> = items
                        .iter()
                        .filter(|i| i.original_path() == *source)
                        .collect();
                    candidatos.sort_by_key(|i| i.time_deleted);
                    match candidatos.last() {
                        Some(i) => (*i).clone(),
                        None => {
                            shared.push_error(format!(
                                "No hay ningún elemento de «{}» en la papelera.",
                                source.display()
                            ));
                            shared.advance_item(source);
                            continue;
                        }
                    }
                }
                Err(e) => {
                    shared.push_error(format!(
                        "No se pudo consultar la papelera para «{}»: {e}",
                        source.display()
                    ));
                    shared.advance_item(source);
                    continue;
                }
            };

            match trash::os_limited::restore_all(std::iter::once(origen)) {
                Ok(()) => {
                    shared.record(Action::Restored {
                        path: source.to_string_lossy().to_string(),
                    });
                    shared.notify(PathChange::Created {
                        path: source.to_string_lossy().to_string(),
                        is_dir: source.is_dir(),
                    });
                }
                Err(e) => shared.push_error(format!(
                    "No se pudo restaurar «{}»: {e}",
                    source.display()
                )),
            }
            shared.advance_item(source);
        }
        Ok(())
    }
}

/// Copiar, duplicar y mover comparten toda la maquinaria: solo cambia el
/// destino y si el original se conserva.
fn transfer(shared: &OpShared, req: &OpRequest, _plan: &Plan) -> Result<(), String> {
    let dest_dir = match req.kind {
        OpKind::Duplicate => None,
        _ => Some(
            req.destination
                .clone()
                .ok_or("Falta la carpeta de destino.")?,
        ),
    };

    if let Some(dir) = &dest_dir {
        if !dir.is_dir() {
            return Err(format!("«{}» no es una carpeta.", dir.display()));
        }
        for src in &req.sources {
            if src.is_dir() && is_inside(dir, src) {
                return Err(format!(
                    "No se puede mover «{}» dentro de sí misma.",
                    src.display()
                ));
            }
        }
    }

    for source in &req.sources {
        if shared.is_cancelled() {
            break;
        }

        let file_name = match source.file_name() {
            Some(n) => n.to_os_string(),
            None => {
                shared.push_error(format!("Ruta inválida: {}", source.display()));
                continue;
            }
        };

        let mut target = match &dest_dir {
            Some(dir) => dir.join(&file_name),
            None => {
                // Duplicar: junto al original, con nombre libre.
                match unique_path(source) {
                    Some(p) => p,
                    None => {
                        shared.push_error(format!(
                            "No hay un nombre libre para duplicar «{}».",
                            source.display()
                        ));
                        continue;
                    }
                }
            }
        };

        if target == *source {
            continue;
        }

        if target.exists() && req.kind != OpKind::Duplicate {
            match resolve(shared, req, source, &target)? {
                Resolution::Skip => continue,
                Resolution::KeepBoth => match unique_path(&target) {
                    Some(p) => target = p,
                    None => {
                        shared.push_error(format!(
                            "No hay un nombre libre en el destino para «{}».",
                            source.display()
                        ));
                        continue;
                    }
                },
                Resolution::Overwrite => {
                    shared.record(Action::Overwrote {
                        path: target.to_string_lossy().to_string(),
                    });
                    shared.mark_not_undoable();
                    if target.is_dir() && !source.is_dir() {
                        shared.push_error(format!(
                            "«{}» es una carpeta y el origen no lo es.",
                            target.display()
                        ));
                        continue;
                    }
                    if target.is_file()
                        && let Err(e) = fs::remove_file(&target)
                    {
                        shared.push_error(format!(
                            "No se pudo sustituir «{}»: {e}",
                            target.display()
                        ));
                        continue;
                    }
                }
                Resolution::Cancel => {
                    shared.cancel();
                    break;
                }
            }
        }

        let moving = req.kind == OpKind::Move;

        if moving {
            // Dentro del mismo volumen, renombrar es instantáneo y no copia ni
            // un byte. Solo si el sistema dice que no —volúmenes distintos— se
            // cae a copiar y borrar.
            match fs::rename(source, &target) {
                Ok(()) => {
                    shared.record(Action::Moved {
                        from: source.to_string_lossy().to_string(),
                        to: target.to_string_lossy().to_string(),
                    });
                    shared.notify(PathChange::Renamed {
                        from: source.to_string_lossy().to_string(),
                        to: target.to_string_lossy().to_string(),
                    });
                    // El plan contó el árbol entero; al renombrar se saldan
                    // todos sus elementos de golpe.
                    let (mut items, mut bytes) = (0, 0);
                    count_tree(&target, &mut items, &mut bytes);
                    shared.advance_bulk(items, bytes, &target);
                    continue;
                }
                Err(_) => { /* volúmenes distintos: copiar y borrar */ }
            }
        }

        if let Err(e) = copy_tree(shared, source, &target) {
            shared.push_error(e);
            continue;
        }
        shared.record(Action::Copied {
            to: target.to_string_lossy().to_string(),
        });
        shared.notify(PathChange::Created {
            path: target.to_string_lossy().to_string(),
            is_dir: target.is_dir(),
        });

        if moving && !shared.is_cancelled() {
// El original solo se retira **después** de que la copia esté
            // completa, y a la papelera: si algo salió mal, el archivo sigue
            // estando en alguna parte.
            match do_trash(source) {
                Ok(()) => {
                    shared.record(Action::Trashed {
                        original: source.to_string_lossy().to_string(),
                    });
                    shared.notify(PathChange::Removed {
                        path: source.to_string_lossy().to_string(),
                    });
                }
                Err(e) => shared.push_error(format!(
                    "Se copió «{}» pero no se pudo retirar el original: {e}",
                    source.display()
                )),
            }
        }
    }

    Ok(())
}

fn copy_tree(shared: &OpShared, source: &Path, target: &Path) -> Result<(), String> {
    if shared.is_cancelled() {
        return Ok(());
    }
    let md = fs::symlink_metadata(source)
        .map_err(|e| format!("No se pudo leer «{}»: {e}", source.display()))?;

    if md.is_dir() {
        fs::create_dir_all(target)
            .map_err(|e| format!("No se pudo crear «{}»: {e}", target.display()))?;
        shared.advance_item(target);
        let rd = fs::read_dir(source)
            .map_err(|e| format!("No se pudo abrir «{}»: {e}", source.display()))?;
        for entry in rd.flatten() {
            if shared.is_cancelled() {
                return Ok(());
            }
            let child_target = target.join(entry.file_name());
            if let Err(e) = copy_tree(shared, &entry.path(), &child_target) {
                shared.push_error(e);
            }
        }
        return Ok(());
    }

    copy_file(shared, source, target)
}

/// Copia un archivo por bloques, informando del avance y atendiendo la
/// cancelación entre bloque y bloque.
///
/// `std::fs::copy` sería una línea, pero no se puede cancelar ni informa de
/// nada: copiar una carpeta de sesiones de 4 GB dejaría la barra congelada y el
/// botón de cancelar sin efecto hasta el final.
fn copy_file(shared: &OpShared, source: &Path, target: &Path) -> Result<(), String> {
    shared.set_current(source);

    let mut input = fs::File::open(source)
        .map_err(|e| format!("No se pudo abrir «{}»: {e}", source.display()))?;
    let mut output = fs::File::create(target)
        .map_err(|e| format!("No se pudo crear «{}»: {e}", target.display()))?;

    let mut buf = vec![0u8; chunk()];
    loop {
        if shared.is_cancelled() {
            drop(output);
            // Una copia a medias no se deja en el disco.
            let _ = fs::remove_file(target);
            return Ok(());
        }
        let read = input
            .read(&mut buf)
            .map_err(|e| format!("Error leyendo «{}»: {e}", source.display()))?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buf[..read])
            .map_err(|e| format!("Error escribiendo «{}»: {e}", target.display()))?;
        shared.advance_bytes(read as u64);
    }

    output
        .flush()
        .map_err(|e| format!("Error cerrando «{}»: {e}", target.display()))?;
    drop(output);

    // Conservar la fecha de modificación importa: para un DJ, el orden por
    // fecha es cómo encuentra lo último que importó.
    if let Ok(md) = fs::metadata(source)
        && let Ok(mtime) = md.modified()
        && let Ok(f) = fs::File::options().write(true).open(target)
    {
        let _ = f.set_modified(mtime);
    }

    shared.advance_item(target);
    Ok(())
}

/// Pregunta —o aplica la política ya elegida— ante un destino ocupado.
fn resolve(
    shared: &OpShared,
    req: &OpRequest,
    source: &Path,
    target: &Path,
) -> Result<Resolution, String> {
    match req.conflict {
        ConflictPolicy::KeepBoth => return Ok(Resolution::KeepBoth),
        ConflictPolicy::Skip => return Ok(Resolution::Skip),
        ConflictPolicy::Overwrite => return Ok(Resolution::Overwrite),
        ConflictPolicy::Ask => {}
    }

    if let Some(previa) = shared.sticky_resolution() {
        return Ok(previa);
    }

    let conflicto = describe_conflict(source, target);
    Ok(shared.ask(conflicto))
}

fn describe_conflict(source: &Path, target: &Path) -> Conflict {
    let leer = |p: &Path| -> (u64, u32, bool) {
        match fs::metadata(p) {
            Ok(md) => {
                let mtime = md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as u32)
                    .unwrap_or(0);
                (md.len(), mtime, md.is_dir())
            }
            Err(_) => (0, 0, false),
        }
    };
    let (s_size, s_mtime, _) = leer(source);
    let (d_size, d_mtime, d_dir) = leer(target);
    Conflict {
        source: source.to_string_lossy().to_string(),
        destination: target.to_string_lossy().to_string(),
        source_size: s_size,
        destination_size: d_size,
        source_mtime: s_mtime,
        destination_mtime: d_mtime,
        destination_is_dir: d_dir,
    }
}

/// Construye la petición que deshace un recibo, invirtiendo cada acción en
/// orden contrario al que se ejecutaron.
pub fn inverse_requests(receipt: &Receipt) -> Vec<OpRequest> {
    if !receipt.undoable {
        return Vec::new();
    }
    let mut out = Vec::new();
    for action in receipt.actions.iter().rev() {
        match action {
            Action::Moved { from, to } => {
                // Devolver algo a su sitio puede exigir dos pasos: llevarlo a la
                // carpeta de origen y, si además se le cambió el nombre,
                // devolvérselo. Un renombrado a secas solo cubre el segundo.
                let origen = PathBuf::from(from);
                let actual = PathBuf::from(to);
                let (Some(origen_dir), Some(origen_name)) =
                    (origen.parent(), origen.file_name().map(|n| n.to_os_string()))
                else {
                    continue;
                };
                let (Some(actual_dir), Some(actual_name)) =
                    (actual.parent(), actual.file_name().map(|n| n.to_os_string()))
                else {
                    continue;
                };

                let mut ruta_intermedia = actual.clone();
                if origen_dir != actual_dir {
                    out.push(
                        OpRequest::new(OpKind::Move, vec![actual.clone()])
                            .with_destination(origen_dir.to_path_buf())
                            .with_policy(ConflictPolicy::Skip),
                    );
                    ruta_intermedia = origen_dir.join(&actual_name);
                }
                if origen_name != actual_name {
                    out.push(
                        OpRequest::new(OpKind::Rename, vec![ruta_intermedia])
                            .with_new_name(origen_name.to_string_lossy().to_string())
                            .with_policy(ConflictPolicy::Skip),
                    );
                }
            }
            Action::Created { path, .. } | Action::Copied { to: path } => {
                out.push(OpRequest::new(OpKind::Trash, vec![PathBuf::from(path)]));
            }
            Action::Trashed { original } => {
                // Deshacer un envío a la papelera es restaurar el elemento a su
                // ubicación original. Se evita una cascada: si el destino sigue
                // ocupado, la operación falla y se informa.
                out.push(
                    OpRequest::new(OpKind::Restore, vec![PathBuf::from(original)])
                        .with_policy(ConflictPolicy::Skip),
                );
            }
Action::Restored { path } => {
                out.push(OpRequest::new(OpKind::Trash, vec![PathBuf::from(path)]));
            }
            Action::Overwrote { .. } => {}
            Action::Deleted { .. } => {}
        }
    }
    out
}

fn compress_zip(shared: &OpShared, req: &OpRequest) -> Result<(), String> {
    shared.mark_not_undoable();
    let dest_buf = req
        .destination
        .as_ref()
        .ok_or_else(|| "Falta la ruta destino del archivo zip".to_string())?;
    let dest_path = dest_buf.as_path();

    let file = fs::File::create(dest_path)
        .map_err(|e| format!("No se pudo crear el archivo zip: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut buf = vec![0u8; chunk()];

    for src_buf in &req.sources {
        if shared.is_cancelled() {
            break;
        }
        let src = src_buf.as_path();
        if !src.exists() {
            continue;
        }

        let base_dir = src.parent().unwrap_or_else(|| Path::new(""));

        let mut stack = vec![src.to_path_buf()];
        while let Some(current) = stack.pop() {
            if shared.is_cancelled() {
                break;
            }

            let rel_name = current
                .strip_prefix(base_dir)
                .unwrap_or(&current)
                .to_string_lossy()
                .replace('\\', "/");

            if current.is_dir() {
                let name = if rel_name.ends_with('/') {
                    rel_name
                } else {
                    format!("{}/", rel_name)
                };
                let _ = zip.add_directory(&name, options);
                if let Ok(entries) = fs::read_dir(&current) {
                    for entry in entries.flatten() {
                        stack.push(entry.path());
                    }
                }
            } else {
                if let Ok(mut f) = fs::File::open(&current)
                    && zip.start_file(&rel_name, options).is_ok()
                {
                    loop {
                        if shared.is_cancelled() {
                            break;
                        }
                        match f.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                if zip.write_all(&buf[..n]).is_err() {
                                    break;
                                }
                                shared.advance_bytes(n as u64);
                            }
                            Err(_) => break,
                        }
                    }
                }
                shared.advance_item(&current);
            }
        }
    }

    zip.finish().map_err(|e| format!("Error al cerrar el archivo zip: {e}"))?;

    shared.notify(PathChange::Created {
        path: dest_path.to_string_lossy().to_string(),
        is_dir: false,
    });

    Ok(())
}

fn extract_zip(shared: &OpShared, req: &OpRequest) -> Result<(), String> {
    shared.mark_not_undoable();
    let dest_buf = req
        .destination
        .as_ref()
        .ok_or_else(|| "Falta la carpeta destino para descomprimir".to_string())?;
    let dest_path = dest_buf.as_path();
    if !dest_path.exists() {
        fs::create_dir_all(dest_path)
            .map_err(|e| format!("No se pudo crear el directorio de destino: {e}"))?;
    }

    let mut buf = vec![0u8; chunk()];

    for src_buf in &req.sources {
        if shared.is_cancelled() {
            break;
        }
        let src = src_buf.as_path();
        let file = fs::File::open(src)
            .map_err(|e| format!("No se pudo abrir el archivo zip {}: {e}", src.display()))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| format!("El archivo {} no es un ZIP válido: {e}", src.display()))?;

        for i in 0..archive.len() {
            if shared.is_cancelled() {
                break;
            }
            let mut entry = match archive.by_index(i) {
                Ok(e) => e,
                Err(_) => continue,
            };

            let outpath = match entry.enclosed_name() {
                Some(p) => dest_path.join(p),
                None => continue,
            };

            if entry.is_dir() {
                let _ = fs::create_dir_all(&outpath);
                shared.notify(PathChange::Created {
                    path: outpath.to_string_lossy().to_string(),
                    is_dir: true,
                });
            } else {
                if let Some(parent) = outpath.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if let Ok(mut outfile) = fs::File::create(&outpath) {
                    loop {
                        if shared.is_cancelled() {
                            break;
                        }
                        match entry.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                if outfile.write_all(&buf[..n]).is_err() {
                                    break;
                                }
                                shared.advance_bytes(n as u64);
                            }
                            Err(_) => break,
                        }
                    }
                }
                shared.advance_item(&outpath);
                shared.notify(PathChange::Created {
                    path: outpath.to_string_lossy().to_string(),
                    is_dir: false,
                });
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deshacer_un_envio_a_la_papelera_lanza_una_restauracion() {
        let recibo = Receipt {
            id: 1,
            kind: OpKind::Trash,
            undoable: true,
            actions: vec![Action::Trashed {
                original: "C:\\Musica\\pista.wav".to_string(),
            }],
        };
        let peticiones = inverse_requests(&recibo);
        assert_eq!(peticiones.len(), 1);
        assert_eq!(peticiones[0].kind, OpKind::Restore);
        assert_eq!(
            peticiones[0].sources,
            vec![std::path::PathBuf::from("C:\\Musica\\pista.wav")]
        );
    }

    #[test]
    fn deshacer_una_restauracion_devuelve_el_elemento_a_la_papelera() {
        let recibo = Receipt {
            id: 2,
            kind: OpKind::Restore,
            undoable: true,
            actions: vec![Action::Restored {
                path: "C:\\Musica\\pista.wav".to_string(),
            }],
        };
        let peticiones = inverse_requests(&recibo);
        assert_eq!(peticiones.len(), 1);
        assert_eq!(peticiones[0].kind, OpKind::Trash);
    }

    #[test]
    fn una_operacion_no_reversible_no_devuelve_nada() {
        let recibo = Receipt {
            id: 3,
            kind: OpKind::Copy,
            undoable: false,
            actions: vec![Action::Overwrote {
                path: "C:\\x.wav".to_string(),
            }],
        };
        assert!(inverse_requests(&recibo).is_empty());
    }
}
