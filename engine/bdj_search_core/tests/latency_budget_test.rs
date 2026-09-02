//! Presupuesto de latencia sobre un índice grande.
//!
//! Marcado `#[ignore]` porque construye un corpus de varios millones de
//! entradas y solo tiene sentido en modo release. Para ejecutarlo:
//!
//! ```text
//! cargo test --release -p bdj_search_core --test latency_budget_test -- --ignored --nocapture
//! ```
//!
//! Las cifras que imprime dependen del número de núcleos de la máquina. Lo que
//! el test **afirma** es lo que no debe depender de la máquina:
//!
//! - refinar tras la tercera letra cuesta una fracción del recorrido completo;
//! - el conteo total es exacto aunque solo se devuelvan las primeras filas;
//! - ordenar por nombre no depende del tamaño del índice.

use bdj_search_core::index::{IndexBuilder, MmapIndex};
use bdj_search_core::search::Engine;
use std::io::BufWriter;
use std::time::Instant;
use tempfile::NamedTempFile;

const ARTISTS: [&str; 8] = [
    "Michael Jackson",
    "Madonna",
    "Daft Punk",
    "David Guetta",
    "Calvin Harris",
    "Carl Cox",
    "Fisher",
    "Bicep",
];
const TRACKS: [&str; 8] = [
    "Billie Jean",
    "Titanium",
    "Levels",
    "Animals",
    "Ferrari",
    "Strobe",
    "Gecko",
    "Cinema",
];
const EXTS: [&str; 5] = ["wav", "mp3", "flac", "als", "zip"];

fn build(count: usize) -> (NamedTempFile, MmapIndex) {
    let mut builder = IndexBuilder::with_capacity(count);
    builder.vol_table.add_or_update("C:\\", "bench", "NTFS", true);
    for i in 0..count {
        let name = format!(
            "{} - {} ({}).{}",
            ARTISTS[i % ARTISTS.len()],
            TRACKS[(i / ARTISTS.len()) % TRACKS.len()],
            i,
            EXTS[i % EXTS.len()]
        );
        builder.add_entry(
            u32::MAX,
            &name,
            false,
            false,
            false,
            0,
            (i as u64 % 500) * 1024 * 1024,
            1_700_000_000 + (i as u32 % 1000),
            1_700_000_000,
        );
    }
    let temp = NamedTempFile::new().unwrap();
    let file = std::fs::File::create(temp.path()).unwrap();
    let mut w = BufWriter::new(file);
    builder.write_to(&mut w).unwrap();
    drop(w);
    let mmap = MmapIndex::open(temp.path()).unwrap();
    (temp, mmap)
}

#[test]
#[ignore = "construye un corpus de 10 millones de entradas; ejecutar en release"]
fn presupuesto_de_latencia_sobre_diez_millones() {
    let count = 10_000_000usize;
    let t0 = Instant::now();
    let (_temp, mmap) = build(count);
    println!("corpus de {count} entradas construido en {:?}", t0.elapsed());

    let t1 = Instant::now();
    let view = mmap.view().unwrap();
    let abrir = t1.elapsed();
    println!("mmap abierto en {abrir:?}");
    assert!(
        abrir.as_millis() < 50,
        "abrir el índice debe ser instantáneo, tardó {abrir:?}"
    );

    let engine = Engine::new();

    // Primera pulsación: recorrido completo.
    let t = Instant::now();
    let (_g, fria) = engine.search(&view, "m", 0, true);
    let t_fria = t.elapsed();
    println!(
        "«m» -> {} coincidencias en {t_fria:?} (ventana de {} filas)",
        fria.total_count,
        fria.entry_ids.len()
    );

    // El conteo es exacto aunque la ventana esté recortada.
    assert!(fria.total_count > 1_000_000);
    assert!(fria.truncated);
    assert!(
        fria.entry_ids.len() <= 2_000,
        "la ventana no debe materializar millones de filas, tiene {}",
        fria.entry_ids.len()
    );

    // Secuencia de tecleo.
    let mut ultima = t_fria;
    for q in ["mi", "mic", "mich", "micha", "michae", "michael"] {
        let t = Instant::now();
        let (_g, res) = engine.search(&view, q, 0, true);
        let dt = t.elapsed();
        println!("«{q}» -> {} coincidencias en {dt:?}", res.total_count);
        ultima = dt;
    }
    assert!(
        ultima < t_fria / 4,
        "la última pulsación ({ultima:?}) debería costar una fracción del recorrido completo ({t_fria:?})"
    );

    // Multipalabra: el caso real de artista + título.
    let engine2 = Engine::new();
    let t = Instant::now();
    let (_g, _) = engine2.search(&view, "michael billie", 0, true);
    let t_multi_fria = t.elapsed();
    let t = Instant::now();
    let (_g, res) = engine2.search(&view, "michael billie j", 0, true);
    let t_multi_refinada = t.elapsed();
    println!(
        "«michael billie j» -> {} coincidencias en {t_multi_refinada:?} (frío: {t_multi_fria:?})",
        res.total_count
    );
    assert!(
        t_multi_refinada < t_multi_fria,
        "teclear dentro de una consulta de varias palabras debe refinar, no recorrer de nuevo"
    );

    // Ordenar por nombre un resultado pequeño no puede depender del tamaño del
    // índice.
    let engine3 = Engine::new();
    let t = Instant::now();
    let (_g, pocos) = engine3.search(&view, "michael billie (7).wav", 0, true);
    println!(
        "consulta muy selectiva -> {} coincidencias en {:?}",
        pocos.total_count,
        t.elapsed()
    );

    // Ordenar por tamaño y por fecha usa el radix, no comparaciones.
    for (col, nombre) in [(3u8, "tamaño"), (4u8, "fecha")] {
        let e = Engine::new();
        let t = Instant::now();
        let (_g, res) = e.search(&view, "m", col, false);
        println!(
            "«m» ordenado por {nombre} descendente -> {} coincidencias en {:?}",
            res.total_count,
            t.elapsed()
        );
        assert_eq!(res.entry_ids.len(), 2_000);
    }
}
