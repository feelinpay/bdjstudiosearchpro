#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct Header {
    pub magic: [u8; 8],
    pub version: u32,
    pub header_crc: u32,
    pub entry_count: u64,
    pub arena_len: u64,
    pub created_at: u64,
    pub generation: u64,
    pub install_id: [u8; 16],
}

pub const MAGIC: &[u8; 8] = b"BDJXIDX\0";
pub const CURRENT_VERSION: u32 = 1;
