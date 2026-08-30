use super::ast::QueryAst;
use crate::index::view::IndexView;
use crate::search::matcher::SubstringMatcher;
use regex::Regex;

pub struct QueryEvaluator;

impl QueryEvaluator {
    /// Evaluates if an index entry matches the QueryAst.
    pub fn matches(ast: &QueryAst, view: &IndexView, idx: usize) -> bool {
        match ast {
            QueryAst::Term(term) => {
                if term.is_empty() {
                    return true;
                }
                let matcher = SubstringMatcher::new(term);
                let is_non_ascii = (view.flags[idx] & crate::index::layout::FLAG_NON_ASCII) != 0;
                matcher.matches(view.get_name_bytes(idx), is_non_ascii)
            }
            QueryAst::Exact(exact) => {
                if exact.is_empty() {
                    return true;
                }
                let matcher = SubstringMatcher::new(exact);
                let is_non_ascii = (view.flags[idx] & crate::index::layout::FLAG_NON_ASCII) != 0;
                matcher.matches(view.get_name_bytes(idx), is_non_ascii)
            }
            QueryAst::Ext(exts) => {
                let actual_ext = view.get_extension(idx);
                if actual_ext.is_empty() {
                    return false;
                }
                exts.iter().any(|e| e.eq_ignore_ascii_case(actual_ext))
            }
            QueryAst::Size { min, max } => {
                let s = view.size[idx];
                if min.is_some_and(|min_val| s < min_val) {
                    return false;
                }
                if max.is_some_and(|max_val| s > max_val) {
                    return false;
                }
                true
            }
            QueryAst::DateModified { min, max } => {
                let m = view.mtime[idx];
                if min.is_some_and(|min_val| m < min_val) {
                    return false;
                }
                if max.is_some_and(|max_val| m > max_val) {
                    return false;
                }
                true
            }
            QueryAst::DateCreated { min, max } => {
                let c = view.ctime[idx];
                if min.is_some_and(|min_val| c < min_val) {
                    return false;
                }
                if max.is_some_and(|max_val| c > max_val) {
                    return false;
                }
                true
            }
            QueryAst::Path(expected_path) => {
                let full_path = view.resolve_full_path(idx);
                let matcher = SubstringMatcher::new(expected_path);
                matcher.matches(full_path.as_bytes(), !expected_path.is_ascii())
            }
            QueryAst::Parent(expected_parent) => {
                let parent_id = view.parent[idx];
                if parent_id == u32::MAX {
                    return false;
                }
                if let Some(parent_name) = view.get_name(parent_id as usize) {
                    let matcher = SubstringMatcher::new(expected_parent);
                    matcher.matches(parent_name.as_bytes(), !expected_parent.is_ascii())
                } else {
                    false
                }
            }
            QueryAst::FileType(type_name) => Self::matches_file_type(type_name, view, idx),
            QueryAst::Regex(pattern) => {
                if let Ok(re) = Regex::new(pattern) {
                    if let Some(name) = view.get_name(idx) {
                        re.is_match(name)
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            QueryAst::Case(case_term) => {
                if let Some(name) = view.get_name(idx) {
                    name.contains(case_term)
                } else {
                    false
                }
            }
            QueryAst::FileOnly => !view.is_dir(idx),
            QueryAst::FolderOnly => view.is_dir(idx),
            QueryAst::And(terms) => terms.iter().all(|t| Self::matches(t, view, idx)),
            QueryAst::Or(terms) => terms.iter().any(|t| Self::matches(t, view, idx)),
            QueryAst::Not(inner) => !Self::matches(inner, view, idx),
        }
    }

    /// Evaluates if an index entry matches the CompiledQueryAst.
    #[inline(always)]
    pub fn matches_compiled(ast: &crate::query::CompiledQueryAst, view: &IndexView, idx: usize) -> bool {
        ast.matches(view, idx)
    }


    pub(crate) fn matches_file_type(type_name: &str, view: &IndexView, idx: usize) -> bool {
        let is_dir = view.is_dir(idx);
        let ext = view.get_extension(idx);

        match type_name {
            "audio" => {
                !is_dir
                    && matches!(
                        ext,
                        "wav"
                            | "flac"
                            | "aiff"
                            | "aif"
                            | "mp3"
                            | "m4a"
                            | "ogg"
                            | "opus"
                            | "wma"
                            | "alac"
                            | "ape"
                            | "wv"
                            | "aac"
                    )
            }
            "video" => {
                !is_dir
                    && matches!(
                        ext,
                        "mp4"
                            | "mov"
                            | "avi"
                            | "mkv"
                            | "m4v"
                            | "webm"
                            | "mpg"
                            | "mpeg"
                            | "wmv"
                            | "flv"
                    )
            }
            "imagen" | "image" => {
                !is_dir
                    && matches!(
                        ext,
                        "jpg"
                            | "jpeg"
                            | "png"
                            | "gif"
                            | "webp"
                            | "heic"
                            | "tiff"
                            | "bmp"
                            | "svg"
                            | "psd"
                    )
            }
            "documentos" | "docs" => {
                !is_dir
                    && matches!(
                        ext,
                        "pdf" | "docx" | "doc" | "txt" | "rtf" | "odt" | "xlsx" | "pptx" | "md"
                    )
            }
            "proyectos" | "dj" => {
                !is_dir
                    && matches!(
                        ext,
                        "als"
                            | "flp"
                            | "ptx"
                            | "cpr"
                            | "logicx"
                            | "rpp"
                            | "nki"
                            | "nkm"
                            | "cue"
                            | "m3u"
                            | "m3u8"
                            | "xml"
                    )
            }
            "comprimidos" | "zip" => {
                !is_dir
                    && matches!(
                        ext,
                        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz"
                    )
            }
            "apps" | "aplicaciones" => {
                !is_dir && matches!(ext, "exe" | "msi" | "dll" | "app" | "dmg" | "pkg")
            }
            "carpetas" | "folder" => is_dir,
            _ => false,
        }
    }

    /// Determines if a new AST is monotonically more restrictive than an old AST.
    /// Used to validate if we can refine from the previous search result set.
    pub fn is_monotonically_restrictive(old_ast: &QueryAst, new_ast: &QueryAst) -> bool {
        match (old_ast, new_ast) {
            (QueryAst::Term(old_t), QueryAst::Term(new_t)) => {
                new_t.starts_with(old_t)
            }
            (QueryAst::Exact(old_e), QueryAst::Exact(new_e)) => {
                new_e.starts_with(old_e)
            }
            (QueryAst::And(old_terms), QueryAst::And(new_terms)) => {
                // New AND query has all old terms plus additional terms or prefixes
                if new_terms.len() < old_terms.len() {
                    return false;
                }
                old_terms.iter().all(|ot| new_terms.contains(ot))
            }
            _ => false,
        }
    }
}
