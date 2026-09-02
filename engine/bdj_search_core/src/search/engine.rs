use super::cancel::CancellationToken;
use super::refine::{MAX_CACHED_PER_ENTRY, RefinementStack};
use super::scan::scan_names_bulk;
use crate::index::view::IndexView;
use crate::query::compiled::MatchScratch;
use crate::query::{CompiledQuery, Parser};
use crate::sort::RadixSort;
use rayon::prelude::*;
use std::sync::Mutex;
use std::time::Instant;

/// Cuántas filas devuelve una búsqueda por defecto.
///
/// La tabla muestra unas treinta filas; dos mil dan margen de sobra para
/// desplazarse sin volver a preguntar. Si el usuario baja más allá, la interfaz
/// pide una ventana mayor con `search_with_limit`.
pub const DEFAULT_LIMIT: usize = 2_000;

/// Entradas por bloque del recorrido paralelo.
///
/// Sale del perfil de la máquina y no de una constante fija: un bloque tiene que
/// caber holgadamente en la caché de segundo nivel, y la de un portátil de dos
/// núcleos no se parece a la de un sobremesa. Se lee una sola vez.
fn chunk_size() -> usize {
    crate::tuning::Tuning::current().chunk_size
}

/// Grupo de hilos propio de la búsqueda, con un núcleo menos que la máquina.
///
/// Sin esto la búsqueda usa el grupo global de rayon, que ocupa **todos** los
/// núcleos. En un portátil de dos núcleos eso deja al hilo que dibuja la
/// interfaz sin dónde ejecutarse durante toda la consulta: se busca en cincuenta
/// milisegundos y se escribe a trompicones. Nadie percibe los cincuenta
/// milisegundos; todo el mundo percibe el tirón.
///
/// Si el grupo no se puede crear se sigue con el global: ir un poco a
/// trompicones es mejor que no buscar.
fn pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let hilos = crate::tuning::Tuning::current().search_threads;
        rayon::ThreadPoolBuilder::new()
            .num_threads(hilos)
            .thread_name(|i| format!("bdj-busqueda-{i}"))
            .build()
            .ok()
    })
    .as_ref()
}

/// Ejecuta `f` en el grupo de hilos de la búsqueda.
fn en_el_grupo<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    match pool() {
        Some(p) => p.install(f),
        None => f(),
    }
}

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
    /// Las mejores `limit` coincidencias, ya ordenadas.
    pub entry_ids: Vec<u32>,
    pub generation: u64,
    /// Número **exacto** de coincidencias, aunque `entry_ids` esté recortado.
    pub total_count: u32,
    pub elapsed_ms: u64,
    /// Cierto cuando hay más coincidencias de las devueltas.
    pub truncated: bool,
    /// Ventana pedida, para que quien llame sepa si merece la pena ampliarla.
    pub limit: u32,
}

impl SearchResult {
    fn empty(generation: u64, total_count: u32, elapsed_ms: u64, limit: usize) -> Self {
        Self {
            entry_ids: Vec::new(),
            generation,
            total_count,
            elapsed_ms,
            truncated: total_count > 0,
            limit: limit as u32,
        }
    }
}

/// De dónde sale la clave de ordenación de cada resultado.
///
/// Todas las claves acaban normalizadas a `u64` para poder invertirlas —y con
/// ellas el sentido del orden— con una sola resta.
enum KeySource<'a> {
    /// Posición alfabética precalculada. Ordenar por nombre sin comparar cadenas.
    U32(&'a [u32]),
    U64(&'a [u64]),
    /// Clave sacada de una tabla pequeña indexada por identificador de extensión.
    ///
    /// Sirve para ordenar por extensión y por tipo sin mirar ni una cadena: la
    /// tabla se construye una vez por consulta —hay unos pocos miles de
    /// extensiones distintas, no millones de archivos— y a partir de ahí cada
    /// entrada cuesta una lectura.
    ByExt {
        ext_id: &'a [u16],
        flags: &'a [u8],
        /// Desempate: a igualdad de extensión o de tipo, orden alfabético.
        name_rank: &'a [u32],
        /// Clave de las carpetas, que no tienen extensión.
        dir_key: u32,
        table: Vec<u32>,
    },
    /// Carpeta contenedora y, dentro de ella, nombre. Es la columna «Ruta».
    ///
    /// La ruta completa **no** se reconstruye para ordenar: hacerlo son dos
    /// cadenas nuevas por comparación, y sobre millones de resultados es la
    /// operación más cara del motor. Se ordena por el rango alfabético de la
    /// carpeta en los 32 bits altos y el del propio nombre en los bajos.
    ///
    /// El resultado es lo que se espera de una columna de ruta —agrupado por
    /// carpeta, alfabético dentro de cada una— y, sobre todo, es **la misma
    /// clave** que se usa para elegir las filas cuando hay más de las que caben.
    /// Dos carpetas distintas que se llamen igual se entremezclan; es la única
    /// diferencia frente a comparar la ruta entera, y sale barata.
    ParentThenName {
        parent: &'a [u32],
        name_rank: &'a [u32],
    },
    /// Sin columna: el propio identificador, que es el orden natural del índice.
    Id,
}

struct SortKey<'a> {
    source: KeySource<'a>,
    descending: bool,
}

impl SortKey<'_> {
    #[inline(always)]
    fn of(&self, id: u32) -> u64 {
        let raw = match &self.source {
            KeySource::U32(keys) => *keys.get(id as usize).unwrap_or(&0) as u64,
            KeySource::U64(keys) => *keys.get(id as usize).unwrap_or(&0),
            KeySource::ByExt {
                ext_id,
                flags,
                name_rank,
                dir_key,
                table,
            } => {
                let i = id as usize;
                let grupo = if flags
                    .get(i)
                    .is_some_and(|f| (f & crate::index::layout::FLAG_DIR) != 0)
                {
                    *dir_key as u64
                } else {
                    let e = *ext_id.get(i).unwrap_or(&0) as usize;
                    *table.get(e).unwrap_or(&0) as u64
                };
                (grupo << 32) | *name_rank.get(i).unwrap_or(&0) as u64
            }
            KeySource::ParentThenName { parent, name_rank } => {
                let propio = *name_rank.get(id as usize).unwrap_or(&0) as u64;
                let carpeta = match parent.get(id as usize) {
                    Some(&p) if p != u32::MAX => *name_rank.get(p as usize).unwrap_or(&0) as u64,
                    _ => 0,
                };
                (carpeta << 32) | propio
            }
            KeySource::Id => id as u64,
        };
        if self.descending { u64::MAX - raw } else { raw }
    }

    /// Materializa los pares (clave, id) en una sola pasada.
    ///
    /// `ids` viene en orden creciente, así que leer la columna de claves es un
    /// recorrido secuencial. Consultar `keys[id]` dentro del comparador serían
    /// accesos dispersos a un vector de decenas de megas: un fallo de caché por
    /// comparación.
    fn pairs(&self, ids: &[u32]) -> Vec<(u64, u32)> {
        ids.iter().map(|&id| (self.of(id), id)).collect()
    }

    /// Los `n` mejores de `ids`, sin ordenarlos entre sí.
    ///
    /// `select_nth_unstable` es lineal: no paga el `n log n` de ordenar un
    /// conjunto del que solo se van a mirar las primeras filas.
    fn best_of(&self, ids: &[u32], n: usize) -> Vec<u32> {
        if ids.len() <= n {
            return ids.to_vec();
        }
        let mut pairs = self.pairs(ids);
        pairs.select_nth_unstable(n);
        pairs.truncate(n);
        pairs.into_iter().map(|p| p.1).collect()
    }

    fn keep_best(&self, ids: &mut Vec<u32>, n: usize) {
        if ids.len() <= n {
            return;
        }
        *ids = self.best_of(ids, n);
    }

    /// Ordena `ids` por esta clave.
    ///
    /// Es el **único** camino de ordenación del motor. Tener uno solo es lo que
    /// garantiza que elegir las mejores `N` filas y ordenarlas después dé
    /// exactamente la cabeza del resultado completo: con dos comparadores
    /// distintos, un resultado grande enseñaba unas filas y las ordenaba con
    /// otro criterio, sin ningún aviso.
    fn sort(&self, ids: &mut [u32]) {
        let mut pairs = self.pairs(ids);
        RadixSort::sort_pairs(&mut pairs);
        for (slot, pair) in ids.iter_mut().zip(pairs.iter()) {
            *slot = pair.1;
        }
    }
}

/// Lo que devuelve cada bloque del recorrido paralelo.
struct ChunkHits {
    count: usize,
    ids: Vec<u32>,
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

    /// Reserva un número de resultado nuevo y cancela lo que hubiera en vuelo.
    ///
    /// La ordenación de un conjunto ya conocido —los hijos de una carpeta, que
    /// el explorador saca de la columna de hijos y de la capa de cambios— no
    /// pasa por el motor, pero su resultado se guarda y se lee con las mismas
    /// funciones que el de una búsqueda. Necesita, por tanto, un número de la
    /// misma serie: sin esto, dos resultados distintos podrían compartir número
    /// y la interfaz pintaría las filas de uno creyendo que son del otro.
    ///
    /// No toca la pila de refinamiento: esa guarda resultados del índice base,
    /// que aquí no cambia.
    pub fn next_generation(&self) -> u64 {
        self.token.bump()
    }

    pub fn cancel_all(&self) -> u64 {
        let cancel_gen = self.token.bump();
        let mut stack = self.refine_stack.lock().unwrap();
        stack.clear();
        cancel_gen
    }

    /// Búsqueda con la ventana por defecto.
    pub fn search(
        &self,
        view: &IndexView,
        query: &str,
        sort_col: u8,
        ascending: bool,
    ) -> (u64, SearchResult) {
        self.search_with_limit(view, query, sort_col, ascending, DEFAULT_LIMIT)
    }

    /// Devuelve el conteo exacto de coincidencias y las `limit` mejores, ya
    /// ordenadas.
    ///
    /// El motor anterior recolectaba **todos** los identificadores en un vector
    /// y ordenaba el conjunto entero antes de devolver nada. Con diez millones
    /// de entradas, la letra «m» producía casi nueve millones de resultados que
    /// había que materializar y ordenar para pintar treinta filas: trescientos
    /// milisegundos donde el presupuesto son treinta.
    ///
    /// Aquí cada bloque se recorta a sus mejores `limit` antes de fundirse con
    /// los demás, así que el coste de ordenar depende de la ventana pedida y no
    /// del número de coincidencias. El conteo total sigue siendo exacto porque
    /// se suma durante el recorrido, que hay que hacer de todos modos.
    pub fn search_with_limit(
        &self,
        view: &IndexView,
        query: &str,
        sort_col: u8,
        ascending: bool,
        limit: usize,
    ) -> (u64, SearchResult) {
        let start = Instant::now();
        let search_gen = self.token.bump();
        let limit = limit.max(1);
        let trimmed = query.trim();

        let count = view.entry_count();
        if trimmed.is_empty() || count == 0 {
            // Sin consulta no hay resultados: cero.
            //
            // Devolver aquí el número de entradas vivas hacía que la tabla
            // creyera que tenía diez millones de filas que pintar nada más
            // arrancar, con la barra de desplazamiento entera y ninguna fila que
            // enseñar. Cuántas entradas hay indexadas se pregunta aparte, con
            // `engine_status`.
            return (
                search_gen,
                SearchResult::empty(search_gen, 0, Self::ms(start), limit),
            );
        }

        let ast = Parser::parse(trimmed);

        // La consulta se compila UNA vez: comparadores, expresiones regulares y
        // conjuntos de extensiones quedan listos antes de tocar la primera
        // entrada. Y sus términos se reordenan del filtro más barato al más caro.
        let mut compiled = CompiledQuery::compile(&ast, view);
        compiled.optimize();

        // ¿Hay un resultado anterior que contenga con seguridad a este?
        let candidates = {
            let stack = self.refine_stack.lock().unwrap();
            stack.find_candidate(trimmed, &ast).map(|s| s.to_vec())
        };

        let chunks: Vec<ChunkHits> = match candidates {
            Some(prev) => Self::scan_candidates(view, &compiled, &prev, &self.token, search_gen),
            None => Self::scan_all(view, &compiled, count, &self.token, search_gen),
        };

        if self.token.is_cancelled(search_gen) {
            return (
                search_gen,
                SearchResult::empty(search_gen, 0, Self::ms(start), limit),
            );
        }

        let total: usize = chunks.iter().map(|c| c.count).sum();
        let key = Self::sort_key(view, sort_col, ascending);

        if total <= limit {
            // Cabe entero en la ventana: se funde, se ordena y se guarda para
            // refinar la siguiente pulsación.
            let mut all = Vec::with_capacity(total);
            for c in &chunks {
                all.extend_from_slice(&c.ids);
            }
            key.sort(&mut all);
            self.remember(trimmed, &ast, &all);
            return (
                search_gen,
                SearchResult {
                    entry_ids: all,
                    generation: search_gen,
                    total_count: total as u32,
                    elapsed_ms: Self::ms(start),
                    truncated: false,
                    limit: limit as u32,
                },
            );
        }

        // Hay más coincidencias que filas pedidas. Cada bloque aporta solo sus
        // `limit` mejores: el mejor global está forzosamente entre ellos, y así
        // el coste de ordenar depende de la ventana y no del número de aciertos.
        let mut merged: Vec<u32> = chunks
            .par_iter()
            .map(|c| key.best_of(&c.ids, limit))
            .reduce(Vec::new, |mut a, b| {
                a.extend_from_slice(&b);
                a
            });

        // El conjunto completo, sin ordenar, alimenta el refinamiento de la
        // siguiente pulsación. Solo se copia si cabe dentro del presupuesto.
        if total <= MAX_CACHED_PER_ENTRY {
            let mut all = Vec::with_capacity(total);
            for c in &chunks {
                all.extend_from_slice(&c.ids);
            }
            self.remember(trimmed, &ast, &all);
        }
        drop(chunks);

        key.keep_best(&mut merged, limit);
        key.sort(&mut merged);
        let returned = merged.len().min(limit);
        merged.truncate(returned);

        (
            search_gen,
            SearchResult {
                entry_ids: merged,
                generation: search_gen,
                total_count: total as u32,
                elapsed_ms: Self::ms(start),
                truncated: total > returned,
                limit: limit as u32,
            },
        )
    }

    /// Guarda un resultado completo para poder refinar la siguiente pulsación.
    fn remember(&self, query: &str, ast: &crate::query::QueryAst, ids: &[u32]) {
        if ids.len() > MAX_CACHED_PER_ENTRY {
            return;
        }
        let mut stack = self.refine_stack.lock().unwrap();
        stack.push(query.to_string(), ast.clone(), ids.to_vec());
    }

    /// Ordena y recorta un conjunto de identificadores ya conocido.
    ///
    /// Es lo que necesita el explorador: los hijos de una carpeta salen del
    /// índice sin filtrar nada, pero tienen que ordenarse y paginarse con las
    /// mismas reglas que un resultado de búsqueda. Compartir este camino evita
    /// que la tabla acabe teniendo dos formas distintas de pintar lo mismo.
    ///
    /// No alimenta la pila de refinamiento: navegar no es teclear, y guardar
    /// estos conjuntos solo serviría para que la siguiente búsqueda partiera de
    /// un candidato equivocado.
    pub fn rank(
        &self,
        view: &IndexView,
        mut ids: Vec<u32>,
        sort_col: u8,
        ascending: bool,
        limit: usize,
    ) -> (u64, SearchResult) {
        let start = Instant::now();
        let gen_id = self.token.bump();
        let limit = limit.max(1);
        let total = ids.len();
        let key = Self::sort_key(view, sort_col, ascending);

        if total <= limit {
            key.sort(&mut ids);
        } else {
            key.keep_best(&mut ids, limit);
            key.sort(&mut ids);
        }

        let returned = ids.len().min(limit);
        ids.truncate(returned);

        (
            gen_id,
            SearchResult {
                entry_ids: ids,
                generation: gen_id,
                total_count: total as u32,
                elapsed_ms: Self::ms(start),
                truncated: total > returned,
                limit: limit as u32,
            },
        )
    }

    fn ms(start: Instant) -> u64 {
        start.elapsed().as_millis() as u64
    }

    /// Recorrido completo del índice, por bloques y en paralelo.
    fn scan_all(
        view: &IndexView,
        compiled: &CompiledQuery,
        count: usize,
        token: &CancellationToken,
        gen_id: u64,
    ) -> Vec<ChunkHits> {
        let tam_bloque = chunk_size();
        let num_chunks = count.div_ceil(tam_bloque);
        // Si la consulta obliga a que el nombre contenga un texto concreto, ese
        // texto conduce el barrido: se recorre la arena de nombres en bloque,
        // con instrucciones vectoriales, en vez de abrir un nombre por archivo.
        let driver = compiled.driving_term();
        let driver_is_everything = compiled.is_only_term();

        // Todo el recorrido paralelo va en el grupo de hilos de la búsqueda,
        // que deja un núcleo libre para que la interfaz siga dibujando.
        en_el_grupo(|| {
        (0..num_chunks)
            .into_par_iter()
            .map(|chunk_idx| {
                if token.is_cancelled(gen_id) {
                    return ChunkHits {
                        count: 0,
                        ids: Vec::new(),
                    };
                }
                // Un juego de buffers por bloque: los filtros que necesitan la
                // ruta completa no reservan memoria por archivo.
                let mut scratch = MatchScratch::new();
                let start_idx = chunk_idx * tam_bloque;
                let end_idx = (start_idx + tam_bloque).min(count);
                let mut ids = Vec::new();

                let bulk_done = match driver {
                    Some(term) => scan_names_bulk(view, term, start_idx, end_idx, &mut ids),
                    None => false,
                };

                if bulk_done {
                    if !driver_is_everything {
                        // El barrido solo garantiza el término conductor; el
                        // resto de la consulta se comprueba sobre los pocos
                        // candidatos que han sobrevivido.
                        ids.retain(|&id| compiled.matches(view, id as usize, &mut scratch));
                    }
                } else {
                    for idx in start_idx..end_idx {
                        if !view.is_alive(idx) {
                            continue;
                        }
                        if compiled.matches(view, idx, &mut scratch) {
                            ids.push(idx as u32);
                        }
                    }
                }

                ChunkHits {
                    count: ids.len(),
                    ids,
                }
            })
            .collect()
        })
    }

    /// Refinamiento: solo se vuelven a evaluar los supervivientes de la
    /// pulsación anterior.
    fn scan_candidates(
        view: &IndexView,
        compiled: &CompiledQuery,
        candidates: &[u32],
        token: &CancellationToken,
        gen_id: u64,
    ) -> Vec<ChunkHits> {
        en_el_grupo(|| {
        candidates
            .par_chunks(chunk_size())
            .map(|slice| {
                if token.is_cancelled(gen_id) {
                    return ChunkHits {
                        count: 0,
                        ids: Vec::new(),
                    };
                }
                let mut scratch = MatchScratch::new();
                let mut ids = Vec::new();
                for &id in slice {
                    let u_idx = id as usize;
                    if !view.is_alive(u_idx) {
                        continue;
                    }
                    if compiled.matches(view, u_idx, &mut scratch) {
                        ids.push(id);
                    }
                }
                ChunkHits {
                    count: ids.len(),
                    ids,
                }
            })
            .collect()
        })
    }

    fn sort_key<'a>(view: &'a IndexView, sort_col: u8, ascending: bool) -> SortKey<'a> {
        let source = match sort_col {
            // Nombre: el rango alfabético precalculado. Un índice sin esa
            // columna se rechaza al abrirlo, así que este camino siempre está
            // disponible; la guarda es por si acaso.
            0 if view.name_rank.len() >= view.entry_count() => KeySource::U32(view.name_rank),
            1 if view.name_rank.len() >= view.entry_count() => KeySource::ParentThenName {
                parent: view.parent,
                name_rank: view.name_rank,
            },
            2 => KeySource::ByExt {
                ext_id: view.ext_id,
                flags: view.flags,
                name_rank: view.name_rank,
                // Una carpeta no tiene extensión: va con las cadenas vacías.
                dir_key: 0,
                table: Self::extension_rank_table(view),
            },
            3 => KeySource::U64(view.size),
            4 => KeySource::U32(view.mtime),
            5 => KeySource::ByExt {
                ext_id: view.ext_id,
                flags: view.flags,
                name_rank: view.name_rank,
                // Las carpetas van primero, como en cualquier explorador.
                dir_key: 0,
                table: Self::type_rank_table(view),
            },
            _ => KeySource::Id,
        };
        SortKey {
            source,
            descending: !ascending,
        }
    }

    /// Posición alfabética de cada extensión, indexada por su identificador.
    ///
    /// Los identificadores de extensión se asignan por orden de aparición, no
    /// alfabéticamente, así que hace falta esta traducción. La tabla tiene
    /// tantas entradas como extensiones distintas haya en el disco —unos pocos
    /// miles— y se construye una vez por consulta.
    fn extension_rank_table(view: &IndexView) -> Vec<u32> {
        let n = view.ext_table.len();
        let mut orden: Vec<u16> = (0..n as u16).collect();
        orden.sort_by(|&a, &b| {
            let ea = view.ext_table.get_name(a).unwrap_or("");
            let eb = view.ext_table.get_name(b).unwrap_or("");
            unicase::Ascii::new(ea).cmp(&unicase::Ascii::new(eb))
        });
        let mut tabla = vec![0u32; n];
        for (rango, &id) in orden.iter().enumerate() {
            tabla[id as usize] = rango as u32;
        }
        tabla
    }

    /// Categoría de cada extensión: 1 audio, 2 vídeo, 3 imagen, 4 documento,
    /// 5 proyecto, 6 comprimido, 7 aplicación, 8 el resto. Las carpetas son 0.
    fn type_rank_table(view: &IndexView) -> Vec<u32> {
        let n = view.ext_table.len();
        let mut tabla = vec![8u32; n];
        for (id, celda) in tabla.iter_mut().enumerate() {
            let ext = view.ext_table.get_name(id as u16).unwrap_or("");
            *celda = categoria_de_extension(ext);
        }
        tabla
    }

}

/// Categoría de una extensión, en el orden en que se muestran los tipos.
///
/// Está fuera del motor porque la comparten la tabla de claves y la ordenación
/// de la ventana, y tener dos copias es justo lo que hace que un resultado
/// grande se ordene distinto que uno pequeño.
fn categoria_de_extension(ext: &str) -> u32 {
    match ext.to_ascii_lowercase().as_str() {
        "wav" | "flac" | "aiff" | "aif" | "mp3" | "m4a" | "ogg" | "opus" | "wma" | "alac"
        | "ape" | "wv" | "aac" => 1,
        "mp4" | "mov" | "avi" | "mkv" | "m4v" | "webm" | "mpg" | "mpeg" | "wmv" | "flv" => 2,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "tiff" | "bmp" | "svg" | "psd" => 3,
        "pdf" | "docx" | "doc" | "txt" | "rtf" | "odt" | "xlsx" | "pptx" | "md" => 4,
        "als" | "flp" | "ptx" | "cpr" | "logicx" | "rpp" | "nki" | "nkm" | "cue" | "m3u"
        | "m3u8" => 5,
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" => 6,
        "exe" | "msi" | "app" | "dll" | "dmg" | "pkg" | "bat" | "cmd" | "sh" => 7,
        _ => 8,
    }
}
