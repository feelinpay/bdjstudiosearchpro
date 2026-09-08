//! Qué consume el motor, no solo cuánto tarda.
//!
//! Los bancos anteriores miden latencias de búsqueda. Eso dice si el producto es
//! rápido en la máquina donde se mide, pero no dice si **cabe** en una máquina
//! modesta, que es lo que decide si funciona o no en un portátil de 4 GB.
//!
//! Aquí se mide lo otro: memoria pico durante la construcción del índice, bytes
//! por entrada en disco, y el coste que la capa de cambios añade a cada
//! pulsación de tecla.
//!
//! Uso:
//! ```text
//! cargo run --release -p bdj_search_core --example bench_footprint -- 2000000
//! ```

use bdj_search_core::index::{
    IndexBuilder, MmapIndex, OverlayIndex, OverlaySnapshot, write_overlay,
};
use bdj_search_core::search::{Engine, search_merged};
use std::io::BufWriter;
use std::path::Path;
use std::time::Instant;

/// Memoria máxima que ha llegado a ocupar este proceso, en MB.
///
/// `VmHWM` es la marca de agua: el pico real, no el uso actual. Es la cifra que
/// importa, porque es la que decide si el sistema empieza a paginar.
fn pico_memoria_mb() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(texto) = std::fs::read_to_string("/proc/self/status") {
            for linea in texto.lines() {
                if let Some(resto) = linea.strip_prefix("VmHWM:")
                    && let Some(kb) = resto.split_whitespace().next()
                    && let Ok(v) = kb.parse::<u64>()
                {
                    return v / 1024;
                }
            }
        }
    }
    0
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn construir(count: usize, path: &Path) -> (u64, f64) {
    const ARTISTAS: [&str; 8] = [
        "Michael", "Madonna", "Daft Punk", "David Guetta", "Calvin Harris", "Tiesto", "Fisher",
        "Charlotte de Witte",
    ];
    const TITULOS: [&str; 8] = [
        "Billie Jean", "Titanium", "Levels", "Animals", "Ferrari", "Strobe", "Gecko", "Doppler",
    ];
    const EXTS: [&str; 8] = ["wav", "mp3", "flac", "als", "aiff", "mp4", "jpg", "zip"];

    let inicio = Instant::now();
    let mut b = IndexBuilder::with_capacity(count);
    b.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);
    let raiz = b.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);

    // Un árbol realista: unas 2000 carpetas, no todo colgando de la raíz.
    let mut carpetas = Vec::with_capacity(2048);
    for i in 0..2048 {
        carpetas.push(b.add_entry(
            raiz,
            &format!("Set {i:04}"),
            true,
            false,
            false,
            0,
            0,
            1_700_000_000,
            1_700_000_000,
        ));
    }

    for i in 0..count {
        let padre = carpetas[i % carpetas.len()];
        let nombre = format!(
            "{} - {} {}.{}",
            ARTISTAS[i % ARTISTAS.len()],
            TITULOS[(i / 8) % TITULOS.len()],
            i,
            EXTS[i % EXTS.len()]
        );
        b.add_entry(
            padre,
            &nombre,
            false,
            false,
            false,
            0,
            ((i % 5000) as u64) * 1024,
            1_700_000_000 + (i % 100_000) as u32,
            1_700_000_000,
        );
    }
    let construido = inicio.elapsed();
    let pico_construccion = pico_memoria_mb();

    // Desglose: ¿el coste de publicar es CPU (ordenar nombres) o disco?
    let t_orden = Instant::now();
    b.compute_name_order();
    let orden = t_orden.elapsed();

    let inicio_escritura = Instant::now();
    let f = std::fs::File::create(path).unwrap();
    let mut w = BufWriter::with_capacity(4 * 1024 * 1024, f);
    let escritos = b.write_to(&mut w).unwrap();
    drop(w);
    let escrito = inicio_escritura.elapsed();

    let pico_total = pico_memoria_mb();
    let entradas = b.count() as u64;

    println!("=== CONSTRUCCIÓN ===");
    println!("  entradas            {entradas}");
    println!("  en memoria          {construido:?}");
    println!("  ordenar nombres     {orden:?}");
    println!("  escritura a disco   {escrito:?}  (el orden ya estaba hecho)");
    println!("  archivo             {:.1} MB", mb(escritos as u64));
    println!(
        "  bytes por entrada   {:.1} B",
        escritos as f64 / entradas as f64
    );
    println!("  PICO DE MEMORIA     {pico_construccion} MB (tras escribir: {pico_total} MB)");
    println!(
        "  memoria por entrada {:.1} B",
        (pico_construccion * 1024 * 1024) as f64 / entradas as f64
    );

    // Desglose real de lo que ocupa el constructor, para no optimizar a ciegas.
    let columnas = entradas
        * (4  // parents
            + 4  // name_offs
            + 1  // name_lens
            + 1  // flags
            + 2  // ext_ids
            + 1  // volumes
            + 4  // sizes (u32 desde la versión 3)
            + 4  // mtimes
            + 4); // ctimes
    let arena = b.arena.len() as u64;
    println!("\n  --- de dónde viene el pico ---");
    println!(
        "  columnas            {:>7.1} MB  ({:.1} B/entrada)",
        mb(columnas),
        columnas as f64 / entradas as f64
    );
    println!(
        "  arena de nombres    {:>7.1} MB  ({:.1} B/entrada)  {:.0}% del total",
        mb(arena),
        arena as f64 / entradas as f64,
        100.0 * arena as f64 / (columnas + arena) as f64
    );
    println!(
        "  de las columnas, ctime (lo usa el filtro «creado:»): {:.1} MB",
        mb(entradas * 4)
    );

    (
        pico_construccion,
        escritos as f64 / entradas as f64,
    )
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let count: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);

    let dir = std::env::temp_dir().join("bdj_footprint");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("corpus_{count}.bdjx"));

    let t = Tuning();
    println!(
        "Máquina: {} núcleos, {} MB, gama {}\n",
        t.cores,
        t.memory_mb,
        t.tier.name()
    );

    let (pico, _bytes_entrada) = construir(count, &path);

    let mmap = MmapIndex::open(&path).unwrap();
    let view = mmap.view().unwrap();

    // --- Coste de la capa de cambios en cada consulta ---
    println!("\n=== BÚSQUEDA: SIN CAPA vs CON CAPA ===");

    let capa_path = dir.join("overlay.bdjo");
    let mut capa = OverlayIndex::with_base_count(view.entry_count() as u32);
    capa.builder.vol_table = view.vol_table.clone();
    // Un escenario realista: se acaban de copiar 200 archivos y borrar 50.
    for i in 0..200 {
        capa.add_entry(1, &format!("recien {i}.wav"), false, false, false, 0, 1, 0, 0);
    }
    for i in 0..50u32 {
        capa.mark_deleted(3000 + i * 7);
    }
    write_overlay(&mut capa, view.header.generation, 1, &capa_path).unwrap();
    let snap = OverlaySnapshot::open(&capa_path, view.header.generation).unwrap();
    let oi = snap.to_overlay_index(Some(&mmap));

    let consultas: [(&str, u8); 5] = [
        ("m", 0),
        ("michael", 0),
        ("ext:wav", 0),
        ("michael billie", 0),
        ("m", 3),
    ];

    println!(
        "{:<20} {:>12} {:>12} {:>10} {:>12}",
        "consulta", "sin capa", "con capa", "sobrecoste", "resultados"
    );
    for (q, col) in consultas {
        let m1 = Engine::new();
        let t1 = Instant::now();
        let (_g, r1) = m1.search_with_limit(&view, q, col, true, 2000);
        let sin = t1.elapsed();

        let m2 = Engine::new();
        let t2 = Instant::now();
        let (_g, r2) = search_merged(
            &m2,
            &view,
            Some(&snap),
            oi.as_ref(),
            q,
            "",
            col,
            true,
            2000,
        );
        let con = t2.elapsed();

        let sobrecoste = con.as_secs_f64() / sin.as_secs_f64().max(1e-9);
        println!(
            "{:<20} {:>10.1?} {:>10.1?} {:>9.2}x {:>12}",
            q, sin, con, sobrecoste, r2.total_count
        );
        assert!(r2.total_count >= r1.total_count.saturating_sub(60));
    }

    // --- Coste de reconstruir el índice unificado en cada republicación ---
    println!("\n=== RECARGA DE LA CAPA ===");
    let t = Instant::now();
    for _ in 0..20 {
        let s = OverlaySnapshot::open(&capa_path, view.header.generation).unwrap();
        std::hint::black_box(s.to_overlay_index(Some(&mmap)));
    }
    println!(
        "  abrir + reconstruir  {:?} por vez ({} entradas en la capa)",
        t.elapsed() / 20,
        snap.entry_count()
    );

    println!("\n=== RESUMEN ===");
    println!("  pico de memoria construyendo: {pico} MB para {count} entradas");
    println!(
        "  proyección a 10 M de archivos: {:.0} MB",
        pico as f64 * (10_000_000.0 / count as f64)
    );
    let _ = std::fs::remove_file(&capa_path);
}

#[allow(non_snake_case)]
fn Tuning() -> &'static bdj_search_core::tuning::Tuning {
    bdj_search_core::tuning::Tuning::current()
}
