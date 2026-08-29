use bdj_search_core::index::{IndexBuilder, MmapIndex, OverlayIndex};
use rand::Rng;
use std::io::BufWriter;
use std::time::Instant;
use tempfile::NamedTempFile;

#[test]
fn test_index_build_mmap_and_path_resolution() {
    let count = 100_000usize;
    let mut builder = IndexBuilder::with_capacity(count);
    builder.vol_table.add_or_update(
        if cfg!(windows) { "D:\\" } else { "/Volumes/Music" },
        "SSD Música",
        "NTFS",
        true,
    );

    // Create 100 directories
    for i in 0..100 {
        builder.add_entry(
            u32::MAX,
            &format!("Dir_{}", i),
            true,
            false,
            false,
            0,
            0,
            1700000000 + i as u32,
            1700000000,
        );
    }

    // Create remaining files inside directories
    let mut expected_names = Vec::with_capacity(count);
    let mut expected_parents = Vec::with_capacity(count);
    let mut expected_sizes = Vec::with_capacity(count);

    for i in 100..count {
        let parent_id = (i % 100) as u32;
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
            1700000000 + (i % 1000) as u32,
            1700000000,
        );
    }

    // Write to temporary file
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    let write_start = Instant::now();
    let file = std::fs::File::create(path).unwrap();
    let mut writer = BufWriter::with_capacity(2 * 1024 * 1024, file);
    builder.write_to(&mut writer).unwrap();
    drop(writer);
    let write_time = write_start.elapsed();
    println!("Wrote {} entries in {:?}", count, write_time);

    // Test Acceptance Criteria: Open index via mmap in < 100 ms
    let open_start = Instant::now();
    let mmap_index = MmapIndex::open(path).expect("MmapIndex::open should succeed");
    let view = mmap_index.view().expect("view() should parse correctly");
    let open_time = open_start.elapsed();
    println!("Mmap open + header validation took: {:?}", open_time);
    assert!(
        open_time.as_millis() < 100,
        "Apertura del índice debe ser < 100 ms, tomó {:?}",
        open_time
    );

    assert_eq!(view.entry_count(), count);

    // Verify sample of 1,000 random entries
    let mut rng = rand::thread_rng();
    for _ in 0..1000 {
        let idx = rng.gen_range(100..count);
        let exp_name = &expected_names[idx - 100];
        let exp_size = expected_sizes[idx - 100];
        let exp_parent = expected_parents[idx - 100];

        assert_eq!(view.get_name(idx), Some(exp_name.as_str()));
        assert_eq!(view.get_extension(idx), "wav");
        assert_eq!(view.size[idx], exp_size);
        assert_eq!(view.parent[idx], exp_parent);

        let resolved_path = view.resolve_full_path(idx);
        let expected_parent_name = format!("Dir_{}", exp_parent);
        assert!(
            resolved_path.contains(&expected_parent_name),
            "Resolved path '{}' should contain parent '{}'",
            resolved_path,
            expected_parent_name
        );
        assert!(
            resolved_path.ends_with(exp_name),
            "Resolved path '{}' should end with track name '{}'",
            resolved_path,
            exp_name
        );
    }

    println!("Random sample of 1,000 entries verified successfully!");
}

#[test]
fn test_compaction_with_tombstones_and_updates() {
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    // 1. Initial base index
    let mut builder = IndexBuilder::new();
    builder.add_entry(u32::MAX, "RootFolder", true, false, false, 0, 0, 100, 100);
    builder.add_entry(0, "Song_A.mp3", false, false, false, 0, 1000, 100, 100);
    builder.add_entry(0, "Song_B.flac", false, false, false, 0, 2000, 100, 100);

    let file = std::fs::File::create(path).unwrap();
    let mut writer = BufWriter::new(file);
    builder.write_to(&mut writer).unwrap();
    drop(writer);

    // 2. Open base and prepare overlay
    let mmap_base = MmapIndex::open(path).unwrap();
    let mut overlay = OverlayIndex::new();

    // Delete Song_A (ID 1)
    overlay.mark_deleted(1);
    // Add Song_C in overlay
    overlay.add_entry(0, "Song_C.wav", false, false, false, 0, 3000, 200, 200);

    // 3. Compact
    overlay.compact(Some(&mmap_base), path, 2).unwrap();

    // 4. Verify compacted index
    let mmap_compacted = MmapIndex::open(path).unwrap();
    let view = mmap_compacted.view().unwrap();

    assert_eq!(view.entry_count(), 3); // RootFolder, Song_B and Song_C present
    assert_eq!(view.header.generation, 2);
    assert_eq!(view.get_name(0), Some("RootFolder"));
    assert_eq!(view.get_name(1), Some("Song_B.flac"));
    assert_eq!(view.get_name(2), Some("Song_C.wav"));
    assert_eq!(view.get_name(3), None);
}

#[test]
fn test_1m_index_open_latency_and_budget() {
    let count = 1_000_000usize;
    let mut builder = IndexBuilder::with_capacity(count);

    // Build 1M entries
    for i in 0..count {
        let is_dir = i < 500;
        let parent = if is_dir { u32::MAX } else { (i % 500) as u32 };
        let name = if is_dir {
            format!("Folder_{}", i)
        } else {
            format!("DJ_Track_{:07}.wav", i)
        };
        builder.add_entry(parent, &name, is_dir, false, false, 0, 1024 * 1024 * 10, 1700000000, 1700000000);
    }

    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    let write_start = Instant::now();
    let file = std::fs::File::create(path).unwrap();
    let mut writer = BufWriter::with_capacity(4 * 1024 * 1024, file);
    let bytes_written = builder.write_to(&mut writer).unwrap();
    drop(writer);
    println!("1M Index: wrote {} bytes ({:.2} MB) in {:?}", bytes_written, bytes_written as f64 / 1_048_576.0, write_start.elapsed());

    // Acceptance criterion: mmap open in < 100 ms
    let open_start = Instant::now();
    let mmap_index = MmapIndex::open(path).expect("MmapIndex 1M should open");
    let view = mmap_index.view().expect("1M view parse should succeed");
    let open_time = open_start.elapsed();
    println!("1M Index: mmap open + validation took: {:?}", open_time);

    assert!(
        open_time.as_millis() < 100,
        "Apertura del índice de 1M debe ser < 100 ms, tomó {:?}",
        open_time
    );

    assert_eq!(view.entry_count(), count);
    assert_eq!(view.get_name(0), Some("Folder_0"));
    assert_eq!(view.get_name(999_999), Some("DJ_Track_0999999.wav"));
    assert_eq!(view.get_extension(999_999), "wav");

    // Verify 1,000 random path resolutions
    let mut rng = rand::thread_rng();
    for _ in 0..1000 {
        let idx = rng.gen_range(500..count);
        let path = view.resolve_full_path(idx);
        assert!(path.contains("Folder_"));
        assert!(path.ends_with(".wav"));
    }
    println!("1M Index: 1,000 paths verified successfully!");
}
