use super::ast::QueryAst;
use crate::index::view::IndexView;
use crate::search::matcher::{SubstringMatcher, WildcardMatcher};
use regex::{Regex, RegexBuilder};

/// Buffers reutilizables para evaluar filtros que necesitan la ruta completa.
///
/// Reconstruir una ruta reserva memoria. Con un buffer por hilo y por consulta,
/// el filtro `ruta:` deja de reservar una vez por archivo: sobre diez millones
/// de entradas eran veinte millones de reservas por pulsación de tecla.
#[derive(Default)]
pub struct MatchScratch {
    segments: Vec<u32>,
    path: String,
}

impl MatchScratch {
    pub fn new() -> Self {
        Self {
            segments: Vec::with_capacity(32),
            path: String::with_capacity(512),
        }
    }
}

/// Consulta lista para evaluarse: comparadores y expresiones regulares ya
/// construidos, y extensiones resueltas a identificadores internos.
///
/// El evaluador original construía un `SubstringMatcher` —dos reservas de
/// memoria— **dentro del bucle de entradas**. La expresión regular era peor: se
/// compilaba el autómata una vez por archivo.
pub enum CompiledQuery {
    /// Subcadena en el nombre.
    Term(SubstringMatcher),
    /// Comodines `*` y `?` sobre el nombre completo.
    Wildcard(WildcardMatcher),
    /// Conjunto de extensiones, por identificador interno.
    ///
    /// Comparar un `u16` contra una tabla de bits evita mirar un solo carácter
    /// del nombre, que es la operación más frecuente en el flujo de un DJ.
    ExtSet(Vec<bool>),
    Size {
        min: Option<u64>,
        max: Option<u64>,
    },
    DateModified {
        min: Option<u32>,
        max: Option<u32>,
    },
    DateCreated {
        min: Option<u32>,
        max: Option<u32>,
    },
    Path(SubstringMatcher),
    Parent(SubstringMatcher),
    ParentId(u32),
    Regex(Regex),
    Case(String),
    FileOnly,
    FolderOnly,
    HiddenOnly,
    VisibleOnly,
    And(Vec<CompiledQuery>),
    Or(Vec<CompiledQuery>),
    Not(Box<CompiledQuery>),
    /// Coincide siempre (término vacío).
    Always,
    /// No coincide nunca: expresión regular inválida o función desconocida.
    Never,
}

/// Extensiones de cada grupo predefinido de la barra de filtros.
///
/// Es la única definición de los grupos en el motor: la barra de filtros de la
/// interfaz y el evaluador comparten esta tabla.
pub fn file_type_extensions(type_name: &str) -> Option<&'static [&'static str]> {
    const AUDIO: &[&str] = &[
        "wav", "flac", "aiff", "aif", "mp3", "m4a", "ogg", "opus", "wma", "alac", "ape", "wv",
        "aac",
    ];
    const VIDEO: &[&str] = &[
        "mp4", "mov", "avi", "mkv", "m4v", "webm", "mpg", "mpeg", "wmv", "flv",
    ];
    const IMAGE: &[&str] = &[
        "jpg", "jpeg", "png", "gif", "webp", "heic", "tiff", "bmp", "svg", "psd",
    ];
    const DOCS: &[&str] = &[
        "pdf", "docx", "doc", "txt", "rtf", "odt", "xlsx", "pptx", "md",
    ];
    const PROJECTS: &[&str] = &[
        "als", "flp", "ptx", "cpr", "logicx", "rpp", "nki", "nkm", "cue", "m3u", "m3u8", "xml",
    ];
    const ARCHIVES: &[&str] = &["zip", "rar", "7z", "tar", "gz", "bz2", "xz"];
    const APPS: &[&str] = &["exe", "msi", "dll", "app", "dmg", "pkg", "bat", "cmd", "sh"];

    match type_name {
        "audio" => Some(AUDIO),
        "video" => Some(VIDEO),
        "imagen" | "image" => Some(IMAGE),
        "documentos" | "docs" | "documento" => Some(DOCS),
        "proyectos" | "dj" | "proyecto" => Some(PROJECTS),
        "comprimidos" | "zip" | "comprimido" => Some(ARCHIVES),
        "apps" | "aplicaciones" | "ejecutables" | "ejecutable" | "app" => Some(APPS),
        _ => None,
    }
}

impl CompiledQuery {
    /// Traduce el árbol sintáctico a su forma ejecutable. Se hace **una vez por
    /// consulta**, no una vez por archivo.
    pub fn compile(ast: &QueryAst, view: &IndexView<'_>) -> Self {
        match ast {
            QueryAst::Term(t) | QueryAst::Exact(t) => {
                if t.is_empty() {
                    CompiledQuery::Always
                } else if WildcardMatcher::contains_wildcard(t) {
                    CompiledQuery::Wildcard(WildcardMatcher::new(t))
                } else {
                    CompiledQuery::Term(SubstringMatcher::new(t))
                }
            }
            QueryAst::Ext(exts) => {
                CompiledQuery::ExtSet(Self::ext_mask(view, exts.iter().map(|s| s.as_str())))
            }
            QueryAst::FileType(name) => {
                let lowered = name.to_ascii_lowercase();
                if lowered == "carpetas" || lowered == "carpeta" || lowered == "folder" {
                    return CompiledQuery::FolderOnly;
                }
                match file_type_extensions(&lowered) {
                    Some(list) => CompiledQuery::And(vec![
                        CompiledQuery::FileOnly,
                        CompiledQuery::ExtSet(Self::ext_mask(view, list.iter().copied())),
                    ]),
                    None => CompiledQuery::Never,
                }
            }
            QueryAst::Size { min, max } => CompiledQuery::Size {
                min: *min,
                max: *max,
            },
            QueryAst::DateModified { min, max } => CompiledQuery::DateModified {
                min: *min,
                max: *max,
            },
            QueryAst::DateCreated { min, max } => CompiledQuery::DateCreated {
                min: *min,
                max: *max,
            },
            QueryAst::Path(p) => CompiledQuery::Path(SubstringMatcher::new(p)),
            QueryAst::Parent(p) => CompiledQuery::Parent(SubstringMatcher::new(p)),
            QueryAst::ParentId(id) => CompiledQuery::ParentId(*id),
            QueryAst::Regex(pattern) => {
                // El límite de tamaño evita que un patrón patológico reserve
                // memoria sin freno y tumbe el proceso.
                match RegexBuilder::new(pattern)
                    .size_limit(1 << 20)
                    .dfa_size_limit(1 << 20)
                    .build()
                {
                    Ok(re) => CompiledQuery::Regex(re),
                    Err(_) => CompiledQuery::Never,
                }
            }
            QueryAst::Case(c) => CompiledQuery::Case(c.clone()),
            QueryAst::FileOnly => CompiledQuery::FileOnly,
            QueryAst::FolderOnly => CompiledQuery::FolderOnly,
            QueryAst::HiddenOnly => CompiledQuery::HiddenOnly,
            QueryAst::VisibleOnly => CompiledQuery::VisibleOnly,
            QueryAst::And(items) => {
                CompiledQuery::And(items.iter().map(|a| Self::compile(a, view)).collect())
            }
            QueryAst::Or(items) => {
                CompiledQuery::Or(items.iter().map(|a| Self::compile(a, view)).collect())
            }
            QueryAst::Not(inner) => CompiledQuery::Not(Box::new(Self::compile(inner, view))),
        }
    }

    /// Tabla de bits sobre los identificadores de extensión del índice.
    fn ext_mask<'s, I: Iterator<Item = &'s str>>(view: &IndexView<'_>, exts: I) -> Vec<bool> {
        let mut mask = vec![false; view.ext_table.len().max(1)];
        for wanted in exts {
            if let Some(id) = view.ext_table.get_id(wanted)
                && (id as usize) < mask.len()
            {
                mask[id as usize] = true;
            }
        }
        mask
    }

    /// Evalúa una entrada. En la ruta caliente no reserva memoria.
    pub fn matches(&self, view: &IndexView<'_>, idx: usize, scratch: &mut MatchScratch) -> bool {
        match self {
            CompiledQuery::Always => true,
            CompiledQuery::Never => false,
            CompiledQuery::Term(m) => {
                let non_ascii = (view.flags[idx] & crate::index::layout::FLAG_NON_ASCII) != 0;
                m.matches(view.get_name_bytes(idx), non_ascii)
            }
            CompiledQuery::Wildcard(m) => {
                let non_ascii = (view.flags[idx] & crate::index::layout::FLAG_NON_ASCII) != 0;
                m.matches(view.get_name_bytes(idx), non_ascii)
            }
            CompiledQuery::ExtSet(mask) => {
                let id = view.ext_id[idx];
                // El identificador 0 significa «sin extensión».
                id != 0 && (id as usize) < mask.len() && mask[id as usize]
            }
            CompiledQuery::Size { min, max } => {
                let s = view.size_of(idx);
                !(min.is_some_and(|v| s < v) || max.is_some_and(|v| s > v))
            }
            CompiledQuery::DateModified { min, max } => {
                let m = view.mtime[idx];
                !(min.is_some_and(|v| m < v) || max.is_some_and(|v| m > v))
            }
            CompiledQuery::DateCreated { min, max } => {
                let c = view.ctime[idx];
                !(min.is_some_and(|v| c < v) || max.is_some_and(|v| c > v))
            }
            CompiledQuery::Path(m) => {
                // Reconstruir la ruta es lo más caro que hace el evaluador, así
                // que este filtro va siempre en último lugar dentro de un Y
                // lógico (véase `cost`).
                view.resolve_full_path_into(idx, &mut scratch.segments, &mut scratch.path);
                let ascii = scratch.path.is_ascii();
                m.matches(scratch.path.as_bytes(), !ascii)
            }
            CompiledQuery::Parent(m) => {
                let parent = view.parent[idx];
                if parent == u32::MAX {
                    return false;
                }
                let p = parent as usize;
                if p >= view.entry_count() {
                    return false;
                }
                let non_ascii = (view.flags[p] & crate::index::layout::FLAG_NON_ASCII) != 0;
                m.matches(view.get_name_bytes(p), non_ascii)
            }
            CompiledQuery::ParentId(expected_id) => view.parent[idx] == *expected_id,
            CompiledQuery::Regex(re) => match view.get_name(idx) {
                Some(name) => re.is_match(name),
                None => false,
            },
            CompiledQuery::Case(c) => match view.get_name(idx) {
                Some(name) => name.contains(c.as_str()),
                None => false,
            },
            CompiledQuery::FileOnly => !view.is_dir(idx),
            CompiledQuery::FolderOnly => view.is_dir(idx),
            CompiledQuery::HiddenOnly => view.is_hidden(idx),
            CompiledQuery::VisibleOnly => !view.is_hidden(idx),
            CompiledQuery::And(items) => items.iter().all(|i| i.matches(view, idx, scratch)),
            CompiledQuery::Or(items) => items.iter().any(|i| i.matches(view, idx, scratch)),
            CompiledQuery::Not(inner) => !inner.matches(view, idx, scratch),
        }
    }

    /// Término de texto que **toda** coincidencia debe cumplir, si lo hay.
    ///
    /// Sirve para conducir el barrido en bloque de la arena: se recorre la
    /// región de nombres buscando ese término y solo los candidatos que lo
    /// cumplen pagan la evaluación del resto de la consulta.
    ///
    /// De un Y lógico se elige el término más largo, que es el que más descarta.
    /// De un O lógico o una negación no se puede extraer ninguno: una
    /// coincidencia puede no cumplir ninguna de sus ramas de texto.
    pub fn driving_term(&self) -> Option<&SubstringMatcher> {
        match self {
            CompiledQuery::Term(m) if m.is_ascii_pattern() && !m.pattern_bytes().is_empty() => {
                Some(m)
            }
            CompiledQuery::And(items) => items
                .iter()
                .filter_map(|i| i.driving_term())
                .max_by_key(|m| m.pattern_bytes().len()),
            _ => None,
        }
    }

    /// Cierto si la consulta se reduce a ese único término de texto.
    ///
    /// En ese caso el barrido en bloque ya da la respuesta definitiva y no hay
    /// que volver a evaluar nada.
    pub fn is_only_term(&self) -> bool {
        matches!(self, CompiledQuery::Term(_))
    }

    /// Coste relativo de evaluar este nodo.
    ///
    /// Sirve para ordenar los términos de un Y lógico y descartar por el filtro
    /// más barato antes de mirar un solo carácter del nombre.
    fn cost(&self) -> u8 {
        match self {
            CompiledQuery::Always | CompiledQuery::Never => 0,
            CompiledQuery::FileOnly
            | CompiledQuery::FolderOnly
            | CompiledQuery::HiddenOnly
            | CompiledQuery::VisibleOnly => 1,
            CompiledQuery::ExtSet(_) => 1,
            CompiledQuery::Size { .. }
            | CompiledQuery::DateModified { .. }
            | CompiledQuery::DateCreated { .. } => 1,
            CompiledQuery::Term(_) | CompiledQuery::Wildcard(_) | CompiledQuery::Case(_) => 3,
            CompiledQuery::Parent(_) => 4,
            CompiledQuery::ParentId(_) => 1,
            CompiledQuery::Regex(_) => 6,
            // Reconstruye una ruta por entrada: siempre el último.
            CompiledQuery::Path(_) => 9,
            CompiledQuery::Not(inner) => inner.cost(),
            CompiledQuery::And(items) | CompiledQuery::Or(items) => {
                items.iter().map(|i| i.cost()).max().unwrap_or(0)
            }
        }
    }

    /// Reordena los términos de cada Y lógico de más barato a más caro.
    pub fn optimize(&mut self) {
        match self {
            CompiledQuery::And(items) => {
                for i in items.iter_mut() {
                    i.optimize();
                }
                items.sort_by_key(|i| i.cost());
            }
            CompiledQuery::Or(items) => {
                for i in items.iter_mut() {
                    i.optimize();
                }
            }
            CompiledQuery::Not(inner) => inner.optimize(),
            _ => {}
        }
    }
}
