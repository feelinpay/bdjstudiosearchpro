use super::ast::QueryAst;
use crate::index::view::IndexView;
use crate::search::matcher::SubstringMatcher;
use regex::Regex;

#[derive(Clone)]
pub enum CompiledQueryAst {
    Term(SubstringMatcher),
    Exact(SubstringMatcher),
    Ext(Vec<String>),
    Size { min: Option<u64>, max: Option<u64> },
    DateModified { min: Option<u32>, max: Option<u32> },
    DateCreated { min: Option<u32>, max: Option<u32> },
    Path(SubstringMatcher),
    Parent(SubstringMatcher),
    FileType(String),
    Regex(Option<Regex>),
    Case(String),
    FileOnly,
    FolderOnly,
    And(Vec<CompiledQueryAst>),
    Or(Vec<CompiledQueryAst>),
    Not(Box<CompiledQueryAst>),
}

impl CompiledQueryAst {
    pub fn compile(ast: &QueryAst) -> Self {
        match ast {
            QueryAst::Term(term) => CompiledQueryAst::Term(SubstringMatcher::new(term)),
            QueryAst::Exact(exact) => CompiledQueryAst::Exact(SubstringMatcher::new(exact)),
            QueryAst::Ext(exts) => CompiledQueryAst::Ext(exts.clone()),
            QueryAst::Size { min, max } => CompiledQueryAst::Size { min: *min, max: *max },
            QueryAst::DateModified { min, max } => CompiledQueryAst::DateModified { min: *min, max: *max },
            QueryAst::DateCreated { min, max } => CompiledQueryAst::DateCreated { min: *min, max: *max },
            QueryAst::Path(path) => CompiledQueryAst::Path(SubstringMatcher::new(path)),
            QueryAst::Parent(parent) => CompiledQueryAst::Parent(SubstringMatcher::new(parent)),
            QueryAst::FileType(ft) => CompiledQueryAst::FileType(ft.clone()),
            QueryAst::Regex(pattern) => CompiledQueryAst::Regex(Regex::new(pattern).ok()),
            QueryAst::Case(case) => CompiledQueryAst::Case(case.clone()),
            QueryAst::FileOnly => CompiledQueryAst::FileOnly,
            QueryAst::FolderOnly => CompiledQueryAst::FolderOnly,
            QueryAst::And(terms) => CompiledQueryAst::And(terms.iter().map(Self::compile).collect()),
            QueryAst::Or(terms) => CompiledQueryAst::Or(terms.iter().map(Self::compile).collect()),
            QueryAst::Not(inner) => CompiledQueryAst::Not(Box::new(Self::compile(inner))),
        }
    }

    #[inline(always)]
    pub fn matches(&self, view: &IndexView, idx: usize) -> bool {
        match self {
            CompiledQueryAst::Term(matcher) => {
                let is_non_ascii = (view.flags[idx] & crate::index::layout::FLAG_NON_ASCII) != 0;
                matcher.matches(view.get_name_bytes(idx), is_non_ascii)
            }
            CompiledQueryAst::Exact(matcher) => {
                let is_non_ascii = (view.flags[idx] & crate::index::layout::FLAG_NON_ASCII) != 0;
                matcher.matches(view.get_name_bytes(idx), is_non_ascii)
            }
            CompiledQueryAst::Ext(exts) => {
                let actual_ext = view.get_extension(idx);
                if actual_ext.is_empty() {
                    return false;
                }
                exts.iter().any(|e| e.eq_ignore_ascii_case(actual_ext))
            }
            CompiledQueryAst::Size { min, max } => {
                let s = view.size[idx];
                if min.is_some_and(|min_val| s < min_val) {
                    return false;
                }
                if max.is_some_and(|max_val| s > max_val) {
                    return false;
                }
                true
            }
            CompiledQueryAst::DateModified { min, max } => {
                let m = view.mtime[idx];
                if min.is_some_and(|min_val| m < min_val) {
                    return false;
                }
                if max.is_some_and(|max_val| m > max_val) {
                    return false;
                }
                true
            }
            CompiledQueryAst::DateCreated { min, max } => {
                let c = view.ctime[idx];
                if min.is_some_and(|min_val| c < min_val) {
                    return false;
                }
                if max.is_some_and(|max_val| c > max_val) {
                    return false;
                }
                true
            }
            CompiledQueryAst::Path(matcher) => {
                let full_path = view.resolve_full_path(idx);
                matcher.matches(full_path.as_bytes(), true) // Path resolution could involve non-ascii
            }
            CompiledQueryAst::Parent(matcher) => {
                let parent_id = view.parent[idx];
                if parent_id == u32::MAX {
                    return false;
                }
                if let Some(parent_name) = view.get_name(parent_id as usize) {
                    matcher.matches(parent_name.as_bytes(), true)
                } else {
                    false
                }
            }
            CompiledQueryAst::FileType(type_name) => crate::query::eval::QueryEvaluator::matches_file_type(type_name, view, idx),
            CompiledQueryAst::Regex(re_opt) => {
                if let Some(re) = re_opt {
                    if let Some(name) = view.get_name(idx) {
                        re.is_match(name)
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CompiledQueryAst::Case(case_term) => {
                if let Some(name) = view.get_name(idx) {
                    name.contains(case_term)
                } else {
                    false
                }
            }
            CompiledQueryAst::FileOnly => !view.is_dir(idx),
            CompiledQueryAst::FolderOnly => view.is_dir(idx),
            CompiledQueryAst::And(terms) => terms.iter().all(|t| t.matches(view, idx)),
            CompiledQueryAst::Or(terms) => terms.iter().any(|t| t.matches(view, idx)),
            CompiledQueryAst::Not(inner) => !inner.matches(view, idx),
        }
    }
}
