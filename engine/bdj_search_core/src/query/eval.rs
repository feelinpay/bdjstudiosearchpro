use super::ast::QueryAst;

pub struct Evaluator;

impl Evaluator {
    pub fn is_monotonically_restrictive(old_ast: &QueryAst, new_ast: &QueryAst) -> bool {
        match (old_ast, new_ast) {
            (QueryAst::Term(old_term), QueryAst::Term(new_term)) => {
                new_term.starts_with(old_term)
            }
            (QueryAst::Exact(old_term), QueryAst::Exact(new_term)) => {
                new_term.starts_with(old_term)
            }
            _ => false,
        }
    }
}
