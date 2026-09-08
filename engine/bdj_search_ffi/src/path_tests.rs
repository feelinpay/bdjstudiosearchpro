#![cfg(test)]
//! Las rutas que la interfaz recibe tienen que ser rutas de verdad.
//!
//! Un usuario informó de que copiar el nombre, copiar la ruta, mostrar la
//! ubicación, renombrar, duplicar, comprimir y enviar a la papelera habían
//! dejado de funcionar **todas a la vez**, y que lo que salía era «solo la ruta
//! C». Siete funciones distintas rompiéndose el mismo día no son siete fallos:
//! son uno solo, en lo único que todas comparten, que es preguntarle al motor
//! por la ruta de la fila seleccionada.
//!
//! Esa resolución se reescribió al introducir la capa de cambios: antes se
//! indexaba la vista del índice directamente y ahora pasa por un resolvedor que
//! decide, según el identificador, si la entrada vive en el índice base o en la
//! capa. Si esa decisión se equivoca, no falla ruidosamente: devuelve la ruta de
//! otra entrada, o la raíz del volumen.
//!
//! Esta prueba recorre el camino entero —abrir el índice, buscar, navegar, pedir
//! filas y pedir rutas— igual que lo hace la aplicación, y comprueba que lo que
//! sale son las rutas correctas.
//!
//! Va toda en una sola función a propósito: el motor guarda su estado en
//! variables globales, así que dos pruebas en paralelo se pisarían.

use bdj_search_core::index::{IndexBuilder, MmapIndex, OverlayIndex, write_overlay};
use crate::api;
use std::io::BufWriter;
use tempfile::TempDir;

/// Índice de prueba con una estructura realista de tres niveles.
fn escribir_indice(dir: &TempDir) -> (std::path::PathBuf, u32, u64) {
    let path = dir.path().join("index.bdjx");
    let mut b = IndexBuilder::new();
    b.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);

    let raiz = b.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
    let musica = b.add_entry(raiz, "Musica", true, false, false, 0, 0, 10, 10);
    let sets = b.add_entry(musica, "Sets 2026", true, false, false, 0, 0, 11, 11);
    b.add_entry(sets, "intro.wav", false, false, false, 0, 111, 20, 20);
    b.add_entry(sets, "cierre.wav", false, false, false, 0, 222, 21, 21);
    b.add_entry(musica, "acapella.mp3", false, false, false, 0, 333, 22, 22);

    {
        let f = std::fs::File::create(&path).unwrap();
        let mut w = BufWriter::new(f);
        b.write_to(&mut w).unwrap();
    }
    let mmap = MmapIndex::open(&path).unwrap();
    let v = mmap.view().unwrap();
    (path.clone(), v.entry_count() as u32, v.header.generation)
}

/// Todas las rutas de un resultado, en orden.
fn rutas_de(generacion: u64, cuantas: u32) -> Vec<String> {
    let filas: Vec<u32> = (0..cuantas).collect();
    api::paths_for_rows(generacion, filas)
}

#[test]
fn la_interfaz_recibe_rutas_reales_con_y_sin_capa_de_cambios() {
    let dir = TempDir::new().unwrap();
    let (index_path, base_count, generacion_base) = escribir_indice(&dir);

    // ─────────────── 1. Sin capa publicada ───────────────
    api::engine_open(index_path.to_string_lossy().to_string()).expect("el índice debe abrirse");

    let g = api::search_with_limit("wav".into(), String::new(), 0, true, 100);
    let st = api::search_status(g);
    assert_eq!(st.total_count, 2, "hay dos .wav en el índice");

    let rutas = rutas_de(g, st.ready_count);
    let mut ordenadas = rutas.clone();
    ordenadas.sort();
    assert_eq!(
        ordenadas,
        vec![
            "C:\\Musica\\Sets 2026\\cierre.wav".to_string(),
            "C:\\Musica\\Sets 2026\\intro.wav".to_string(),
        ],
        "sin capa, las rutas ya tienen que ser completas"
    );

    // Y la de una sola fila, que es la que usa «copiar ruta».
    let una = api::full_path(g, 0);
    assert!(
        una.starts_with("C:\\Musica\\Sets 2026\\"),
        "«copiar ruta» devolvió «{una}»"
    );
    assert_ne!(una, "C:\\", "no puede devolver la raíz del volumen");

    // Las filas traen nombre y carpeta contenedora de verdad.
    let lote = api::rows(g, 0, 10);
    assert_eq!(lote.count, 2);
    for i in 0..lote.count as usize {
        assert!(
            lote.names[i].ends_with(".wav"),
            "«copiar nombre» devolvió «{}»",
            lote.names[i]
        );
        assert_eq!(
            lote.paths[i], "C:\\Musica\\Sets 2026",
            "«mostrar ubicación» devolvió «{}»",
            lote.paths[i]
        );
    }

    // ─────────────── 2. Navegando una carpeta ───────────────
    let gb = api::browse_path("C:\\Musica\\Sets 2026".into(), 0, true, 100);
    let stb = api::search_status(gb);
    assert_eq!(stb.total_count, 2, "la carpeta tiene dos archivos");
    let rutas_b = rutas_de(gb, stb.ready_count);
    for r in &rutas_b {
        assert!(
            r.starts_with("C:\\Musica\\Sets 2026\\"),
            "navegando, la ruta salió «{r}»"
        );
    }

    // ─────────────── 3. Con una capa publicada ───────────────
    // Es el caso nuevo, y el que rompió las rutas: la capa introduce un segundo
    // espacio de identificadores y hay que decidir bien a cuál pertenece cada
    // fila.
    let mut capa = OverlayIndex::with_base_count(base_count);
    {
        let mmap = MmapIndex::open(&index_path).unwrap();
        capa.builder.vol_table = mmap.view().unwrap().vol_table.clone();
    }
    // Un archivo recién copiado dentro de «Sets 2026» (identificador 2).
    capa.add_entry(2, "recien.wav", false, false, false, 0, 444, 30, 30);
    let capa_path = bdj_search_core::index::overlay_path_for(&index_path);
    write_overlay(&mut capa, generacion_base, 1, &capa_path).unwrap();

    // La aplicación se entera igual que en producción.
    assert!(api::reload_if_changed(), "debería detectar la capa nueva");

    let g2 = api::search_with_limit("wav".into(), String::new(), 0, true, 100);
    let st2 = api::search_status(g2);
    assert_eq!(st2.total_count, 3, "ahora hay tres .wav: dos del base y el nuevo");

    let rutas2 = rutas_de(g2, st2.ready_count);
    assert_eq!(rutas2.len(), 3, "tienen que venir las tres rutas");
    for r in &rutas2 {
        assert!(
            r.starts_with("C:\\Musica\\Sets 2026\\") && r.len() > "C:\\Musica\\Sets 2026\\".len(),
            "con la capa publicada, una ruta salió «{r}»"
        );
        assert_ne!(r, "C:\\", "ninguna fila puede resolver a la raíz del volumen");
    }
    let mut ord2 = rutas2.clone();
    ord2.sort();
    assert_eq!(
        ord2,
        vec![
            "C:\\Musica\\Sets 2026\\cierre.wav".to_string(),
            "C:\\Musica\\Sets 2026\\intro.wav".to_string(),
            "C:\\Musica\\Sets 2026\\recien.wav".to_string(),
        ],
        "las rutas del base y las de la capa tienen que salir todas bien"
    );

    // Las filas también: nombre y carpeta, con la capa mezclada.
    let lote2 = api::rows(g2, 0, 10);
    assert_eq!(lote2.count, 3);
    for i in 0..lote2.count as usize {
        assert!(
            !lote2.names[i].is_empty() && lote2.names[i] != "C:\\",
            "el nombre de la fila {i} salió «{}»",
            lote2.names[i]
        );
        assert_eq!(
            lote2.paths[i], "C:\\Musica\\Sets 2026",
            "la carpeta de la fila {i} salió «{}»",
            lote2.paths[i]
        );
    }

    // ─────────────── 4. Navegar con la capa publicada ───────────────
    let gb2 = api::browse_path("C:\\Musica\\Sets 2026".into(), 0, true, 100);
    let stb2 = api::search_status(gb2);
    assert_eq!(
        stb2.total_count, 3,
        "la carpeta ahora tiene tres archivos, contando el recién copiado"
    );
    let rutas_b2 = rutas_de(gb2, stb2.ready_count);
    let mut ordb2 = rutas_b2.clone();
    ordb2.sort();
    assert_eq!(
        ordb2,
        vec![
            "C:\\Musica\\Sets 2026\\cierre.wav".to_string(),
            "C:\\Musica\\Sets 2026\\intro.wav".to_string(),
            "C:\\Musica\\Sets 2026\\recien.wav".to_string(),
        ],
        "navegando con la capa publicada, las rutas tienen que seguir siendo completas"
    );

    api::engine_close();
}
