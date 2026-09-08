//! Archivos de 4 GiB o más: el caso raro que no puede salir mal.
//!
//! El tamaño pasó de ocho bytes por entrada a cuatro, con una tabla aparte para
//! los que no caben. Eso ahorra cuarenta megas de memoria y otros cuarenta de
//! disco sobre diez millones de archivos, pero introduce un camino nuevo que solo
//! se recorre con archivos grandes.
//!
//! Y un DJ tiene archivos grandes: un pack de samples de 6 GB, un vídeo de un
//! directo, una imagen de disco de una biblioteca entera. Si ese camino
//! estuviera mal, el fallo sería del peor tipo: el archivo aparecería en la lista
//! con un tamaño absurdo, o se ordenaría como si fuera diminuto, y nadie lo
//! relacionaría con un cambio en el formato del índice.

use bdj_search_core::index::{IndexBuilder, MmapIndex};
use bdj_search_core::search::Engine;
use std::io::BufWriter;
use tempfile::NamedTempFile;

const GIB: u64 = 1024 * 1024 * 1024;

/// Justo por debajo del límite de cuatro bytes: tiene que caber en la columna.
const JUSTO_DEBAJO: u64 = u32::MAX as u64 - 1;
/// Exactamente el límite: va a la tabla aparte, porque ese valor es la marca.
const EXACTO: u64 = u32::MAX as u64;

fn indice_con(tamanos: &[(&str, u64)]) -> (NamedTempFile, MmapIndex) {
    let mut b = IndexBuilder::with_capacity(tamanos.len() + 2);
    b.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);
    let raiz = b.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
    for (nombre, size) in tamanos {
        b.add_entry(raiz, nombre, false, false, false, 0, *size, 100, 100);
    }

    let temp = NamedTempFile::new().unwrap();
    let f = std::fs::File::create(temp.path()).unwrap();
    let mut w = BufWriter::new(f);
    b.write_to(&mut w).unwrap();
    drop(w);
    let mmap = MmapIndex::open(temp.path()).unwrap();
    (temp, mmap)
}

#[test]
fn un_archivo_enorme_conserva_su_tamano_exacto() {
    let casos: Vec<(&str, u64)> = vec![
        ("pequeno.wav", 1024),
        ("justo debajo.bin", JUSTO_DEBAJO),
        ("exacto.bin", EXACTO),
        ("pack de samples.zip", 6 * GIB),
        ("directo.mp4", 12 * GIB + 12345),
        ("biblioteca.dmg", 500 * GIB),
        ("vacio.txt", 0),
    ];
    let (_t, mmap) = indice_con(&casos);
    let view = mmap.view().unwrap();

    for (i, (nombre, esperado)) in casos.iter().enumerate() {
        let idx = i + 1; // el 0 es la raíz
        assert_eq!(
            view.size_of(idx),
            *esperado,
            "«{nombre}» debería medir {esperado} bytes"
        );
    }
}

#[test]
fn ordenar_por_tamano_pone_los_enormes_al_final() {
    // Es el fallo que más costaría detectar: si los grandes se ordenaran por su
    // marca en vez de por su tamaño real, quedarían todos juntos al final pero
    // **desordenados entre ellos**, y nadie lo miraría dos veces.
    let casos: Vec<(&str, u64)> = vec![
        ("a mediano.bin", 500 * 1024 * 1024),
        ("b enorme.bin", 50 * GIB),
        ("c pequeno.bin", 1024),
        ("d grande.bin", 5 * GIB),
        ("e gigante.bin", 200 * GIB),
        ("f justo debajo.bin", JUSTO_DEBAJO),
    ];
    let (_t, mmap) = indice_con(&casos);
    let view = mmap.view().unwrap();

    let motor = Engine::new();
    let (_g, res) = motor.search_with_limit(&view, "bin", 3, true, 100);

    let tamanos: Vec<u64> = res
        .entry_ids
        .iter()
        .map(|&id| view.size_of(id as usize))
        .collect();

    let mut esperado: Vec<u64> = casos.iter().map(|c| c.1).collect();
    esperado.sort_unstable();
    assert_eq!(
        tamanos, esperado,
        "los archivos de más de 4 GiB tienen que ordenarse por su tamaño real"
    );

    // Y al revés.
    let motor2 = Engine::new();
    let (_g, desc) = motor2.search_with_limit(&view, "bin", 3, false, 100);
    let tamanos_desc: Vec<u64> = desc
        .entry_ids
        .iter()
        .map(|&id| view.size_of(id as usize))
        .collect();
    esperado.reverse();
    assert_eq!(tamanos_desc, esperado);
}

#[test]
fn el_filtro_de_tamano_encuentra_los_enormes() {
    // `tam:>4gb` tiene que ver el tamaño real, no la marca.
    let casos: Vec<(&str, u64)> = vec![
        ("chico.wav", 1024),
        ("mediano.wav", 100 * 1024 * 1024),
        ("enorme.wav", 8 * GIB),
        ("gigante.wav", 40 * GIB),
    ];
    let (_t, mmap) = indice_con(&casos);
    let view = mmap.view().unwrap();
    let motor = Engine::new();

    let (_g, res) = motor.search_with_limit(&view, "tam:>4gb", 0, true, 100);
    let nombres: Vec<&str> = res
        .entry_ids
        .iter()
        .map(|&id| view.get_name(id as usize).unwrap_or(""))
        .collect();
    assert_eq!(nombres, vec!["enorme.wav", "gigante.wav"]);
}

#[test]
fn un_archivo_que_encoge_sale_de_la_tabla_aparte() {
    // La fase 2 del escaneo rellena los tamaños después de crear las entradas, y
    // un archivo puede pasar de grande a pequeño entre dos publicaciones. Si al
    // encoger se quedara en la tabla, arrastraría su tamaño viejo para siempre.
    let mut b = IndexBuilder::with_capacity(4);
    b.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);
    let raiz = b.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
    let id = b.add_entry(raiz, "cambiante.bin", false, false, false, 0, 9 * GIB, 0, 0);
    assert_eq!(b.size_at(id), 9 * GIB);

    b.set_metadata(id, 4096, 0, 0);
    assert_eq!(
        b.size_at(id),
        4096,
        "al encoger tiene que dejar de leerse de la tabla aparte"
    );

    // Y al revés: de pequeño a enorme.
    b.set_metadata(id, 30 * GIB, 0, 0);
    assert_eq!(b.size_at(id), 30 * GIB);

    let temp = NamedTempFile::new().unwrap();
    let f = std::fs::File::create(temp.path()).unwrap();
    let mut w = BufWriter::new(f);
    b.write_to(&mut w).unwrap();
    drop(w);
    let mmap = MmapIndex::open(temp.path()).unwrap();
    assert_eq!(mmap.view().unwrap().size_of(id as usize), 30 * GIB);
}

#[test]
fn un_indice_sin_archivos_enormes_no_gasta_nada() {
    // El caso normal: la tabla aparte queda vacía y no cuesta ni un byte de más
    // por entrada.
    let casos: Vec<(&str, u64)> = (0..100)
        .map(|i| ("pista.wav", (i as u64) * 1024))
        .collect();
    let (_t, mmap) = indice_con(&casos);
    let view = mmap.view().unwrap();
    assert!(
        view.size_big_id.is_empty(),
        "sin archivos de 4 GiB la tabla aparte debe estar vacía"
    );
    for i in 0..100usize {
        assert_eq!(view.size_of(i + 1), (i as u64) * 1024);
    }
}
