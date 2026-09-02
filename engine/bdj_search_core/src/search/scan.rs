//! Barrido de la arena de nombres en bloque.
//!
//! El recorrido natural —abrir cada entrada y buscar el patrón dentro de su
//! nombre— hace una llamada a `memchr` por archivo. Sobre diez millones de
//! entradas son diez millones de llamadas sobre rodajas de cuarenta y cinco
//! bytes: demasiado cortas para que la búsqueda vectorizada llegue a amortizar
//! su preparación. El coste acaba siendo el de la llamada, no el de comparar.
//!
//! Aquí se hace al revés. Los nombres de un bloque ocupan una región
//! **contigua** de la arena, así que se recorre esa región entera de una vez
//! buscando la primera letra del patrón, a la anchura que dé el procesador, y
//! solo después se traduce cada acierto a la entrada que lo contiene. La
//! traducción es un cursor que avanza —los desplazamientos crecen con el
//! identificador—, sin búsquedas binarias.
//!
//! El barrido rápido solo cubre nombres ASCII. Los que llevan acentos, japonés
//! o emoji necesitan plegar mayúsculas con las reglas de Unicode, así que se
//! resuelven aparte, en una segunda pasada sobre la columna de banderas.

use crate::index::layout::FLAG_NON_ASCII;
use crate::index::view::IndexView;
use crate::search::matcher::SubstringMatcher;
use memchr::memchr2_iter;

/// Recorre `[start, end)` buscando `term` y deja en `out` los identificadores
/// que coinciden, en orden creciente.
///
/// Devuelve `false` si el bloque no cumple las condiciones del barrido rápido
/// —desplazamientos no crecientes, o algo fuera de rango—; en ese caso `out`
/// queda intacto y quien llame debe usar el recorrido entrada por entrada.
pub fn scan_names_bulk(
    view: &IndexView<'_>,
    term: &SubstringMatcher,
    start: usize,
    end: usize,
    out: &mut Vec<u32>,
) -> bool {
    let pattern = term.pattern_bytes();
    let plen = pattern.len();
    if plen == 0 || !term.is_ascii_pattern() || start >= end || end > view.entry_count() {
        return false;
    }

    let name_off = view.name_off;
    let name_len = view.name_len;
    let arena = view.name_arena;

    let region_start = name_off[start] as usize;
    let last = end - 1;
    let region_end = name_off[last] as usize + name_len[last] as usize;
    if region_start > region_end || region_end > arena.len() {
        return false;
    }

    let (needle_lo, needle_hi) = term.first_bytes();
    let region = &arena[region_start..region_end];

    let before = out.len();
    // Cursor sobre las entradas del bloque. Avanza y nunca retrocede, así que
    // traducir todos los aciertos del bloque cuesta, en total, un recorrido de
    // sus entradas.
    let mut entry = start;
    let mut prev_off = name_off[start] as usize;

    // `memchr2_iter` mantiene el estado del barrido vectorial entre aciertos.
    // Reiniciar la búsqueda en cada uno costaba la preparación completa: con un
    // patrón de una sola letra son decenas de millones de reinicios.
    for hit in memchr2_iter(needle_lo, needle_hi, region) {
        let abs = region_start + hit;
        if abs + plen > arena.len() {
            break;
        }

        // Primero la comprobación barata: comparar el patrón donde ha caído el
        // acierto. La inmensa mayoría se descarta en el segundo byte, sin tocar
        // el cursor ni ninguna otra columna.
        if !term.eq_at(&arena[abs..abs + plen]) {
            continue;
        }

        while entry < end {
            let off = name_off[entry] as usize;
            if off < prev_off {
                // Los desplazamientos deberían crecer con el identificador. Si
                // no lo hacen, el índice no cumple la invariante y este barrido
                // daría resultados equivocados: se abandona sin tocar `out`.
                out.truncate(before);
                return false;
            }
            prev_off = off;
            if off + name_len[entry] as usize > abs {
                break;
            }
            entry += 1;
        }
        if entry >= end {
            break;
        }

        let off = name_off[entry] as usize;
        let len = name_len[entry] as usize;

        // El patrón tiene que caber **dentro** de este nombre: dos nombres
        // consecutivos quedan pegados en la arena y una coincidencia a caballo
        // entre ambos no existe.
        if abs < off || abs + plen > off + len {
            continue;
        }
        // Los nombres con acentos o alfabetos no latinos se resuelven en la
        // segunda pasada, con plegado Unicode.
        if (view.flags[entry] & FLAG_NON_ASCII) != 0 {
            continue;
        }
        if out.last() == Some(&(entry as u32)) {
            // El patrón aparece más de una vez en el mismo nombre.
            continue;
        }
        if !view.is_alive(entry) {
            continue;
        }
        out.push(entry as u32);
    }

    // Segunda pasada: las entradas que necesitan plegado Unicode.
    //
    // Se recorre la columna de banderas, que son diez megas por cada diez
    // millones de entradas y se leen de forma secuencial. En un disco de
    // biblioteca musical típico casi ninguna entrada llega hasta aquí.
    let mut unicode_hits: Vec<u32> = Vec::new();
    for idx in start..end {
        if (view.flags[idx] & FLAG_NON_ASCII) == 0 {
            continue;
        }
        if !view.is_alive(idx) {
            continue;
        }
        if term.matches(view.get_name_bytes(idx), true) {
            unicode_hits.push(idx as u32);
        }
    }

    if !unicode_hits.is_empty() {
        // Ambas listas están ordenadas: se funden en una sola.
        let ascii_hits: Vec<u32> = out.split_off(before);
        let mut a = ascii_hits.into_iter().peekable();
        let mut u = unicode_hits.into_iter().peekable();
        loop {
            match (a.peek(), u.peek()) {
                (Some(&x), Some(&y)) => {
                    if x <= y {
                        out.push(x);
                        a.next();
                    } else {
                        out.push(y);
                        u.next();
                    }
                }
                (Some(_), None) => {
                    out.extend(a.by_ref());
                    break;
                }
                (None, Some(_)) => {
                    out.extend(u.by_ref());
                    break;
                }
                (None, None) => break,
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{IndexBuilder, MmapIndex};
    use std::io::BufWriter;
    use tempfile::NamedTempFile;

    fn indice_con(nombres: &[&str]) -> (NamedTempFile, MmapIndex) {
        let mut builder = IndexBuilder::new();
        builder.vol_table.add_or_update("C:\\", "test", "NTFS", true);
        for n in nombres {
            builder.add_entry(u32::MAX, n, false, false, false, 0, 1, 1, 1);
        }
        let temp = NamedTempFile::new().unwrap();
        let file = std::fs::File::create(temp.path()).unwrap();
        let mut w = BufWriter::new(file);
        builder.write_to(&mut w).unwrap();
        drop(w);
        let mmap = MmapIndex::open(temp.path()).unwrap();
        (temp, mmap)
    }

    /// Referencia lenta pero obviamente correcta.
    fn referencia(view: &IndexView<'_>, term: &SubstringMatcher) -> Vec<u32> {
        (0..view.entry_count())
            .filter(|&i| view.is_alive(i))
            .filter(|&i| {
                let non_ascii = (view.flags[i] & FLAG_NON_ASCII) != 0;
                term.matches(view.get_name_bytes(i), non_ascii)
            })
            .map(|i| i as u32)
            .collect()
    }

    #[test]
    fn el_barrido_en_bloque_coincide_con_el_recorrido_entrada_por_entrada() {
        let nombres = [
            "Michael Jackson - Billie Jean.mp3",
            "madonna.wav",
            "MICHAEL.flac",
            "Canción de Cuna.wav",
            "sin extension",
            "mmm.mp3",
            "Zzz.als",
            "Michael Michael Michael.wav",
            "ñam ñam.mp3",
            "",
        ];
        let (_t, mmap) = indice_con(&nombres);
        let view = mmap.view().unwrap();

        // Patrones ASCII: el barrido rápido se aplica y debe coincidir con la
        // referencia, incluidos los nombres con acentos o eñes del corpus.
        for patron in ["m", "michael", "z", "xyz", "n", "a", "ion", "cun"] {
            let term = SubstringMatcher::new(patron);
            let mut got = Vec::new();
            let ok = scan_names_bulk(&view, &term, 0, view.entry_count(), &mut got);
            assert!(ok, "el barrido debería aplicarse con el patrón «{patron}»");
            let want = referencia(&view, &term);
            assert_eq!(got, want, "discrepancia con el patrón «{patron}»");
        }
    }

    #[test]
    fn el_barrido_encuentra_cada_nombre_una_sola_vez() {
        let (_t, mmap) = indice_con(&["mmmmm.mp3", "mm.wav"]);
        let view = mmap.view().unwrap();
        let term = SubstringMatcher::new("m");
        let mut got = Vec::new();
        assert!(scan_names_bulk(&view, &term, 0, 2, &mut got));
        assert_eq!(got, vec![0, 1]);
    }

    #[test]
    fn el_patron_no_puede_cruzar_la_frontera_entre_dos_nombres() {
        // «ab» + «cd» quedan pegados en la arena como «abcd»: buscar «bc» no
        // debe encontrar nada.
        let (_t, mmap) = indice_con(&["ab", "cd"]);
        let view = mmap.view().unwrap();
        let term = SubstringMatcher::new("bc");
        let mut got = Vec::new();
        assert!(scan_names_bulk(&view, &term, 0, 2, &mut got));
        assert!(got.is_empty(), "no debe cruzar la frontera entre nombres");
    }

    #[test]
    fn respeta_el_rango_de_bloque() {
        let (_t, mmap) = indice_con(&["mA", "mB", "mC", "mD"]);
        let view = mmap.view().unwrap();
        let term = SubstringMatcher::new("m");
        let mut got = Vec::new();
        assert!(scan_names_bulk(&view, &term, 1, 3, &mut got));
        assert_eq!(got, vec![1, 2]);
    }

    #[test]
    fn un_patron_no_ascii_no_usa_el_barrido_rapido() {
        let (_t, mmap) = indice_con(&["canción.wav"]);
        let view = mmap.view().unwrap();
        let term = SubstringMatcher::new("ó");
        let mut got = Vec::new();
        assert!(!scan_names_bulk(&view, &term, 0, 1, &mut got));
    }
}
