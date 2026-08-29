use super::cancel::CancellationToken;
use super::matcher::SubstringMatcher;
use super::refine::RefinementStack;
use crate::index::view::IndexView;
use crate::query::{Parser, QueryAst};
use rayon::prelude::*;
use std::sync::Mutex;

#[derive(Debug, Clone, Default)]
pub struct SearchStatus {
    pub ready_count: u32,
    pub total_count: u32,
    pub is_complete: bool,
    pub generation: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub entry_ids: Vec<u32>,
    pub generation: u64,
    pub total_count: u32,
    pub elapsed_ms: u64,
}

pub struct Engine {
    token: CancellationToken,
    #[allow(dead_code)]
    refine_stack: Mutex<RefinementStack>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
            refine_stack: Mutex::new(RefinementStack::new()),
        }
    }

    pub fn current_generation(&self) -> u64 {
        self.token.current()
    }

    pub fn cancel_all(&self) -> u64 {
        self.token.bump()
    }

    pub fn search(&self, view: &IndexView, query: &str) -> (u64, SearchResult) {
        let start = std::time::Instant::now();
        let search_gen = self.token.bump();
        let ast = Parser::parse(query);

        // Simple single-term substring matching directly from index view
        let term = match &ast {
            QueryAst::Term(t) => t.as_str(),
            QueryAst::Exact(e) => e.as_str(),
            _ => query,
        };

        let matcher = SubstringMatcher::new(term);
        let count = view.entry_count();

        // If count is 0, return immediately
        if count == 0 {
            return (
                search_gen,
                SearchResult {
                    entry_ids: Vec::new(),
                    generation: search_gen,
                    total_count: 0,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                },
            );
        }

        // Parallel chunk scanning (64k chunks)
        let chunk_size = 65536;
        let num_chunks = count.div_ceil(chunk_size);

        let results: Vec<u32> = (0..num_chunks)
            .into_par_iter()
            .flat_map(|chunk_idx| {
                if self.token.is_cancelled(search_gen) {
                    return Vec::new();
                }

                let start_idx = chunk_idx * chunk_size;
                let end_idx = (start_idx + chunk_size).min(count);
                let mut local = Vec::with_capacity(256);

                for idx in start_idx..end_idx {
                    // Check alive bitmap
                    let word_idx = idx / 64;
                    let bit_idx = idx % 64;
                    if (view.alive[word_idx] & (1 << bit_idx)) == 0 {
                        continue;
                    }

                    let off = view.name_off[idx] as usize;
                    let len = view.name_len[idx] as usize;
                    let name_bytes = &view.name_arena[off..off + len];
                    let is_non_ascii = (view.flags[idx] & 0x10) != 0;

                    if matcher.matches(name_bytes, is_non_ascii) {
                        local.push(idx as u32);
                    }
                }
                local
            })
            .collect();

        let elapsed = start.elapsed().as_millis() as u64;
        let total_count = results.len() as u32;

        (
            search_gen,
            SearchResult {
                entry_ids: results,
                generation: search_gen,
                total_count,
                elapsed_ms: elapsed,
            },
        )
    }
}
