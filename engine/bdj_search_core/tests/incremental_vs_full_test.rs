use bdj_search_core::index::{IndexBuilder, MmapIndex};
use bdj_search_core::search::Engine;
use std::io::BufWriter;
use std::time::Instant;
use tempfile::NamedTempFile;

#[test]
fn test_incremental_refinement_matches_full_scan() {
    let count = 50_000usize;
    let mut builder = IndexBuilder::with_capacity(count);

    let artists = ["Michael", "Madonna", "Daft", "David", "Calvin", "Tiësto", "Fisher"];
    let titles = ["Billie", "Titanium", "Levels", "Animals", "Ferrari", "Strobe", "Gecko"];
    let exts = ["wav", "mp3", "flac", "als", "zip"];

    for i in 0..count {
        let artist = artists[i % artists.len()];
        let title = titles[(i / artists.len()) % titles.len()];
        let ext = exts[i % exts.len()];
        let is_remix = (i % 3) == 0;
        let remix_tag = if is_remix { " Remix" } else { "" };
        let name = format!("{} - {}{}.{}", artist, title, remix_tag, ext);
        builder.add_entry(u32::MAX, &name, false, false, false, 0, (i as u64) * 512, 1700000000, 1700000000);
    }

    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    let file = std::fs::File::create(path).unwrap();
    let mut writer = BufWriter::new(file);
    builder.write_to(&mut writer).unwrap();
    drop(writer);

    let mmap_index = MmapIndex::open(path).unwrap();
    let view = mmap_index.view().unwrap();

    let engine = Engine::new();

    // Keystroke sequence: "m" -> "mi" -> "mic" -> "mich" -> "micha" -> "michae" -> "michael"
    let keystrokes = ["m", "mi", "mic", "mich", "micha", "michae", "michael"];

    for query in keystrokes {
        let start = Instant::now();
        let (_gen, refined_res) = engine.search(&view, query, 0, true);
        let elapsed_refined = start.elapsed();

        // Perform independent full scan with fresh engine
        let fresh_engine = Engine::new();
        let inicio_completo = Instant::now();
        let (_gen2, full_res) = fresh_engine.search(&view, query, 0, true);
        let elapsed_full = inicio_completo.elapsed();

        // Acceptance check 1: Refinement MUST produce the exact same set of results as full scan
        let mut refined_sorted = refined_res.entry_ids.clone();
        let mut full_sorted = full_res.entry_ids.clone();
        refined_sorted.sort_unstable();
        full_sorted.sort_unstable();

        assert_eq!(
            refined_sorted, full_sorted,
            "Query '{}': Refinement set does not match full scan set!",
            query
        );

        println!(
            "Query '{}' -> {} matches | Refinement time: {:?} | Full scan time: {:?}",
            query,
            refined_res.total_count,
            elapsed_refined,
            elapsed_full
        );

        // Comprobación 2: el refinamiento tiene que ser competitivo o más barato
        // que rehacer el recorrido completo.
        //
        // En runners virtuales compartidos (CI) y mediciones a escala de microsegundos,
        // la caché caliente y el jitter del planificador del SO pueden causar variaciones
        // menores de 1-2 ms. Se añade un margen de tolerancia para evitar falsos positivos.
        if query != "m" {
            let jitter_tolerance = std::time::Duration::from_millis(10);
            assert!(
                elapsed_refined <= elapsed_full + jitter_tolerance,
                "la consulta «{query}» tardó {elapsed_refined:?} refinando y \
                 {elapsed_full:?} recorriendo entero (excediendo tolerancia de {jitter_tolerance:?}): refinar debería salir más barato"
            );
        }
    }

    println!("Progressive keystroke refinement verified successfully within budgets!");
}

#[test]
fn test_query_functions_evaluation() {
    let mut builder = IndexBuilder::new();
    // 0: Audio WAV 50MB
    builder.add_entry(u32::MAX, "Bassline.wav", false, false, false, 0, 50 * 1024 * 1024, 1700000000, 1700000000);
    // 1: Audio MP3 8MB Remix
    builder.add_entry(u32::MAX, "Vocal Remix.mp3", false, false, false, 0, 8 * 1024 * 1024, 1700000000, 1700000000);
    // 2: Project ALS 2MB
    builder.add_entry(u32::MAX, "ClubSet.als", false, false, false, 0, 2 * 1024 * 1024, 1700000000, 1700000000);
    // 3: Folder
    builder.add_entry(u32::MAX, "Samples", true, false, false, 0, 0, 1700000000, 1700000000);

    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    let file = std::fs::File::create(path).unwrap();
    let mut writer = BufWriter::new(file);
    builder.write_to(&mut writer).unwrap();
    drop(writer);

    let mmap_index = MmapIndex::open(path).unwrap();
    let view = mmap_index.view().unwrap();
    let engine = Engine::new();

    // Test ext:
    let (_, res) = engine.search(&view, "ext:wav", 0, true);
    assert_eq!(res.entry_ids, vec![0]);

    // Test size:
    let (_, res_size) = engine.search(&view, "tam:>20mb", 0, true);
    assert_eq!(res_size.entry_ids, vec![0]);

    // Test NOT:
    let (_, res_not) = engine.search(&view, "!remix type:audio", 0, true);
    assert_eq!(res_not.entry_ids, vec![0]); // Bassline.wav (no remix)

    // Test DJ Projects type:
    let (_, res_dj) = engine.search(&view, "type:dj", 0, true);
    assert_eq!(res_dj.entry_ids, vec![2]); // ClubSet.als

    // Test folder only:
    let (_, res_folder) = engine.search(&view, "folder:", 0, true);
    assert_eq!(res_folder.entry_ids, vec![3]); // Samples folder

    println!("All query filter functions verified successfully!");
}
