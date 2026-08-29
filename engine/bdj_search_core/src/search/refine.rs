use crate::query::ast::QueryAst;
use crate::query::eval::QueryEvaluator;

#[derive(Clone, Debug)]
pub struct RefinementEntry {
    pub query: String,
    pub ast: QueryAst,
    pub results: Vec<u32>,
}

#[derive(Default, Debug)]
pub struct RefinementStack {
    stack: Vec<RefinementEntry>,
}

impl RefinementStack {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    pub fn push(&mut self, query: String, ast: QueryAst, results: Vec<u32>) {
        // Keep max 16 progressive keystroke steps in the stack
        if self.stack.len() >= 16 {
            self.stack.remove(0);
        }
        self.stack.push(RefinementEntry {
            query,
            ast,
            results,
        });
    }

    /// Finds the closest/longest prefix query in the stack that is monotonically compatible
    /// with `new_query` and `new_ast`.
    pub fn find_candidate(&self, new_query: &str, new_ast: &QueryAst) -> Option<&[u32]> {
        for entry in self.stack.iter().rev() {
            if new_query.starts_with(&entry.query)
                && QueryEvaluator::is_monotonically_restrictive(&entry.ast, new_ast)
            {
                return Some(&entry.results);
            }
        }
        None
    }

    pub fn clear(&mut self) {
        self.stack.clear();
    }
}
