use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let count: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);

    let output_path = args
        .get(2)
        .map(|s| s.as_str())
        .unwrap_or("corpus_1m.bdjx");

    println!("Generating synthetic corpus of {} entries to '{}'...", count, output_path);
    let start = Instant::now();

    let artists = [
        "Michael Jackson", "Daft Punk", "David Guetta", "Tiësto", "Calvin Harris",
        "Carl Cox", "Armin van Buuren", "Deadmau5", "Fisher", "Skrillex",
        "Avicii", "Martin Garrix", "Swedish House Mafia", "Charlotte de Witte",
        "Peggy Gou", "Fred again..", "Bicep", "Disclosure", "James Hype", "ACRAZE",
        "Bad Bunny", "J Balvin", "Daddy Yankee", "Bizarrap", "Rauw Alejandro",
    ];

    let tracks = [
        "Billie Jean", "One More Time", "Titanium", "Levels", "Animals",
        "Don't You Worry Child", "Losing It", "Ferrari", "Do It To It",
        "Stay The Night", "Midnight City", "Strobe", "Adagio for Strings",
        "Satisfaction", "Gecko", "Show Me Love", "Barbra Streisand",
        "Turn Off The Lights", "Cinema", "Reload", "Pepas", "Gasolina",
        "Tití Me Preguntó", "Provenza", "Session 52",
    ];

    let mixes = [
        "Original Mix", "Extended Mix", "Club Mix", "Dub Mix", "Radio Edit",
        "VIP Remix", "Acapella", "Instrumental", "Remastered", "Live 2026",
    ];

    let audio_exts = ["wav", "mp3", "flac", "aiff", "m4a"];
    let other_exts = ["als", "flp", "zip", "pdf", "jpg", "mp4"];

    let mut parents = Vec::with_capacity(count);
    let mut name_offs = Vec::with_capacity(count);
    let mut name_lens = Vec::with_capacity(count);
    let mut flags = Vec::with_capacity(count);
    let mut ext_ids = Vec::with_capacity(count);
    let mut volumes = Vec::with_capacity(count);
    let mut sizes = Vec::with_capacity(count);
    let mut mtimes = Vec::with_capacity(count);
    let mut ctimes = Vec::with_capacity(count);
    let mut name_order = Vec::with_capacity(count);
    let mut arena = Vec::new();

    let words_needed = (count + 63) / 64;
    let mut alive = vec![!0u64; words_needed];
    if count % 64 != 0 {
        let remainder = count % 64;
        alive[words_needed - 1] = (1u64 << remainder) - 1;
    }

    // Base timestamp (Aug 2026)
    let base_time: u32 = 1787961600;

    for i in 0..count {
        let artist = artists[i % artists.len()];
        let track = tracks[(i / artists.len()) % tracks.len()];
        let mix = mixes[(i / 7) % mixes.len()];

        let is_dir = i < 1000; // First 1000 entries are folders
        let name = if is_dir {
            format!("Folder_{}_{}", artist, i)
        } else {
            let ext = if (i % 10) < 8 {
                audio_exts[i % audio_exts.len()]
            } else {
                other_exts[i % other_exts.len()]
            };
            format!("{} - {} ({}) [DJ Rip].{}", artist, track, mix, ext)
        };

        let off = arena.len() as u32;
        let bytes = name.as_bytes();
        let len = bytes.len().min(255) as u8;
        arena.extend_from_slice(&bytes[..len as usize]);

        let parent = if is_dir {
            u32::MAX
        } else {
            (i % 1000) as u32
        };

        let flag = if is_dir { 1u8 } else { 0u8 };
        let ext_id = if is_dir { 0u16 } else { ((i % 12) + 1) as u16 };
        let vol = (i % 3) as u8; // 3 mock volumes
        let size = if is_dir {
            0u64
        } else {
            1024 * 1024 * ((i % 50) as u64 + 3) // 3 MB to 53 MB
        };
        let mtime = base_time.saturating_sub((i % 100000) as u32 * 60);
        let ctime = mtime.saturating_sub(86400);

        parents.push(parent);
        name_offs.push(off);
        name_lens.push(len);
        flags.push(flag);
        ext_ids.push(ext_id);
        volumes.push(vol);
        sizes.push(size);
        mtimes.push(mtime);
        ctimes.push(ctime);
        name_order.push(i as u32);
    }

    println!("Built structures in memory in {:?}", start.elapsed());
    let write_start = Instant::now();

    // Write .bdjx format
    let file = File::create(output_path).expect("Failed to create output file");
    let mut writer = BufWriter::with_capacity(4 * 1024 * 1024, file);

    // 64-byte Header
    let magic: [u8; 8] = *b"BDJXIDX\0";
    let version: u32 = 1;
    let header_crc: u32 = 0; // Checksum placeholder
    let entry_count: u64 = count as u64;
    let arena_len: u64 = arena.len() as u64;
    let created_at: u64 = base_time as u64;
    let generation: u64 = 1;
    let install_id: [u8; 16] = [0x42; 16];

    writer.write_all(&magic).unwrap();
    writer.write_all(&version.to_le_bytes()).unwrap();
    writer.write_all(&header_crc.to_le_bytes()).unwrap();
    writer.write_all(&entry_count.to_le_bytes()).unwrap();
    writer.write_all(&arena_len.to_le_bytes()).unwrap();
    writer.write_all(&created_at.to_le_bytes()).unwrap();
    writer.write_all(&generation.to_le_bytes()).unwrap();
    writer.write_all(&install_id).unwrap();

    // Align helper
    let mut written = 64usize;
    let align_to_64 = |w: &mut BufWriter<File>, current: &mut usize| {
        let rem = *current % 64;
        if rem != 0 {
            let pad = 64 - rem;
            w.write_all(&vec![0u8; pad]).unwrap();
            *current += pad;
        }
    };

    align_to_64(&mut writer, &mut written);

    // Helper macro to write slice
    macro_rules! write_col {
        ($slice:expr, $elem_size:expr) => {
            for item in $slice {
                writer.write_all(&item.to_le_bytes()).unwrap();
            }
            written += $slice.len() * $elem_size;
            align_to_64(&mut writer, &mut written);
        };
    }

    write_col!(&parents, 4);
    write_col!(&name_offs, 4);

    for &b in &name_lens {
        writer.write_all(&[b]).unwrap();
    }
    written += name_lens.len();
    align_to_64(&mut writer, &mut written);

    for &b in &flags {
        writer.write_all(&[b]).unwrap();
    }
    written += flags.len();
    align_to_64(&mut writer, &mut written);

    write_col!(&ext_ids, 2);

    for &b in &volumes {
        writer.write_all(&[b]).unwrap();
    }
    written += volumes.len();
    align_to_64(&mut writer, &mut written);

    write_col!(&sizes, 8);
    write_col!(&mtimes, 4);
    write_col!(&ctimes, 4);
    write_col!(&alive, 8);
    write_col!(&name_order, 4);

    // Write name arena
    writer.write_all(&arena).unwrap();
    written += arena.len();
    align_to_64(&mut writer, &mut written);

    writer.flush().unwrap();
    println!(
        "Successfully wrote {} bytes ({:.2} MB) in {:?}",
        written,
        written as f64 / (1024.0 * 1024.0),
        write_start.elapsed()
    );
    println!("Total elapsed time: {:?}", start.elapsed());
}
