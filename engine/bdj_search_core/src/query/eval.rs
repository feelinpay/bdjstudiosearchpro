use super::ast::QueryAst;
use crate::index::view::IndexView;
use crate::search::matcher::{SubstringMatcher, WildcardMatcher};
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
                let is_non_ascii = (view.flags[idx] & crate::index::layout::FLAG_NON_ASCII) != 0;
                if WildcardMatcher::contains_wildcard(term) {
                    return WildcardMatcher::new(term).matches(view.get_name_bytes(idx), is_non_ascii);
                }
                let matcher = SubstringMatcher::new(term);
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
                let s = view.size_of(idx);
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
            QueryAst::ParentId(expected_id) => view.parent[idx] == *expected_id,
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
            QueryAst::HiddenOnly => view.is_hidden(idx),
            QueryAst::VisibleOnly => !view.is_hidden(idx),
            QueryAst::And(terms) => terms.iter().all(|t| Self::matches(t, view, idx)),
            QueryAst::Or(terms) => terms.iter().any(|t| Self::matches(t, view, idx)),
            QueryAst::Not(inner) => !Self::matches(inner, view, idx),
        }
    }

    /// Comprueba si una entrada pertenece a un grupo de tipo (audio, vídeo…).
    ///
    /// La lista de extensiones de cada grupo vive en un único sitio,
    /// `query::compiled::file_type_extensions`, para que el evaluador y la
    /// barra de filtros de la interfaz no puedan discrepar.
    pub(crate) fn matches_file_type(type_name: &str, view: &IndexView, idx: usize) -> bool {
        let lowered = type_name.to_ascii_lowercase();
        if lowered == "carpetas" || lowered == "carpeta" || lowered == "folder" {
            return view.is_dir(idx);
        }
        if view.is_dir(idx) {
            return false;
        }
        let ext = view.get_extension(idx);
        if ext.is_empty() {
            return false;
        }
        match crate::query::compiled::file_type_extensions(&lowered) {
            Some(list) => list.iter().any(|e| e.eq_ignore_ascii_case(ext)),
            None => false,
        }
    }

    /// ¿Toda entrada que cumple `new` cumple también `old`?
    ///
    /// Si la respuesta es sí, el resultado de `new` es un subconjunto del de
    /// `old` y basta con volver a filtrar el resultado anterior en lugar de
    /// recorrer el índice entero. Es lo que hace que teclear la sexta letra
    /// cueste microsegundos.
    ///
    /// La versión anterior exigía que los términos viejos aparecieran
    /// **idénticos** en la consulta nueva. Al teclear dentro de una palabra de
    /// una consulta de varias —«michael bil» → «michael bill»— el término viejo
    /// ya no estaba literal y se caía al recorrido completo. Justo el caso más
    /// común: artista y título.
    pub fn implied_by(new_ast: &QueryAst, old_ast: &QueryAst) -> bool {
        if new_ast == old_ast {
            return true;
        }

        // Cumplir un Y lógico es cumplir todas sus partes.
        if let QueryAst::And(olds) = old_ast {
            return olds.iter().all(|o| Self::implied_by(new_ast, o));
        }

        // Basta con que una parte del Y lógico nuevo implique lo viejo.
        if let QueryAst::And(news) = new_ast {
            return news.iter().any(|n| Self::implied_by(n, old_ast));
        }

        match (new_ast, old_ast) {
            // «michael bill» contiene «michael bil»: todo nombre que contenga
            // el término nuevo contiene también el viejo.
            (QueryAst::Term(n), QueryAst::Term(o))
            | (QueryAst::Exact(n), QueryAst::Exact(o))
            | (QueryAst::Exact(n), QueryAst::Term(o)) => {
                !o.is_empty() && n.to_lowercase().contains(&o.to_lowercase())
            }
            (QueryAst::Path(n), QueryAst::Path(o)) | (QueryAst::Parent(n), QueryAst::Parent(o)) => {
                !o.is_empty() && n.to_lowercase().contains(&o.to_lowercase())
            }
            // Un rango más estrecho implica al más ancho.
            (
                QueryAst::Size { min: nmin, max: nmax },
                QueryAst::Size { min: omin, max: omax },
            ) => Self::range_narrows_u64(*nmin, *nmax, *omin, *omax),
            (
                QueryAst::DateModified { min: nmin, max: nmax },
                QueryAst::DateModified { min: omin, max: omax },
            )
            | (
                QueryAst::DateCreated { min: nmin, max: nmax },
                QueryAst::DateCreated { min: omin, max: omax },
            ) => Self::range_narrows_u32(*nmin, *nmax, *omin, *omax),
            (QueryAst::FileOnly, QueryAst::FileOnly) => true,
            (QueryAst::FolderOnly, QueryAst::FolderOnly) => true,
            (QueryAst::HiddenOnly, QueryAst::HiddenOnly) => true,
            (QueryAst::VisibleOnly, QueryAst::VisibleOnly) => true,
            _ => false,
        }
    }

    fn range_narrows_u64(
        nmin: Option<u64>,
        nmax: Option<u64>,
        omin: Option<u64>,
        omax: Option<u64>,
    ) -> bool {
        let min_ok = match omin {
            None => true,
            Some(o) => nmin.is_some_and(|n| n >= o),
        };
        let max_ok = match omax {
            None => true,
            Some(o) => nmax.is_some_and(|n| n <= o),
        };
        min_ok && max_ok
    }

    fn range_narrows_u32(
        nmin: Option<u32>,
        nmax: Option<u32>,
        omin: Option<u32>,
        omax: Option<u32>,
    ) -> bool {
        let min_ok = match omin {
            None => true,
            Some(o) => nmin.is_some_and(|n| n >= o),
        };
        let max_ok = match omax {
            None => true,
            Some(o) => nmax.is_some_and(|n| n <= o),
        };
        min_ok && max_ok
    }
}
