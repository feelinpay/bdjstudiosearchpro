//! Banco de pruebas de latencia sobre un corpus sintético grande.
//!
//! Construye —o reutiliza— un índice de N entradas con la misma forma que
//! produce el indexador real y mide el tiempo de las consultas que más duelen:
//! una sola letra, un filtro de extensión, un grupo de tipos y un filtro
//! puramente numérico.
//!
//! Uso:
//!   cargo run --release -p bdj_search_core --example bench_corpus -- 10000000 /tmp/corpus.bdjx
//!
//! El presupuesto del producto es 30 ms para la primera pulsación y 8 ms para
//! cada refinamiento posterior, sobre 10 millones de entradas.

use bdj_search_core::index::{IndexBuilder, MmapIndex};
use bdj_search_core::search::Engine;
use std::io::BufWriter;
use std::path::Path;
use std::time::Instant;

const ARTISTS: [&str; 25] = [
    "Michael Jackson",
    "Daft Punk",
    "David Guetta",
    "Tiesto",
    "Calvin Harris",
    "Carl Cox",
    "Armin van Buuren",
    "Deadmau5",
    "Fisher",
    "Skrillex",
    "Avicii",
    "Martin Garrix",
    "Swedish House Mafia",
    "Charlotte de Witte",
    "Peggy Gou",
    "Fred again",
    "Bicep",
    "Disclosure",
    "James Hype",
    "ACRAZE",
    "Bad Bunny",
    "J Balvin",
    "Daddy Yankee",
    "Bizarrap",
    "Rauw Alejandro",
];

const TRACKS: [&str; 25] = [
    "Billie Jean",
    "One More Time",
    "Titanium",
    "Levels",
    "Animals",
    "Dont You Worry Child",
    "Losing It",
    "Ferrari",
    "Do It To It",
    "Stay The Night",
    "Midnight City",
    "Strobe",
    "Adagio for Strings",
    "Satisfaction",
    "Gecko",
    "Show Me Love",
    "Barbra Streisand",
    "Turn Off The Lights",
    "Cinema",
    "Reload",
    "Pepas",
    "Gasolina",
    "Titi Me Pregunto",
    "Provenza",
    "Session 52",
];

const MIXES: [&str; 10] = [
    "Original Mix",
    "Extended Mix",
    "Club Mix",
    "Dub Mix",
    "Radio Edit",
    "VIP Remix",
    "Acapella",
    "Instrumental",
    "Remastered",
    "Live 2026",
];

const AUDIO_EXTS: [&str; 5] = ["wav", "mp3", "flac", "aiff", "m4a"];
const OTHER_EXTS: [&str; 6] = ["als", "flp", "zip", "pdf", "jpg", "mp4"];

fn build_corpus(count: usize, path: &Path) {
    println!("Construyendo corpus de {count} entradas en {}...", path.display());
    let t0 = Instant::now();
    let mut builder = IndexBuilder::with_capacity(count);
    builder.vol_table.add_or_update("C:\\", "corpus", "NTFS", true);

    // Las primeras 1000 entradas son carpetas y sirven de padres.
    let folders = 1000.min(count);
    for i in 0..folders {
        builder.add_entry(
            u32::MAX,
            &format!("Carpeta_{}_{}", ARTISTS[i % ARTISTS.len()], i),
            true,
            false,
            false,
            0,
            0,
            1_787_961_600,
            1_787_875_200,
        );
    }

    let base_time: u32 = 1_787_961_600;
    for i in folders..count {
        let artist = ARTISTS[i % ARTISTS.len()];
        let track = TRACKS[(i / ARTISTS.len()) % TRACKS.len()];
        let mix = MIXES[(i / 7) % MIXES.len()];
        let ext = if (i % 10) < 8 {
            AUDIO_EXTS[i % AUDIO_EXTS.len()]
        } else {
            OTHER_EXTS[i % OTHER_EXTS.len()]
        };
        let name = format!("{artist} - {track} ({mix}) [DJ Rip].{ext}");
        let size = 1024 * 1024 * ((i % 50) as u64 + 3);
        let mtime = base_time.saturating_sub((i % 100_000) as u32 * 60);
        builder.add_entry(
            (i % folders.max(1)) as u32,
            &name,
            false,
            false,
            false,
            0,
            size,
            mtime,
            mtime.saturating_sub(86_400),
        );
    }
    println!("  estructuras en memoria: {:?}", t0.elapsed());

    let t1 = Instant::now();
    let file = std::fs::File::create(path).expect("no se pudo crear el corpus");
    let mut writer = BufWriter::with_capacity(4 << 20, file);
    let bytes = builder.write_to(&mut writer).expect("no se pudo escribir");
    drop(writer);
    println!(
        "  escrito: {:.1} MB en {:?} ({:.1} bytes/entrada)",
        bytes as f64 / 1e6,
        t1.elapsed(),
        bytes as f64 / count as f64
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let count: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10_000_000);
    let path_str = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| format!("corpus_{count}.bdjx"));
    let path = Path::new(&path_str);

    if !path.exists() {
        build_corpus(count, path);
    } else {
        println!("Reutilizando corpus existente: {}", path.display());
    }

    let t = Instant::now();
    let mmap = MmapIndex::open(path).expect("no se pudo mapear el corpus");
    let view = mmap.view().expect("formato de índice inválido");
    println!(
        "\nmmap abierto en {:?} · {} entradas\n",
        t.elapsed(),
        view.entry_count()
    );

    // Consultas frías: cada una con un motor nuevo, sin pila de refinamiento.
    let cold: [(&str, u8); 7] = [
        ("m", 0),
        ("ext:wav", 0),
        ("tipo:audio", 0),
        ("tam:>10mb", 0),
        ("michael", 0),
        ("m", 3),
        ("m", 4),
    ];

    println!("{:<16} {:>6} {:>14} {:>12}", "consulta", "orden", "resultados", "tiempo");
    println!("{}", "-".repeat(52));
    for (q, sort_col) in cold {
        let engine = Engine::new();
        let start = Instant::now();
        let (_g, res) = engine.search(&view, q, sort_col, true);
        let elapsed = start.elapsed();
        println!(
            "{:<16} {:>6} {:>14} {:>10.1} ms",
            q,
            sort_col,
            res.total_count,
            elapsed.as_secs_f64() * 1000.0
        );
    }

    // Secuencia de tecleo: mide el refinamiento incremental sobre un solo motor.
    println!("\nSecuencia de tecleo (refinamiento incremental):");
    println!("{:<16} {:>14} {:>12}", "consulta", "resultados", "tiempo");
    println!("{}", "-".repeat(44));
    let engine = Engine::new();
    for q in ["m", "mi", "mic", "mich", "micha", "michae", "michael"] {
        let start = Instant::now();
        let (_g, res) = engine.search(&view, q, 0, true);
        let elapsed = start.elapsed();
        println!(
            "{:<16} {:>14} {:>10.1} ms",
            q,
            res.total_count,
            elapsed.as_secs_f64() * 1000.0
        );
    }

    // Multipalabra: el caso real de "artista + título".
    println!("\nSecuencia multipalabra:");
    println!("{:<24} {:>14} {:>12}", "consulta", "resultados", "tiempo");
    println!("{}", "-".repeat(52));
    let engine = Engine::new();
    for q in [
        "michael",
        "michael b",
        "michael bi",
        "michael bil",
        "michael bill",
        "michael billie",
    ] {
        let start = Instant::now();
        let (_g, res) = engine.search(&view, q, 0, true);
        let elapsed = start.elapsed();
        println!(
            "{:<24} {:>14} {:>10.1} ms",
            q,
            res.total_count,
            elapsed.as_secs_f64() * 1000.0
        );
    }
}
