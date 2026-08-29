use super::arena::NameArena;

#[derive(Default)]
pub struct IndexBuilder {
    pub parents: Vec<u32>,
    pub name_offs: Vec<u32>,
    pub name_lens: Vec<u8>,
    pub flags: Vec<u8>,
    pub ext_ids: Vec<u16>,
    pub volumes: Vec<u8>,
    pub sizes: Vec<u64>,
    pub mtimes: Vec<u32>,
    pub ctimes: Vec<u32>,
    pub arena: NameArena,
    pub name_order: Vec<u32>,
    pub alive: Vec<u64>,
}

impl IndexBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_entry(
        &mut self,
        parent: u32,
        name: &str,
        flag: u8,
        ext_id: u16,
        vol: u8,
        size: u64,
        mtime: u32,
        ctime: u32,
    ) -> u32 {
        let id = self.parents.len() as u32;
        let (off, len) = self.arena.push(name);
        self.parents.push(parent);
        self.name_offs.push(off);
        self.name_lens.push(len);
        self.flags.push(flag);
        self.ext_ids.push(ext_id);
        self.volumes.push(vol);
        self.sizes.push(size);
        self.mtimes.push(mtime);
        self.ctimes.push(ctime);
        self.name_order.push(id);
        
        let word_idx = (id / 64) as usize;
        let bit_idx = id % 64;
        if word_idx >= self.alive.len() {
            self.alive.push(0);
        }
        self.alive[word_idx] |= 1 << bit_idx;
        id
    }

    pub fn count(&self) -> usize {
        self.parents.len()
    }
}
