//! El explorador se apoya en dos cosas que el índice ya tiene: la lista de
//! hijos de cada carpeta y la traducción de una ruta a su entrada.
//!
//! Ninguna de las dos puede tocar el disco: entrar en una carpeta con cincuenta
//! mil archivos tiene que costar lo mismo que leer una rodaja de memoria.

use bdj_search_core::index::{IndexBuilder, MmapIndex, OverlayIndex};
use std::io::BufWriter;
use tempfile::NamedTempFile;

/// Construye un árbol pequeño:
///
/// ```text
/// C:\  (raíz, sin nombre)
///   Musica\
///     Sets\
///       set1.als
///       set2.als
///     kick.wav
///   Documentos\
///     notas.txt
/// ```
fn arbol() -> (NamedTempFile, MmapIndex) {
    let mut b = IndexBuilder::new();
    b.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);

    let raiz = b.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
    let musica = b.add_entry(raiz, "Musica", true, false, false, 0, 0, 10, 10);
    let sets = b.add_entry(musica, "Sets", true, false, false, 0, 0, 20, 20);
    b.add_entry(sets, "set1.als", false, false, false, 0, 100, 30, 30);
    b.add_entry(sets, "set2.als", false, false, false, 0, 200, 40, 40);
    b.add_entry(musica, "kick.wav", false, false, false, 0, 300, 50, 50);
    let docs = b.add_entry(raiz, "Documentos", true, false, false, 0, 0, 60, 60);
    b.add_entry(docs, "notas.txt", false, false, false, 0, 400, 70, 70);

    let temp = NamedTempFile::new().unwrap();
    let file = std::fs::File::create(temp.path()).unwrap();
    let mut w = BufWriter::new(file);
    b.write_to(&mut w).unwrap();
    drop(w);
    let mmap = MmapIndex::open(temp.path()).unwrap();
    (temp, mmap)
}

#[test]
fn los_hijos_de_una_carpeta_salen_del_indice() {
    let (_t, mmap) = arbol();
    let view = mmap.view().unwrap();

    // 0 raíz, 1 Musica, 2 Sets, 3 set1, 4 set2, 5 kick, 6 Documentos, 7 notas
    let hijos_raiz: Vec<&str> = view
        .children(0)
        .iter()
        .map(|&c| view.get_name(c as usize).unwrap_or(""))
        .collect();
    assert_eq!(hijos_raiz, vec!["Musica", "Documentos"]);

    let hijos_musica: Vec<&str> = view
        .children(1)
        .iter()
        .map(|&c| view.get_name(c as usize).unwrap_or(""))
        .collect();
    assert_eq!(hijos_musica, vec!["Sets", "kick.wav"]);

    let hijos_sets: Vec<&str> = view
        .children(2)
        .iter()
        .map(|&c| view.get_name(c as usize).unwrap_or(""))
        .collect();
    assert_eq!(hijos_sets, vec!["set1.als", "set2.als"]);

    // Un archivo no tiene hijos.
    assert!(view.children(3).is_empty());
    assert_eq!(view.child_count(2), 2);
}

#[test]
fn la_miga_de_pan_sube_hasta_la_raiz() {
    let (_t, mmap) = arbol();
    let view = mmap.view().unwrap();

    // set1.als está en el índice 3.
    let cadena = view.ancestors(3);
    let nombres: Vec<&str> = cadena
        .iter()
        .map(|&c| view.get_name(c as usize).unwrap_or("(raíz)"))
        .collect();
    assert_eq!(nombres, vec!["", "Musica", "Sets"]);
    assert_eq!(view.resolve_full_path(3), "C:\\Musica\\Sets\\set1.als");
}

#[test]
fn las_raices_de_volumen_son_el_punto_de_partida() {
    let (_t, mmap) = arbol();
    let view = mmap.view().unwrap();
    assert_eq!(view.volume_roots(), vec![0]);
}

#[test]
fn una_ruta_se_traduce_a_su_entrada() {
    let (_t, mmap) = arbol();
    let view = mmap.view().unwrap();
    let mut overlay = OverlayIndex::with_base_count(view.entry_count() as u32);
    overlay.builder.vol_table = view.vol_table.clone();

    assert_eq!(
        overlay.resolve_id_by_path(Some(&view), "C:\\Musica\\Sets\\set1.als"),
        Some(3)
    );
    assert_eq!(
        overlay.resolve_id_by_path(Some(&view), "C:\\Musica"),
        Some(1)
    );
    // Sin distinguir mayúsculas ni el tipo de barra.
    assert_eq!(
        overlay.resolve_id_by_path(Some(&view), "c:/MUSICA/sets"),
        Some(2)
    );
    // La raíz del volumen.
    assert_eq!(overlay.resolve_id_by_path(Some(&view), "C:\\"), Some(0));
    // Algo que no existe.
    assert_eq!(
        overlay.resolve_id_by_path(Some(&view), "C:\\Musica\\Inexistente.wav"),
        None
    );
    // Un volumen que no está en la tabla.
    assert_eq!(
        overlay.resolve_id_by_path(Some(&view), "Z:\\lo que sea"),
        None
    );
}

#[test]
fn una_entrada_creada_en_la_capa_tambien_se_encuentra_por_ruta() {
    let (_t, mmap) = arbol();
    let view = mmap.view().unwrap();
    let mut overlay = OverlayIndex::with_base_count(view.entry_count() as u32);
    overlay.builder.vol_table = view.vol_table.clone();

    // La aplicación acaba de copiar un archivo dentro de una carpeta que ya
    // estaba en el índice base.
    let nuevo = overlay.add_entry(1, "snare.wav", false, false, false, 0, 500, 80, 80);
    assert_eq!(
        overlay.resolve_id_by_path(Some(&view), "C:\\Musica\\snare.wav"),
        Some(nuevo)
    );

    // Y sus hijos se ven junto a los del base.
    let hijos = overlay.children_of(Some(&view), 1);
    assert!(hijos.contains(&2), "Sets debe seguir estando");
    assert!(hijos.contains(&5), "kick.wav debe seguir estando");
    assert!(hijos.contains(&nuevo), "snare.wav debe aparecer ya");
}

#[test]
fn una_entrada_anulada_desaparece_de_la_lista_de_hijos() {
    let (_t, mmap) = arbol();
    let view = mmap.view().unwrap();
    let mut overlay = OverlayIndex::with_base_count(view.entry_count() as u32);
    overlay.builder.vol_table = view.vol_table.clone();

    overlay.mark_deleted(5); // kick.wav
    let hijos = overlay.children_of(Some(&view), 1);
    assert!(!hijos.contains(&5), "un archivo borrado no debe listarse");
    assert!(hijos.contains(&2));
}

#[test]
fn los_hijos_sobreviven_a_una_compactacion() {
    let (_t, mmap) = arbol();
    let view = mmap.view().unwrap();
    let mut overlay = OverlayIndex::with_base_count(view.entry_count() as u32);
    overlay.builder.vol_table = view.vol_table.clone();
    overlay.add_entry(2, "set3.als", false, false, false, 0, 600, 90, 90);
    overlay.mark_deleted(3); // set1.als

    let destino = NamedTempFile::new().unwrap();
    overlay
        .compact(Some(&mmap), destino.path(), 2)
        .expect("la compactación debería funcionar");

    let nuevo = MmapIndex::open(destino.path()).unwrap();
    let nv = nuevo.view().unwrap();

    // Buscar «Sets» y comprobar sus hijos en el índice ya compactado.
    let sets = (0..nv.entry_count())
        .find(|&i| nv.get_name(i) == Some("Sets"))
        .expect("Sets debe seguir existiendo");
    let hijos: Vec<&str> = nv
        .children(sets)
        .iter()
        .map(|&c| nv.get_name(c as usize).unwrap_or(""))
        .collect();
    assert_eq!(hijos, vec!["set2.als", "set3.als"]);
}
