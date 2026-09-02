//! La ordenación tiene que dar lo mismo quepa o no quepa el resultado entero.
//!
//! El motor tiene dos caminos: si las coincidencias caben en la ventana pedida,
//! se ordenan todas; si no, cada bloque se recorta a sus mejores `N` **por una
//! clave entera** y solo esa ventana se ordena del todo.
//!
//! Si la clave entera no representa la misma ordenación que el comparador final,
//! un resultado grande enseña unas filas y las ordena con otro criterio: el
//! usuario ve las treinta primeras de un orden que no ha pedido, sin ningún aviso
//! de que algo va mal. Es un fallo silencioso, y por eso tiene su propia prueba.

use bdj_search_core::index::{IndexBuilder, MmapIndex};
use bdj_search_core::search::Engine;
use std::io::BufWriter;
use tempfile::NamedTempFile;

/// Corpus variado a propósito: extensiones de todas las categorías, carpetas,
/// archivos sin extensión, tamaños y fechas repetidas.
fn corpus(n: usize) -> (NamedTempFile, MmapIndex) {
    const EXTS: [&str; 12] = [
        "wav", "mp3", "flac", "als", "zip", "pdf", "jpg", "mp4", "exe", "txt", "flp", "rar",
    ];
    let mut b = IndexBuilder::with_capacity(n);
    b.vol_table.add_or_update("C:\\", "prueba", "NTFS", true);
    let raiz = b.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);

    // Unas cuantas carpetas, para que la ordenación por tipo tenga que ponerlas
    // delante.
    let mut carpetas = Vec::new();
    for i in 0..8 {
        carpetas.push(b.add_entry(
            raiz,
            &format!("Carpeta {i}"),
            true,
            false,
            false,
            0,
            0,
            1000 + i,
            1000,
        ));
    }

    for i in 0..n {
        let padre = carpetas[i % carpetas.len()];
        // Uno de cada trece se queda sin extensión.
        let nombre = if i % 13 == 0 {
            format!("archivo sin punto {i}")
        } else {
            format!("pista {i} de prueba.{}", EXTS[i % EXTS.len()])
        };
        b.add_entry(
            padre,
            &nombre,
            false,
            false,
            false,
            0,
            ((i % 97) as u64) * 1024,
            1_700_000_000 + (i % 53) as u32,
            1_700_000_000,
        );
    }

    let temp = NamedTempFile::new().unwrap();
    let file = std::fs::File::create(temp.path()).unwrap();
    let mut w = BufWriter::new(file);
    b.write_to(&mut w).unwrap();
    drop(w);
    let mmap = MmapIndex::open(temp.path()).unwrap();
    (temp, mmap)
}

/// Nombre de cada columna, para que un fallo diga cuál es.
fn nombre_columna(col: u8) -> &'static str {
    match col {
        0 => "Nombre",
        1 => "Ruta",
        2 => "Extensión",
        3 => "Tamaño",
        4 => "Modificado",
        5 => "Tipo",
        _ => "desconocida",
    }
}

#[test]
fn la_ventana_recortada_empieza_donde_empieza_el_resultado_completo() {
    let n = 4000usize;
    let (_t, mmap) = corpus(n);
    let view = mmap.view().unwrap();

    // «prueba» aparece en todos los nombres de archivo.
    let consulta = "prueba";
    const VENTANA: usize = 50;

    for col in [0u8, 1, 2, 3, 4, 5] {
        for ascendente in [true, false] {
            // Camino A: la ventana entera, sin recortar.
            let motor_a = Engine::new();
            let (_g, completo) =
                motor_a.search_with_limit(&view, consulta, col, ascendente, n * 2);

            // Camino B: ventana pequeña, con recorte por bloques.
            let motor_b = Engine::new();
            let (_g, recortado) =
                motor_b.search_with_limit(&view, consulta, col, ascendente, VENTANA);

            assert_eq!(
                completo.total_count, recortado.total_count,
                "columna {}: el conteo total no puede depender de la ventana",
                nombre_columna(col)
            );
            assert!(
                recortado.truncated,
                "columna {}: con {} coincidencias y ventana de {VENTANA} debería avisar de que hay más",
                nombre_columna(col),
                recortado.total_count
            );
            assert_eq!(
                recortado.entry_ids.len(),
                VENTANA,
                "columna {}: la ventana debe venir llena",
                nombre_columna(col)
            );

            let cabeza: Vec<u32> = completo.entry_ids.iter().take(VENTANA).copied().collect();
            assert_eq!(
                recortado.entry_ids, cabeza,
                "columna {} ({}): la ventana recortada no coincide con la cabeza del resultado completo. \
                 La clave con la que se eligen las filas no representa el mismo orden que el comparador final.",
                nombre_columna(col),
                if ascendente { "ascendente" } else { "descendente" }
            );
        }
    }
}

#[test]
fn una_consulta_vacia_no_devuelve_resultados() {
    // Devolver aquí el número de entradas del índice hacía que la tabla creyera
    // que tenía millones de filas que pintar nada más arrancar.
    let (_t, mmap) = corpus(500);
    let view = mmap.view().unwrap();
    let motor = Engine::new();

    for consulta in ["", "   ", "\t"] {
        let (_g, res) = motor.search(&view, consulta, 0, true);
        assert_eq!(
            res.total_count, 0,
            "una consulta vacía no son «todos los archivos», son cero resultados"
        );
        assert!(res.entry_ids.is_empty());
        assert!(!res.truncated);
    }
}

#[test]
fn ordenar_por_tipo_pone_las_carpetas_primero() {
    let (_t, mmap) = corpus(300);
    let view = mmap.view().unwrap();
    let motor = Engine::new();

    // «a» aparece tanto en «Carpeta N» como en «pista N de prueba».
    let (_g, res) = motor.search_with_limit(&view, "a", 5, true, 1000);
    assert!(res.total_count > 8);

    let carpetas_al_principio = res
        .entry_ids
        .iter()
        .take(8)
        .all(|&id| view.is_dir(id as usize));
    assert!(
        carpetas_al_principio,
        "las ocho carpetas deberían encabezar la lista al ordenar por tipo"
    );
}

#[test]
fn ordenar_por_extension_agrupa_las_iguales() {
    let (_t, mmap) = corpus(600);
    let view = mmap.view().unwrap();
    let motor = Engine::new();

    let (_g, res) = motor.search_with_limit(&view, "prueba", 2, true, 2000);
    let extensiones: Vec<String> = res
        .entry_ids
        .iter()
        .map(|&id| view.get_extension(id as usize).to_ascii_lowercase())
        .collect();

    // Cada extensión debe aparecer en un único tramo contiguo.
    let mut vistas: Vec<&str> = Vec::new();
    let mut anterior = "";
    for e in &extensiones {
        if e != anterior {
            assert!(
                !vistas.contains(&e.as_str()),
                "la extensión «{e}» aparece en dos tramos: no está agrupada"
            );
            vistas.push(e);
            anterior = e;
        }
    }
}
