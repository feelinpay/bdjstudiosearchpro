use crate::query::ast::QueryAst;
use crate::query::eval::QueryEvaluator;

/// Cuántas filas como mucho se guardan por peldaño de la pila de refinamiento.
///
/// Un peldaño guarda los identificadores que casaron con una consulta anterior
/// para no repetir el recorrido al teclear la letra siguiente. Sin este tope,
/// escribir una letra que casa con nueve millones de archivos guardaba nueve
/// millones de enteros —36 MB— por cada pulsación, y la pila conserva dieciséis
/// peldaños: medio giga de memoria por escribir una palabra.
///
/// Por encima del tope no se guarda nada: repetir el recorrido de una consulta
/// tan ancha es más barato que arrastrar su resultado.
pub fn max_cached_per_entry() -> usize {
    crate::tuning::Tuning::current().max_cached_refine
}

pub const MAX_CACHED_PER_ENTRY: usize = 2_000_000;

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
            // `implied_by(nueva, vieja)`: todo lo que cumple la consulta nueva
            // cumplía ya la vieja, así que el resultado nuevo es un subconjunto
            // del guardado y basta con filtrarlo.
            if new_query.starts_with(&entry.query)
                && QueryEvaluator::implied_by(new_ast, &entry.ast)
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
