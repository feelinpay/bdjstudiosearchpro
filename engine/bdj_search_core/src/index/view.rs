use super::layout::Header;

pub struct IndexView<'a> {
    pub header: &'a Header,
    pub parent: &'a [u32],
    pub name_off: &'a [u32],
    pub name_len: &'a [u8],
    pub flags: &'a [u8],
    pub ext_id: &'a [u16],
    pub volume: &'a [u8],
    pub size: &'a [u64],
    pub mtime: &'a [u32],
    pub ctime: &'a [u32],
    pub name_arena: &'a [u8],
    pub name_order: &'a [u32],
    pub ext_table: &'a [u8],
    pub alive: &'a [u64],
}

impl<'a> IndexView<'a> {
    pub fn entry_count(&self) -> usize {
        self.header.entry_count as usize
    }

    pub fn get_name(&self, idx: usize) -> Option<&str> {
        if idx >= self.entry_count() {
            return None;
        }
        let off = self.name_off[idx] as usize;
        let len = self.name_len[idx] as usize;
        std::str::from_utf8(&self.name_arena[off..off + len]).ok()
    }
}
