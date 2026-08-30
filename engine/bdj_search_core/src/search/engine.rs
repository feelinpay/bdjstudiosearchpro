use super::cancel::CancellationToken;
use super::refine::RefinementStack;
use crate::index::view::IndexView;
use crate::query::{CompiledQueryAst, Parser};
use crate::sort::{PermutationSort, RadixSort};
use rayon::prelude::*;
use std::sync::Mutex;
use std::time::Instant;

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
        let cancel_gen = self.token.bump();
        let mut stack = self.refine_stack.lock().unwrap();
        stack.clear();
        cancel_gen
    }

    /// Executes high-speed search with incremental refinement and SIMD matching.
    pub fn search(
        &self,
        view: &IndexView,
        query: &str,
        sort_col: u8,
        ascending: bool,
    ) -> (u64, SearchResult) {
        let start = Instant::now();
        let search_gen = self.token.bump();
        let trimmed = query.trim();

        if trimmed.is_empty() {
            let total_alive = view.entry_count() as u32;
            return (
                search_gen,
                SearchResult {
                    entry_ids: Vec::new(),
                    generation: search_gen,
                    total_count: total_alive,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                },
            );
        }

        let ast = Parser::parse(trimmed);
        let count = view.entry_count();
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

        let compiled_ast = CompiledQueryAst::compile(&ast);

        // 1. Check for Incremental Refinement opportunity
        let candidate_slice = {
            let stack = self.refine_stack.lock().unwrap();
            stack.find_candidate(trimmed, &ast).map(|s| s.to_vec())
        };

        let mut matched_ids: Vec<u32> = if let Some(candidates) = candidate_slice {
            // Incremental path: only scan surviving IDs from previous keystroke (< 8 ms!)
            candidates
                .into_par_iter()
                .filter(|&id| {
                    if self.token.is_cancelled(search_gen) {
                        return false;
                    }
                    let u_idx = id as usize;
                    if !view.is_alive(u_idx) {
                        return false;
                    }
                    compiled_ast.matches(view, u_idx)
                })
                .collect()
        } else {
            // Full parallel scan across 64k chunks
            let chunk_size = 65536;
            let num_chunks = count.div_ceil(chunk_size);

            (0..num_chunks)
                .into_par_iter()
                .flat_map(|chunk_idx| {
                    if self.token.is_cancelled(search_gen) {
                        return Vec::new();
                    }

                    let start_idx = chunk_idx * chunk_size;
                    let end_idx = (start_idx + chunk_size).min(count);
                    let mut local = Vec::with_capacity(256);

                    for idx in start_idx..end_idx {
                        if !view.is_alive(idx) {
                            continue;
                        }
                        if compiled_ast.matches(view, idx) {
                            local.push(idx as u32);
                        }
                    }
                    local
                })
                .collect()
        };

        if self.token.is_cancelled(search_gen) {
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

        // Cache this result in refinement stack for the next keystroke
        {
            let mut stack = self.refine_stack.lock().unwrap();
            stack.push(trimmed.to_string(), ast.clone(), matched_ids.clone());
        }

        // 2. Sorting
        Self::apply_sort(&mut matched_ids, view, sort_col, ascending);

        let total_count = matched_ids.len() as u32;
        let elapsed = start.elapsed().as_millis() as u64;

        (
            search_gen,
            SearchResult {
                entry_ids: matched_ids,
                generation: search_gen,
                total_count,
                elapsed_ms: elapsed,
            },
        )
    }

    fn apply_sort(ids: &mut [u32], view: &IndexView, sort_col: u8, ascending: bool) {
        match sort_col {
            0 => {
                // Name order:
                // Fast path: for small result sets (< 1000), sort in-place with zero bitset allocation
                if ids.len() < 1000 {
                    PermutationSort::sort_in_place_small(ids, view, ascending);
                } else {
                    // For large result sets, use precomputed name_order permutation
                    let words_needed = view.entry_count().div_ceil(64);
                    let mut bitset = vec![0u64; words_needed];
                    for &id in ids.iter() {
                        let w = (id / 64) as usize;
                        let b = id % 64;
                        bitset[w] |= 1 << b;
                    }
                    let sorted = PermutationSort::sort_by_name_order(&bitset, view.name_order, ascending, ids.len());
                    ids.copy_from_slice(&sorted[..ids.len()]);
                }
            }
            3 => {
                // Size: Radix sort
                RadixSort::sort_by_u64_key(ids, view.size, ascending);
            }
            4 => {
                // Date Modified: Radix sort
                RadixSort::sort_by_u32_key(ids, view.mtime, ascending);
            }
            _ => {
                // Default keep order or natural
            }
        }
    }
}
