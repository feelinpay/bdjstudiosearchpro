use bdj_search_core::index::{IndexBuilder, MmapIndex, OverlayIndex};
use rand::Rng;
use std::io::BufWriter;
use std::time::Instant;
use tempfile::NamedTempFile;

#[test]
fn test_index_build_mmap_and_path_resolution() {
    // El corpus reproduce la forma real de un volumen: una raiz sin nombre, las
    // carpetas colgando de ella y los archivos dentro. Modelar las carpetas como
    // raices, que es lo que hacia la version anterior de esta prueba, ocultaba
    // que las rutas salian mal.
    let count = 100_000usize;
    let mount = if cfg!(windows) { "D:\\" } else { "/Volumes/Music" };
    let sep = if mount.contains('\\') { '\\' } else { '/' };
    // "D:\\" ya termina en separador; "/Volumes/Music" no. El prefijo de montaje
    // puede venir de cualquiera de las dos formas segun la plataforma.
    let join = |base: &str, seg: &str| -> String {
        if base.ends_with('\\') || base.ends_with('/') {
            format!("{base}{seg}")
        } else {
            format!("{base}{sep}{seg}")
        }
    };

    let mut builder = IndexBuilder::with_capacity(count);
    builder.vol_table.add_or_update(mount, "SSD Musica", "NTFS", true);

    let root = builder.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
    assert_eq!(root, 0);

    // 100 carpetas bajo la raiz del volumen: indices 1..=100
    for i in 0..100 {
        builder.add_entry(
            root,
            &format!("Dir_{}", i),
            true,
            false,
            false,
            0,
            0,
            1_700_000_000 + i as u32,
            1_700_000_000,
        );
    }

    let first_file = 101usize;
    let mut expected_names = Vec::with_capacity(count);
    let mut expected_parents = Vec::with_capacity(count);
    let mut expected_sizes = Vec::with_capacity(count);

    for i in first_file..count {
        let parent_id = 1 + ((i - first_file) % 100) as u32;
        let name = format!("Track_{:06}.wav", i);
        let size = (i as u64) * 1024;
        expected_names.push(name.clone());
        expected_parents.push(parent_id);
        expected_sizes.push(size);

        builder.add_entry(
            parent_id,
            &name,
            false,
            false,
            false,
            0,
            size,
            1_700_000_000 + (i % 1000) as u32,
            1_700_000_000,
        );
    }

    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    let write_start = Instant::now();
    let file = std::fs::File::create(path).unwrap();
    let mut writer = BufWriter::with_capacity(2 * 1024 * 1024, file);
    builder.write_to(&mut writer).unwrap();
    drop(writer);
    println!("Escritas {} entradas en {:?}", count, write_start.elapsed());

    let open_start = Instant::now();
    let mmap_index = MmapIndex::open(path).expect("MmapIndex::open deberia funcionar");
    let view = mmap_index.view().expect("view() deberia validar el formato");
    let open_time = open_start.elapsed();
    println!("Apertura mmap + validacion: {:?}", open_time);
    assert!(
        open_time.as_millis() < 100,
        "La apertura del indice debe ser < 100 ms, tomo {:?}",
        open_time
    );

    assert_eq!(view.entry_count(), count);

    // La raiz del volumen resuelve exactamente al prefijo de montaje, sin
    // repetirlo como segmento.
    assert_eq!(view.resolve_full_path(0), mount);
    assert_eq!(
        view.resolve_full_path(1),
        join(mount, "Dir_0"),
        "una carpeta de primer nivel no debe duplicar el prefijo del volumen"
    );

    let mut rng = rand::thread_rng();
    for _ in 0..1000 {
        let idx = rng.gen_range(first_file..count);
        let exp_name = &expected_names[idx - first_file];
        let exp_size = expected_sizes[idx - first_file];
        let exp_parent = expected_parents[idx - first_file];

        assert_eq!(view.get_name(idx), Some(exp_name.as_str()));
        assert_eq!(view.get_extension(idx), "wav");
        assert_eq!(view.size[idx], exp_size);
        assert_eq!(view.parent[idx], exp_parent);

        // Igualdad exacta, no `contains`: es lo que detecta un prefijo repetido.
        let parent_path = join(mount, &format!("Dir_{}", exp_parent - 1));
        let expected_path = format!("{}{}{}", parent_path, sep, exp_name);
        assert_eq!(
            view.resolve_full_path(idx),
            expected_path,
            "ruta completa incorrecta para la entrada {}",
            idx
        );
        assert_eq!(
            view.resolve_parent_path(idx),
            parent_path,
            "ruta de la carpeta contenedora incorrecta para la entrada {}",
            idx
        );
    }

    println!("Muestra aleatoria de 1.000 entradas verificada.");
}

#[test]
fn test_path_resolution_survives_corrupted_parents() {
    // Un indice con un ciclo en la columna `parent` no debe colgar el proceso.
    let mut builder = IndexBuilder::new();
    builder.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);

    let root = builder.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
    let a = builder.add_entry(root, "A", true, false, false, 0, 0, 0, 0);
    let b = builder.add_entry(a, "B", true, false, false, 0, 0, 0, 0);
    // Ciclo deliberado: A pasa a colgar de B.
    builder.parents[a as usize] = b;

    let temp_file = NamedTempFile::new().unwrap();
    let file = std::fs::File::create(temp_file.path()).unwrap();
    let mut writer = BufWriter::new(file);
    builder.write_to(&mut writer).unwrap();
    drop(writer);

    let mmap_index = MmapIndex::open(temp_file.path()).unwrap();
    let view = mmap_index.view().unwrap();

    // Basta con que termine: sin el tope de profundidad esto no retornaria.
    let resolved = view.resolve_full_path(b as usize);
    assert!(resolved.len() < 4096, "la ruta resuelta debe estar acotada");
}

#[test]
fn test_set_parent_rejects_self_reference() {
    let mut builder = IndexBuilder::new();
    let root = builder.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
    let a = builder.add_entry(root, "A", true, false, false, 0, 0, 0, 0);

    // El registro raiz de NTFS se declara padre de si mismo.
    builder.set_parent(a, a);
    assert_eq!(
        builder.parent_of(a),
        u32::MAX,
        "una entrada nunca debe quedar como padre de si misma"
    );
}

#[test]
fn test_truncate_undoes_a_partial_scan() {
    let mut builder = IndexBuilder::new();
    let root = builder.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
    let base = builder.count() as u32;

    for i in 0..200 {
        builder.add_entry(root, &format!("f{}.wav", i), false, false, false, 0, 0, 0, 0);
    }
    assert_eq!(builder.count(), 201);

    builder.truncate_to(base);
    assert_eq!(builder.count(), 1, "solo debe quedar la raiz");

    // El indice truncado sigue siendo valido y no arrastra entradas vivas.
    let temp_file = NamedTempFile::new().unwrap();
    let file = std::fs::File::create(temp_file.path()).unwrap();
    let mut writer = BufWriter::new(file);
    builder.write_to(&mut writer).unwrap();
    drop(writer);

    let mmap_index = MmapIndex::open(temp_file.path()).unwrap();
    let view = mmap_index.view().unwrap();
    assert_eq!(view.entry_count(), 1);
    assert!(view.is_alive(0));
}

#[test]
fn test_metadata_is_filled_in_a_second_pass() {
    // Reproduce la fase 2: la MFT deja tamano y fechas en cero y el recorrido
    // de directorios los completa despues.
    let mut builder = IndexBuilder::new();
    builder.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);

    let root = builder.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
    let dir = builder.add_entry(root, "Music", true, false, false, 0, 0, 0, 0);
    let file = builder.add_entry(dir, "set.wav", false, false, false, 0, 0, 0, 0);

    assert_eq!(builder.sizes[file as usize], 0);
    builder.set_metadata(file, 52_428_800, 1_755_000_000, 1_754_000_000);
    builder.set_attributes(file, true, false);

    assert_eq!(builder.sizes[file as usize], 52_428_800);
    assert_eq!(builder.mtimes[file as usize], 1_755_000_000);
    assert_eq!(builder.resolve_path(file, "C:\\"), "C:\\Music\\set.wav");

    // Agrupado por carpeta: es como la fase 2 recorre cada directorio una vez.
    let pairs = builder.children_by_parent();
    assert!(pairs.contains(&(dir, file)));
    assert!(pairs.contains(&(root, dir)));
    assert!(
        !pairs.iter().any(|&(p, _)| p == u32::MAX),
        "la raiz del volumen no tiene padre y no debe aparecer agrupada"
    );
}

#[test]
fn test_compaction_with_tombstones_and_updates() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    // 1. Base: C:\ -> Musica\ -> {Song_A.mp3, Song_B.flac}
    let mut builder = IndexBuilder::new();
    builder.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);
    let root = builder.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);
    let musica = builder.add_entry(root, "Musica", true, false, false, 0, 0, 100, 100);
    let song_a = builder.add_entry(musica, "Song_A.mp3", false, false, false, 0, 1000, 100, 100);
    builder.add_entry(musica, "Song_B.flac", false, false, false, 0, 2000, 100, 100);

    let file = std::fs::File::create(path).unwrap();
    let mut writer = BufWriter::new(file);
    builder.write_to(&mut writer).unwrap();
    drop(writer);

    let mmap_base = MmapIndex::open(path).unwrap();
    let base_count = mmap_base.view().unwrap().entry_count() as u32;

    // 2. La capa de cambios se ancla al base: sin eso, sus identificadores no
    //    significan nada y las lapidas se descartan en silencio.
    let mut overlay = OverlayIndex::with_base_count(base_count);
    overlay.mark_deleted(song_a);
    overlay.add_entry(musica, "Song_C.wav", false, false, false, 0, 3000, 200, 200);
    assert_eq!(overlay.tombstone_count(), 1);
    assert!(overlay.has_changes());

    overlay.compact(Some(&mmap_base), path, 2).unwrap();

    // 3. Resultado: raiz, Musica, Song_B y Song_C.
    let mmap_compacted = MmapIndex::open(path).unwrap();
    let view = mmap_compacted.view().unwrap();

    assert_eq!(view.entry_count(), 4);
    assert_eq!(view.header.generation, 2);

    let names: Vec<&str> = (0..view.entry_count())
        .filter_map(|i| view.get_name(i))
        .collect();
    assert!(!names.contains(&"Song_A.mp3"), "la entrada borrada no debe sobrevivir");
    assert!(names.contains(&"Song_B.flac"));
    assert!(names.contains(&"Song_C.wav"));

    // Lo que de verdad importa: los supervivientes siguen en su carpeta pese al
    // renumerado, y la nueva entrada aterriza en la carpeta correcta.
    for wanted in ["Song_B.flac", "Song_C.wav"] {
        let idx = (0..view.entry_count())
            .find(|&i| view.get_name(i) == Some(wanted))
            .unwrap();
        assert_eq!(
            view.resolve_full_path(idx),
            format!("C:\\Musica\\{}", wanted)
        );
    }

    // Y la capa queda anclada al indice recien publicado.
    assert_eq!(overlay.base_count, 4);
    assert!(!overlay.has_changes());
}

#[test]
fn test_1m_index_open_latency_and_budget() {
    let count = 1_000_000usize;
    let mut builder = IndexBuilder::with_capacity(count);
    builder.vol_table.add_or_update("C:\\", "Sistema", "NTFS", true);

    let root = builder.add_entry(u32::MAX, "", true, false, false, 0, 0, 0, 0);

    // 500 carpetas bajo la raiz, indices 1..=500
    for i in 0..500 {
        builder.add_entry(
            root,
            &format!("Folder_{}", i),
            true,
            false,
            false,
            0,
            0,
            1_700_000_000,
            1_700_000_000,
        );
    }

    let first_file = 501usize;
    for i in first_file..count {
        let parent = 1 + ((i - first_file) % 500) as u32;
        builder.add_entry(
            parent,
            &format!("DJ_Track_{:07}.wav", i),
            false,
            false,
            false,
            0,
            10 * 1024 * 1024,
            1_700_000_000,
            1_700_000_000,
        );
    }

    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    let write_start = Instant::now();
    let file = std::fs::File::create(path).unwrap();
    let mut writer = BufWriter::with_capacity(4 * 1024 * 1024, file);
    let bytes_written = builder.write_to(&mut writer).unwrap();
    drop(writer);
    println!(
        "Indice de 1M: {} bytes ({:.2} MB) escritos en {:?}",
        bytes_written,
        bytes_written as f64 / 1_048_576.0,
        write_start.elapsed()
    );

    let open_start = Instant::now();
    let mmap_index = MmapIndex::open(path).expect("el indice de 1M deberia abrirse");
    let view = mmap_index.view().expect("el formato de 1M deberia validar");
    let open_time = open_start.elapsed();
    println!("Indice de 1M: apertura mmap + validacion en {:?}", open_time);

    assert!(
        open_time.as_millis() < 100,
        "La apertura del indice de 1M debe ser < 100 ms, tomo {:?}",
        open_time
    );

    assert_eq!(view.entry_count(), count);
    assert_eq!(view.get_name(1), Some("Folder_0"));
    assert_eq!(view.get_name(999_999), Some("DJ_Track_0999999.wav"));
    assert_eq!(view.get_extension(999_999), "wav");

    // Mil rutas al azar, comprobadas por igualdad exacta contra lo esperado.
    let mut rng = rand::thread_rng();
    for _ in 0..1000 {
        let idx = rng.gen_range(first_file..count);
        let folder = (idx - first_file) % 500;
        let expected = format!("C:\\Folder_{}\\DJ_Track_{:07}.wav", folder, idx);
        assert_eq!(view.resolve_full_path(idx), expected);
    }
    println!("Indice de 1M: 1.000 rutas verificadas.");
}
