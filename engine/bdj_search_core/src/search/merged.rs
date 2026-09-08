//! Búsqueda sobre el índice base **más** la capa de cambios recién publicada.
//!
//! # Por qué hace falta
//!
//! El índice base solo se reescribe al compactar, y compactar diez millones de
//! entradas cuesta unos veinticinco segundos. Si la aplicación consulta
//! únicamente ese archivo, un archivo copiado hace un segundo no existe para
//! ella. Aquí se consulta también la capa publicada —un archivo diminuto que el
//! servicio reescribe cada pocos cientos de milisegundos— y se funden los dos
//! resultados.
//!
//! # Espacio de identificadores
//!
//! El de siempre: por debajo de `base_count` la entrada vive en el índice base;
//! por encima, en la capa, y su posición dentro de la vista incrustada es
//! `id - base_count`. Quien reciba estos identificadores tiene que resolverlos
//! con la misma regla.
//!
//! # Por qué la fusión compara valores de verdad y no claves enteras
//!
//! El motor ordena por claves enteras precalculadas —`name_rank` es la posición
//! alfabética de cada entrada— y eso es lo que hace que ordenar cueste lo que el
//! resultado. Pero ese rango **es relativo a su propio índice**: el `name_rank`
//! de la capa ordena las cuatro entradas de la capa entre sí, y no significa
//! nada comparado con el del base.
//!
//! Compararlos daría un orden plausible y equivocado, del tipo que nadie nota
//! hasta que un archivo aparece en mitad de la lista. Así que en la frontera —y
//! solo en la frontera— se comparan los valores reales: el nombre, el tamaño, la
//! fecha. Sale barato porque solo se hace sobre la ventana visible más las
//! entradas de la capa, que son pocas por construcción: en cuanto pasan del 5 %
//! del índice, el servicio compacta.

use crate::index::overlay::OverlayIndex;
use crate::index::overlay_file::OverlaySnapshot;
use crate::index::view::IndexView;
use crate::query::{CompiledQuery, MatchScratch, Parser};
use std::cmp::Ordering;

use super::engine::{Engine, SearchResult};

/// Clave comparable de una fila, ya extraída de la vista que corresponda.
///
/// Se materializa una vez por fila candidata en lugar de leerse dentro del
/// comparador: un comparador que va a buscar el dato a dos vistas distintas hace
/// un salto de memoria por comparación.
#[derive(PartialEq, Eq)]
enum Clave {
    Texto(String),
    Numero(u64),
}

impl Clave {
    fn cmp_con(&self, otra: &Clave) -> Ordering {
        match (self, otra) {
            (Clave::Texto(a), Clave::Texto(b)) => a.cmp(b),
            (Clave::Numero(a), Clave::Numero(b)) => a.cmp(b),
            // No puede pasar: las dos filas se extraen con la misma columna.
            _ => Ordering::Equal,
        }
    }
}

struct Candidata {
    id: u32,
    es_dir: bool,
    clave: Clave,
    /// Desempate estable: a igualdad de clave, orden alfabético por nombre.
    nombre: String,
}

/// Orden de las carpetas y los grandes grupos de tipo, igual que en el motor.
fn rango_de_tipo(es_dir: bool, ext: &str) -> u64 {
    if es_dir {
        return 0;
    }
    const AUDIO: [&str; 10] = [
        "wav", "mp3", "flac", "aiff", "aif", "ogg", "m4a", "wma", "aac", "opus",
    ];
    const PROYECTO: [&str; 6] = ["als", "flp", "logicx", "ptx", "cpr", "rpp"];
    const VIDEO: [&str; 6] = ["mp4", "mov", "avi", "mkv", "wmv", "webm"];
    const IMAGEN: [&str; 6] = ["jpg", "jpeg", "png", "gif", "bmp", "webp"];
    let e = ext.to_ascii_lowercase();
    if AUDIO.contains(&e.as_str()) {
        1
    } else if PROYECTO.contains(&e.as_str()) {
        2
    } else if VIDEO.contains(&e.as_str()) {
        3
    } else if IMAGEN.contains(&e.as_str()) {
        4
    } else {
        5
    }
}

/// Extrae de una vista la clave de ordenación y el nombre de una entrada.
fn clave_de(
    view: &IndexView<'_>,
    idx: usize,
    sort_col: u8,
    ruta_completa: impl FnOnce() -> String,
) -> (Clave, String) {
    let nombre = view.get_name(idx).unwrap_or("").to_ascii_lowercase();
    let clave = match sort_col {
        0 => Clave::Texto(nombre.clone()),
        1 => Clave::Texto(ruta_completa().to_ascii_lowercase()),
        2 => Clave::Texto(view.get_extension(idx).to_ascii_lowercase()),
        3 => Clave::Numero(view.size_of(idx)),
        4 => Clave::Numero(view.mtime[idx] as u64),
        5 => Clave::Numero(rango_de_tipo(view.is_dir(idx), view.get_extension(idx))),
        _ => Clave::Numero(idx as u64),
    };
    (clave, nombre)
}

/// Cuántas de las entradas anuladas del base habrían casado con la consulta.
///
/// Sin esta resta el contador diría «1.204 objetos» cuando tres de ellos se
/// borraron hace un segundo. Recorre solo el conjunto de lápidas, que es
/// pequeño: en cuanto crece, el servicio compacta y desaparece.
fn lapidas_que_casaban(base: &IndexView<'_>, query: &str, tombstones: &[u32]) -> u32 {
    if tombstones.is_empty() {
        return 0;
    }
    let ast = Parser::parse(query);
    let compiled = CompiledQuery::compile(&ast, base);
    let mut scratch = MatchScratch::new();
    let mut n = 0u32;
    for &id in tombstones {
        let idx = id as usize;
        if idx < base.entry_count() && base.is_alive(idx) && compiled.matches(base, idx, &mut scratch)
        {
            n += 1;
        }
    }
    n
}

/// Resultado de consultar las dos capas a la vez.
///
/// Es un [`SearchResult`] cuyos identificadores están en el espacio unificado.
#[allow(clippy::too_many_arguments)]
pub fn search_merged(
    engine: &Engine,
    base: &IndexView<'_>,
    snapshot: Option<&OverlaySnapshot>,
    overlay_ids: Option<&OverlayIndex>,
    query: &str,
    scope: &str,
    sort_col: u8,
    ascending: bool,
    limit: usize,
) -> (u64, SearchResult) {
    // El acotado a una carpeta viaja aparte y no dentro del texto de la
    // consulta.
    //
    // Sobre el índice base se traduce a un filtro de ruta, que es recursivo:
    // buscar «kick» dentro de C:\Musica encuentra también
    // C:\Musica\Sets\kick.wav, que es lo que espera cualquiera que venga del
    // Explorador. Sobre la capa de cambios **no** puede usarse ese filtro,
    // porque la ruta de una entrada de la capa solo se puede reconstruir
    // cruzando al base; allí se comprueba después, sobre la ruta ya resuelta.
    let consulta_base = if scope.is_empty() {
        query.to_string()
    } else {
        format!("ruta:\"{scope}\" {query}")
    };
    let consulta_base = consulta_base.trim().to_string();

    let Some(snap) = snapshot else {
        // Sin capa publicada, la búsqueda es la de siempre.
        return engine.search_with_limit(base, &consulta_base, sort_col, ascending, limit);
    };

    let mut tombs: Vec<u32> = snap.tombstones.iter().copied().collect();
    tombs.sort_unstable();

    // Al base se le piden filas de más: las que resulten estar anuladas dejarían
    // huecos al final de la ventana, y esos huecos se rellenan con filas que el
    // base sí tenía pero que no habrían entrado en el recorte.
    let colchon = tombs.len().min(50_000);
    let (generacion, base_res) =
        engine.search_with_limit(base, &consulta_base, sort_col, ascending, limit + colchon);

    let vista_capa = snap.view().ok();

    // --- Candidatas del base, sin las anuladas ---
    let mut candidatas: Vec<Candidata> = Vec::with_capacity(limit + 64);
    for &id in &base_res.entry_ids {
        if snap.tombstones.contains(&id) {
            continue;
        }
        let idx = id as usize;
        if idx >= base.entry_count() {
            continue;
        }
        let (clave, nombre) = clave_de(base, idx, sort_col, || base.resolve_full_path(idx));
        candidatas.push(Candidata { id, es_dir: base.is_dir(idx), clave, nombre });
    }

    // --- Candidatas de la capa ---
    let mut coincidencias_capa = 0u32;
    if let Some(vc) = vista_capa.as_ref()
        && vc.entry_count() > 0
    {
        // La capa se consulta con un motor **propio**, no con el del base.
        //
        // Un `Engine` guarda una pila de refinamiento con los resultados de las
        // consultas anteriores, y esos identificadores son posiciones dentro del
        // índice que los produjo. Pasarle dos índices distintos al mismo motor
        // hace que la consulta sobre el segundo reciba el resultado cacheado del
        // primero: la capa devolvía cero coincidencias porque el base ya había
        // respondido cero a esa misma consulta.
        //
        // Construir uno nuevo cada vez no cuesta nada aquí: la capa tiene unos
        // miles de entradas como mucho, así que recorrerla entera son
        // microsegundos y la caché no aportaría nada.
        let motor_capa = Engine::new();
        // La capa es pequeña, así que se consulta entera y sin recortar, y con
        // la consulta **sin** el filtro de ruta: se comprueba luego.
        let (_g, res_capa) =
            motor_capa.search_with_limit(vc, query, sort_col, ascending, vc.entry_count().max(1));
        let ambito = scope.to_ascii_lowercase();
        for &local in &res_capa.entry_ids {
            if snap.dead.contains(&local) {
                continue;
            }
            let idx = local as usize;
            if idx >= vc.entry_count() {
                continue;
            }
            let unificado = snap.base_count + local;

            // Ruta completa de la entrada nueva, cruzando al base si su carpeta
            // ya estaba en el índice.
            let ruta = match overlay_ids {
                Some(oi) => {
                    let prefijo = vc
                        .vol_table
                        .get(vc.volume[idx])
                        .map(|v| v.mount_prefix.clone())
                        .or_else(|| {
                            base.vol_table
                                .get(vc.volume[idx])
                                .map(|v| v.mount_prefix.clone())
                        })
                        .unwrap_or_default();
                    oi.resolve_path(Some(base), unificado, &prefijo)
                }
                None => vc.get_name(idx).unwrap_or("").to_string(),
            };
            if !ambito.is_empty() && !ruta.to_ascii_lowercase().contains(&ambito) {
                continue;
            }

            coincidencias_capa += 1;
            let ruta_para_clave = ruta.clone();
            let (clave, nombre) = clave_de(vc, idx, sort_col, || {
                // La ruta de una entrada de la capa puede subir hasta una carpeta
                // que vive en el base, así que se resuelve con el índice
                // unificado y no con la vista de la capa sola.
                ruta_para_clave
            });
            candidatas.push(Candidata {
                id: unificado,
                es_dir: vc.is_dir(idx),
                clave,
                nombre,
            });
        }
    }

    // --- Fusión ---
    candidatas.sort_by(|a, b| {
        if a.es_dir != b.es_dir {
            return if a.es_dir { Ordering::Less } else { Ordering::Greater };
        }
        let orden = a.clave.cmp_con(&b.clave);
        if orden == Ordering::Equal {
            a.nombre.cmp(&b.nombre).then(a.id.cmp(&b.id))
        } else if ascending {
            orden
        } else {
            orden.reverse()
        }
    });
    candidatas.truncate(limit);

    let anuladas = lapidas_que_casaban(base, &consulta_base, &tombs);
    let total = base_res
        .total_count
        .saturating_sub(anuladas)
        .saturating_add(coincidencias_capa);

    let ids: Vec<u32> = candidatas.into_iter().map(|c| c.id).collect();
    let truncado = (total as usize) > ids.len();

    (
        generacion,
        SearchResult {
            entry_ids: ids,
            generation: generacion,
            total_count: total,
            elapsed_ms: base_res.elapsed_ms,
            truncated: truncado,
            limit: limit as u32,
        },
    )
}

/// Ordena un conjunto conocido de identificadores unificados.
///
/// Es lo que necesita el explorador: los hijos de una carpeta ya se saben
/// —salen de la columna de hijos del índice más los que haya en la capa—, así
/// que no hay nada que buscar, solo que ordenar.
///
/// Se ordena comparando valores reales por el mismo motivo que en la fusión de
/// búsquedas: las claves enteras precalculadas de cada índice no son comparables
/// entre sí. Aquí además da igual el coste, porque una carpeta tiene los
/// archivos que tiene y no nueve millones.
#[allow(clippy::too_many_arguments)]
pub fn rank_merged(
    base: &IndexView<'_>,
    snapshot: Option<&OverlaySnapshot>,
    overlay_ids: Option<&OverlayIndex>,
    ids: Vec<u32>,
    sort_col: u8,
    ascending: bool,
    limit: usize,
    generation: u64,
) -> SearchResult {
    let base_count = snapshot
        .map(|s| s.base_count)
        .unwrap_or(base.entry_count() as u32);
    let vista_capa = snapshot.and_then(|s| s.view().ok());
    let total = ids.len() as u32;

    let mut candidatas: Vec<Candidata> = Vec::with_capacity(ids.len());
    for id in ids {
        if id < base_count {
            let idx = id as usize;
            if idx >= base.entry_count() {
                continue;
            }
            let (clave, nombre) = clave_de(base, idx, sort_col, || base.resolve_full_path(idx));
            candidatas.push(Candidata { id, es_dir: base.is_dir(idx), clave, nombre });
        } else if let Some(vc) = vista_capa.as_ref() {
            let idx = (id - base_count) as usize;
            if idx >= vc.entry_count() {
                continue;
            }
            let (clave, nombre) = clave_de(vc, idx, sort_col, || match overlay_ids {
                Some(oi) => {
                    let prefijo = vc
                        .vol_table
                        .get(vc.volume[idx])
                        .map(|v| v.mount_prefix.clone())
                        .or_else(|| {
                            base.vol_table
                                .get(vc.volume[idx])
                                .map(|v| v.mount_prefix.clone())
                        })
                        .unwrap_or_default();
                    oi.resolve_path(Some(base), id, &prefijo)
                }
                None => vc.get_name(idx).unwrap_or("").to_string(),
            });
            candidatas.push(Candidata { id, es_dir: vc.is_dir(idx), clave, nombre });
        }
    }

    candidatas.sort_by(|a, b| {
        if a.es_dir != b.es_dir {
            return if a.es_dir { Ordering::Less } else { Ordering::Greater };
        }
        let orden = a.clave.cmp_con(&b.clave);
        if orden == Ordering::Equal {
            a.nombre.cmp(&b.nombre).then(a.id.cmp(&b.id))
        } else if ascending {
            orden
        } else {
            orden.reverse()
        }
    });
    candidatas.truncate(limit);

    let entry_ids: Vec<u32> = candidatas.into_iter().map(|c| c.id).collect();
    let truncated = (total as usize) > entry_ids.len();
    SearchResult {
        entry_ids,
        generation,
        total_count: total,
        elapsed_ms: 0,
        truncated,
        limit: limit as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::overlay_file::{OVERLAY_FILE_NAME, write_overlay};
    use crate::index::{IndexBuilder, MmapIndex, OverlayIndex};
    use std::fs::File;
    use std::io::BufWriter;
    use tempfile::TempDir;

    /// Índice base con cuatro pistas dentro de C:\Musica.
    fn escenario() -> (TempDir, MmapIndex, u32) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("index.bdjx");

        let mut b = IndexBuilder::new();
        b.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);
        let raiz = b.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
        let musica = b.add_entry(raiz, "Musica", true, false, false, 0, 0, 10, 10);
        b.add_entry(musica, "beta.wav", false, false, false, 0, 200, 20, 20);
        b.add_entry(musica, "delta.wav", false, false, false, 0, 400, 40, 40);
        b.add_entry(musica, "zulu.wav", false, false, false, 0, 100, 30, 30);
        {
            let f = File::create(&path).unwrap();
            let mut w = BufWriter::new(f);
            b.write_to(&mut w).unwrap();
        }
        let mmap = MmapIndex::open(&path).unwrap();
        let n = mmap.view().unwrap().entry_count() as u32;
        (dir, mmap, n)
    }

    fn publicar(dir: &TempDir, capa: &mut OverlayIndex, generacion: u64) -> OverlaySnapshot {
        let p = dir.path().join(OVERLAY_FILE_NAME);
        write_overlay(capa, generacion, 1, &p).unwrap();
        OverlaySnapshot::open(&p, generacion).unwrap()
    }

    #[test]
    fn un_archivo_recien_creado_aparece_sin_esperar_a_la_compactacion() {
        // Es el caso que motiva todo esto: el usuario copia algo y quiere verlo.
        let (dir, mmap, base_count) = escenario();
        let vista = mmap.view().unwrap();
        let motor = Engine::new();

        // Antes de publicar nada, la búsqueda no lo encuentra.
        let (_g, sin_capa) = motor.search_with_limit(&vista, "alfa", 0, true, 100);
        assert_eq!(sin_capa.total_count, 0);

        let mut capa = OverlayIndex::with_base_count(base_count);
        capa.builder.vol_table = vista.vol_table.clone();
        capa.add_entry(1, "alfa.wav", false, false, false, 0, 50, 50, 50);
        let snap = publicar(&dir, &mut capa, vista.header.generation);
        let oi = snap.to_overlay_index(Some(&mmap));

        let motor2 = Engine::new();
        let (_g, con_capa) =
            search_merged(&motor2, &vista, Some(&snap), oi.as_ref(), "alfa", "", 0, true, 100);
        assert_eq!(con_capa.total_count, 1, "el archivo nuevo debe contarse");
        assert_eq!(con_capa.entry_ids, vec![base_count]);
    }

    #[test]
    fn un_archivo_borrado_deja_de_aparecer_y_deja_de_contarse() {
        // El otro lado: si una operación de borrado dejara registros fantasma,
        // el usuario vería filas que ya no existen en el disco.
        let (dir, mmap, base_count) = escenario();
        let vista = mmap.view().unwrap();

        let motor = Engine::new();
        let (_g, antes) = motor.search_with_limit(&vista, "wav", 0, true, 100);
        assert_eq!(antes.total_count, 3);

        // Se borra «delta.wav», que es el identificador 3.
        let mut capa = OverlayIndex::with_base_count(base_count);
        capa.builder.vol_table = vista.vol_table.clone();
        capa.mark_deleted(3);
        let snap = publicar(&dir, &mut capa, vista.header.generation);
        let oi = snap.to_overlay_index(Some(&mmap));

        let motor2 = Engine::new();
        let (_g, despues) =
            search_merged(&motor2, &vista, Some(&snap), oi.as_ref(), "wav", "", 0, true, 100);
        assert_eq!(despues.total_count, 2, "el borrado debe restar del contador");
        assert!(
            !despues.entry_ids.contains(&3),
            "una entrada anulada no puede seguir apareciendo en la lista"
        );
    }

    #[test]
    fn la_entrada_nueva_se_coloca_en_su_sitio_alfabetico() {
        // Si la fusión comparase los rangos alfabéticos de cada índice —que son
        // relativos a su propio archivo— la fila nueva saldría en un sitio
        // arbitrario. Este es el fallo silencioso que la fusión por valores
        // reales evita.
        let (dir, mmap, base_count) = escenario();
        let vista = mmap.view().unwrap();

        let mut capa = OverlayIndex::with_base_count(base_count);
        capa.builder.vol_table = vista.vol_table.clone();
        // «charlie» va entre «beta» y «delta».
        capa.add_entry(1, "charlie.wav", false, false, false, 0, 300, 35, 35);
        let snap = publicar(&dir, &mut capa, vista.header.generation);
        let oi = snap.to_overlay_index(Some(&mmap));

        let motor = Engine::new();
        let (_g, res) =
            search_merged(&motor, &vista, Some(&snap), oi.as_ref(), "wav", "", 0, true, 100);

        let nombres: Vec<String> = res
            .entry_ids
            .iter()
            .map(|&id| {
                if id < base_count {
                    vista.get_name(id as usize).unwrap_or("").to_string()
                } else {
                    let vc = snap.view().unwrap();
                    vc.get_name((id - base_count) as usize).unwrap_or("").to_string()
                }
            })
            .collect();

        assert_eq!(
            nombres,
            vec!["beta.wav", "charlie.wav", "delta.wav", "zulu.wav"],
            "la entrada de la capa debe caer en su posición alfabética real"
        );
    }

    #[test]
    fn ordenar_por_tamano_tambien_mezcla_bien_las_dos_capas() {
        let (dir, mmap, base_count) = escenario();
        let vista = mmap.view().unwrap();

        let mut capa = OverlayIndex::with_base_count(base_count);
        capa.builder.vol_table = vista.vol_table.clone();
        // 300 bytes: entre zulu (100), beta (200) y delta (400).
        capa.add_entry(1, "medio.wav", false, false, false, 0, 300, 35, 35);
        let snap = publicar(&dir, &mut capa, vista.header.generation);
        let oi = snap.to_overlay_index(Some(&mmap));

        let motor = Engine::new();
        let (_g, res) =
            search_merged(&motor, &vista, Some(&snap), oi.as_ref(), "wav", "", 3, true, 100);

        let tamanos: Vec<u64> = res
            .entry_ids
            .iter()
            .map(|&id| {
                if id < base_count {
                    vista.size_of(id as usize)
                } else {
                    snap.view().unwrap().size_of((id - base_count) as usize)
                }
            })
            .collect();
        assert_eq!(tamanos, vec![100, 200, 300, 400]);
    }

    #[test]
    fn sin_capa_publicada_el_resultado_es_el_de_siempre() {
        let (_d, mmap, _n) = escenario();
        let vista = mmap.view().unwrap();

        let motor_a = Engine::new();
        let (_g, directo) = motor_a.search_with_limit(&vista, "wav", 0, true, 100);
        let motor_b = Engine::new();
        let (_g, fundido) = search_merged(&motor_b, &vista, None, None, "wav", "", 0, true, 100);

        assert_eq!(directo.entry_ids, fundido.entry_ids);
        assert_eq!(directo.total_count, fundido.total_count);
    }

    #[test]
    fn acotar_a_una_carpeta_tambien_acota_lo_recien_copiado() {
        // El acotado tiene que valer para las dos capas. Si solo filtrase el
        // índice base, buscar dentro de una carpeta escondería justo el archivo
        // que el usuario acaba de copiar ahí, que es cuando más quiere verlo.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("index.bdjx");

        let mut b = IndexBuilder::new();
        b.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);
        let raiz = b.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
        let musica = b.add_entry(raiz, "Musica", true, false, false, 0, 0, 0, 0);
        let otra = b.add_entry(raiz, "Documentos", true, false, false, 0, 0, 0, 0);
        b.add_entry(musica, "dentro.wav", false, false, false, 0, 1, 1, 1);
        b.add_entry(otra, "fuera.wav", false, false, false, 0, 2, 2, 2);
        {
            let f = File::create(&path).unwrap();
            let mut w = BufWriter::new(f);
            b.write_to(&mut w).unwrap();
        }
        let mmap = MmapIndex::open(&path).unwrap();
        let vista = mmap.view().unwrap();
        let base_count = vista.entry_count() as u32;

        let mut capa = OverlayIndex::with_base_count(base_count);
        capa.builder.vol_table = vista.vol_table.clone();
        capa.add_entry(musica, "copiado.wav", false, false, false, 0, 3, 3, 3);
        capa.add_entry(otra, "ajeno.wav", false, false, false, 0, 4, 4, 4);
        let snap = publicar(&dir, &mut capa, vista.header.generation);
        let oi = snap.to_overlay_index(Some(&mmap));

        let motor = Engine::new();
        let (_g, res) = search_merged(
            &motor,
            &vista,
            Some(&snap),
            oi.as_ref(),
            "wav",
            "C:\\Musica",
            0,
            true,
            100,
        );

        assert_eq!(
            res.total_count, 2,
            "dentro de C:\\Musica hay dos: la del índice y la recién copiada"
        );
        let nombres: Vec<String> = res
            .entry_ids
            .iter()
            .map(|&id| {
                if id < base_count {
                    vista.get_name(id as usize).unwrap_or("").to_string()
                } else {
                    snap.view()
                        .unwrap()
                        .get_name((id - base_count) as usize)
                        .unwrap_or("")
                        .to_string()
                }
            })
            .collect();
        assert_eq!(nombres, vec!["copiado.wav", "dentro.wav"]);
    }

    #[test]
    fn una_entrada_creada_y_borrada_antes_de_publicar_no_aparece() {
        let (dir, mmap, base_count) = escenario();
        let vista = mmap.view().unwrap();

        let mut capa = OverlayIndex::with_base_count(base_count);
        capa.builder.vol_table = vista.vol_table.clone();
        let efimero = capa.add_entry(1, "temporal.wav", false, false, false, 0, 1, 1, 1);
        capa.mark_deleted(efimero);
        let snap = publicar(&dir, &mut capa, vista.header.generation);
        let oi = snap.to_overlay_index(Some(&mmap));

        let motor = Engine::new();
        let (_g, res) =
            search_merged(&motor, &vista, Some(&snap), oi.as_ref(), "temporal", "", 0, true, 100);
        assert_eq!(res.total_count, 0);
        assert!(res.entry_ids.is_empty());
    }
}
