pub struct SearchStatusFfi {
    pub ready_count: u32,
    pub total_count: u32,
    pub is_complete: bool,
    pub generation: u64,
    pub elapsed_ms: u64,
}

pub struct RowBatchFfi {
    pub generation: u64,
    pub offset: u32,
    pub count: u32,
    pub names: Vec<String>,
    pub paths: Vec<String>,
    pub extensions: Vec<String>,
    pub sizes: Vec<u64>,
    pub mtimes: Vec<u32>,
    pub flags: Vec<u8>,
}

pub fn ping() -> String {
    "pong from BDJ Search Pro Rust Engine".to_string()
}

pub fn engine_open(_index_dir: String) -> Result<(), String> {
    Ok(())
}

pub fn engine_close() {}

pub fn search(_query: String, _sort_col: u8, _ascending: bool) -> u64 {
    1
}

pub fn search_status(generation: u64) -> SearchStatusFfi {
    SearchStatusFfi {
        ready_count: 0,
        total_count: 0,
        is_complete: true,
        generation,
        elapsed_ms: 0,
    }
}

pub fn rows(generation: u64, offset: u32, _count: u32) -> RowBatchFfi {
    RowBatchFfi {
        generation,
        offset,
        count: 0,
        names: Vec::new(),
        paths: Vec::new(),
        extensions: Vec::new(),
        sizes: Vec::new(),
        mtimes: Vec::new(),
        flags: Vec::new(),
    }
}

pub fn full_path(_generation: u64, _row: u32) -> String {
    String::new()
}

pub fn reveal_in_explorer(_generation: u64, _row: u32) -> Result<(), String> {
    Ok(())
}
