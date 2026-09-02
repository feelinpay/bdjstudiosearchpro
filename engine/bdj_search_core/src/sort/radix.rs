/// Ordenamiento por dígitos (LSD radix) sobre los identificadores de resultado.
///
/// El coste es proporcional al número de resultados, no al tamaño del índice, y
/// no hace ni una comparación de cadenas: las claves salen de las columnas del
/// índice, que ya están en memoria.
///
/// Dos detalles importan más de lo que parece:
///
/// 1. Las claves se **materializan una sola vez**, recorriendo la columna en
///    orden creciente de identificador. Mirar `keys[id]` dentro del bucle de
///    ordenación son accesos dispersos a un vector de decenas de megas: un fallo
///    de caché por elemento y por pasada. Copiarlas antes cuesta una pasada
///    secuencial y ahorra todas las demás.
/// 2. La versión original se llamaba `RadixSort` pero por dentro era
///    `sort_unstable_by_key`. Funcionaba, pero el nombre mentía.
pub struct RadixSort;

/// Por debajo de este tamaño gana una ordenación por comparación: el radix tiene
/// que reservar el vector auxiliar y hacer cuatro u ocho pasadas completas.
const COMPARISON_THRESHOLD: usize = 512;

/// Una pasada de conteo sobre el byte `shift` de la clave.
#[inline]
fn radix_pass(src: &[(u64, u32)], dst: &mut [(u64, u32)], shift: u32) {
    let mut counts = [0u32; 256];
    for &(k, _) in src {
        counts[((k >> shift) & 0xFF) as usize] += 1;
    }
    let mut total = 0u32;
    for c in counts.iter_mut() {
        let count = *c;
        *c = total;
        total += count;
    }
    for &pair in src {
        let byte = ((pair.0 >> shift) & 0xFF) as usize;
        dst[counts[byte] as usize] = pair;
        counts[byte] += 1;
    }
}

/// Ordena `pairs` (clave, id) por clave, de menor a mayor, con `passes` pasadas
/// de 8 bits.
fn radix_sort_pairs(pairs: &mut Vec<(u64, u32)>, passes: u32) {
    let n = pairs.len();
    let mut buffer = vec![(0u64, 0u32); n];
    let mut src_is_pairs = true;
    for p in 0..passes {
        let shift = p * 8;
        if src_is_pairs {
            radix_pass(pairs, &mut buffer, shift);
        } else {
            radix_pass(&buffer, pairs, shift);
        }
        src_is_pairs = !src_is_pairs;
    }
    if !src_is_pairs {
        // El resultado quedó en `buffer`.
        std::mem::swap(pairs, &mut buffer);
    }
}

impl RadixSort {
    /// Ordena pares (clave, id) por clave, de menor a mayor.
    ///
    /// Es **estable**: a igualdad de clave se conserva el orden de entrada. Los
    /// identificadores llegan siempre en orden creciente desde el recorrido, así
    /// que los empates quedan resueltos por identificador, igual que haría una
    /// comparación de la tupla entera. Que las dos rutas empaten igual es lo que
    /// permite que una ventana recortada coincida con la cabeza del resultado
    /// completo.
    pub fn sort_pairs(pairs: &mut Vec<(u64, u32)>) {
        if pairs.len() < 2 {
            return;
        }
        if pairs.len() < COMPARISON_THRESHOLD {
            pairs.sort_unstable();
            return;
        }
        radix_sort_pairs(pairs, 8);
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    /// Referencia obviamente correcta: ordenar los pares comparando la tupla.
    fn referencia(pairs: &[(u64, u32)]) -> Vec<(u64, u32)> {
        let mut v = pairs.to_vec();
        v.sort_unstable();
        v
    }

    fn pares(n: usize, modulo: u64) -> Vec<(u64, u32)> {
        (0..n)
            .map(|i| {
                let k = (i as u64).wrapping_mul(6_364_136_223_846_793_005) >> 11;
                ((k % modulo), i as u32)
            })
            .collect()
    }

    #[test]
    fn coincide_con_la_ordenacion_por_comparacion() {
        // Por debajo y por encima del umbral, y en los bordes.
        for n in [0usize, 1, 2, 511, 512, 513, 10_000, 100_000] {
            let mut v = pares(n, u64::MAX);
            let esperado = referencia(&v);
            RadixSort::sort_pairs(&mut v);
            assert_eq!(v, esperado, "falla con n={n}");
        }
    }

    #[test]
    fn ordena_claves_muy_grandes() {
        // Tamaños de archivo por encima de 4 GB: hacen falta las ocho pasadas.
        let base: [u64; 6] = [u64::MAX, 0, 5_000_000_000, 1, 4_294_967_296, 5_000_000_001];
        let mut v: Vec<(u64, u32)> = base
            .iter()
            .cycle()
            .take(1000)
            .enumerate()
            .map(|(i, &k)| (k, i as u32))
            .collect();
        let esperado = referencia(&v);
        RadixSort::sort_pairs(&mut v);
        assert_eq!(v, esperado);
        assert!(v.windows(2).all(|w| w[0].0 <= w[1].0));
    }

    #[test]
    fn con_claves_repetidas_conserva_el_orden_de_entrada() {
        // Es la propiedad de la que depende que una ventana recortada coincida
        // con la cabeza del resultado completo: los empates tienen que
        // resolverse igual por los dos caminos.
        let mut v: Vec<(u64, u32)> = (0..5000u32).map(|i| (7u64, i)).collect();
        RadixSort::sort_pairs(&mut v);
        let ids: Vec<u32> = v.iter().map(|p| p.1).collect();
        assert_eq!(ids, (0..5000u32).collect::<Vec<_>>());
    }

    #[test]
    fn no_pierde_ni_duplica_ningun_identificador() {
        let n = 50_000usize;
        let mut v = pares(n, 1000);
        RadixSort::sort_pairs(&mut v);
        assert_eq!(v.len(), n);
        let mut vistos: Vec<u32> = v.iter().map(|p| p.1).collect();
        vistos.sort_unstable();
        assert_eq!(vistos, (0..n as u32).collect::<Vec<_>>());
    }
}
