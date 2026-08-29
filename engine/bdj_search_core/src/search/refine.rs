pub struct RefinementStack {
    stack: Vec<(String, Vec<u32>)>,
}

impl RefinementStack {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    pub fn push(&mut self, query: String, results: Vec<u32>) {
        self.stack.push((query, results));
    }

    pub fn find_longest_prefix(&self, query: &str) -> Option<&[u32]> {
        for (q, res) in self.stack.iter().rev() {
            if query.starts_with(q) {
                return Some(res.as_slice());
            }
        }
        None
    }

    pub fn clear(&mut self) {
        self.stack.clear();
    }
}
