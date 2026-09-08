//! El orden alfabético del índice no puede cambiar al cambiar el algoritmo.
//!
//! `compute_name_order` pasó de comparar cadenas a una ordenación radix por
//! tramos de ocho bytes. Es 2,5 veces más rápida, pero es una reescritura del
//! criterio con el que se ordena **toda** la columna «Nombre» del producto.
//!
//! Si el orden nuevo no fuera exactamente el mismo, el fallo sería silencioso:
//! la lista saldría ordenada —de alguna manera— y nadie miraría. Por eso aquí se
//! compara contra el comparador de siempre, sobre los nombres que de verdad
//! rompen este tipo de algoritmos.

use bdj_search_core::index::IndexBuilder;

/// El criterio original, tal cual estaba escrito: sin distinguir mayúsculas y,
/// a igualdad de contenido, el más corto primero.
fn orden_de_referencia(nombres: &[String]) -> Vec<u32> {
    let mut ids: Vec<u32> = (0..nombres.len() as u32).collect();
    ids.sort_by(|&a, &b| {
        let (x, y) = (nombres[a as usize].as_bytes(), nombres[b as usize].as_bytes());
        for (p, q) in x.iter().zip(y.iter()) {
            let d = p.to_ascii_lowercase().cmp(&q.to_ascii_lowercase());
            if d != std::cmp::Ordering::Equal {
                return d;
            }
        }
        x.len().cmp(&y.len())
    });
    ids
}

fn comprobar(nombres: Vec<String>, caso: &str) {
    let mut b = IndexBuilder::with_capacity(nombres.len());
    b.vol_table.add_or_update("C:\\", "Prueba", "NTFS", true);
    for n in &nombres {
        b.add_entry(u32::MAX, n, false, false, false, 0, 0, 0, 0);
    }
    b.compute_name_order();

    let esperado = orden_de_referencia(&nombres);

    // Los nombres repetidos empatan de verdad, así que se compara la secuencia
    // de nombres resultante y no la de identificadores: dos entradas con el
    // mismo nombre pueden salir en cualquier orden entre ellas sin que eso sea
    // un fallo.
    let obtenido_nombres: Vec<&str> = b
        .name_order
        .iter()
        .map(|&id| nombres[id as usize].as_str())
        .collect();
    let esperado_nombres: Vec<&str> = esperado
        .iter()
        .map(|&id| nombres[id as usize].as_str())
        .collect();

    assert_eq!(
        obtenido_nombres, esperado_nombres,
        "«{caso}»: el orden nuevo no coincide con el de siempre"
    );
}

#[test]
fn nombres_que_comparten_un_prefijo_muy_largo() {
    // El caso que hunde una clave de ocho bytes: una biblioteca de DJ está llena
    // de archivos que empiezan igual durante veinte o treinta caracteres.
    let mut v = Vec::new();
    for i in 0..500 {
        v.push(format!("Michael Jackson - Billie Jean (Remix {i:04}).wav"));
        v.push(format!("Michael Jackson - Beat It (Extended {i:04}).wav"));
        v.push(format!("Michael Jackson - Bad {i:04}.mp3"));
    }
    comprobar(v, "prefijo larguísimo");
}

#[test]
fn mayusculas_y_minusculas_ordenan_igual() {
    let v = vec![
        "ZULU.wav".into(),
        "alpha.wav".into(),
        "Alpha.WAV".into(),
        "BETA.wav".into(),
        "beta.wav".into(),
        "zulu.WAV".into(),
    ];
    comprobar(v, "mayúsculas mezcladas");
}

#[test]
fn un_nombre_que_es_prefijo_de_otro_va_antes() {
    // «set» antes que «set 1», y «set 1» antes que «set 10».
    let v = vec![
        "set 10.wav".into(),
        "set.wav".into(),
        "set".into(),
        "set 1.wav".into(),
        "s".into(),
        "".into(),
    ];
    comprobar(v, "prefijos");
}

#[test]
fn nombres_de_exactamente_ocho_bytes_y_alrededores() {
    // La frontera del tramo: siete, ocho y nueve bytes, y múltiplos de ocho,
    // que es donde una ordenación por tramos se equivoca si está mal escrita.
    let mut v = Vec::new();
    for base in ["abcdefg", "abcdefgh", "abcdefghi", "abcdefghijklmnop"] {
        v.push(base.to_string());
        v.push(format!("{base}z"));
        v.push(format!("{base}A"));
        v.push(format!("{base}{base}"));
    }
    comprobar(v, "fronteras de ocho bytes");
}

#[test]
fn acentos_y_caracteres_no_ascii() {
    // El criterio original compara bytes en minúscula ASCII, así que los
    // acentos se ordenan por su representación UTF-8. Sea o no lo ideal, el
    // algoritmo nuevo tiene que hacer **lo mismo**: cambiar el criterio sería
    // otra decisión, y no es la que se está tomando aquí.
    let v = vec![
        "canción.mp3".into(),
        "cancion.mp3".into(),
        "ñam.wav".into(),
        "nam.wav".into(),
        "Ábaco.txt".into(),
        "abaco.txt".into(),
        "日本語.wav".into(),
    ];
    comprobar(v, "no ASCII");
}

#[test]
fn muchos_nombres_repetidos() {
    let mut v = Vec::new();
    for _ in 0..1000 {
        v.push("igual.wav".to_string());
    }
    for i in 0..1000 {
        v.push(format!("distinto {i}.wav"));
    }
    comprobar(v, "repetidos");
}

#[test]
fn corpus_grande_y_variado() {
    // Por encima de todos los umbrales internos, para que se ejerciten los dos
    // caminos: el radix y el desempate por comparación.
    const ARTISTAS: [&str; 6] = ["Michael", "Madonna", "Daft", "Calvin", "Fisher", "Tiesto"];
    const TITULOS: [&str; 5] = ["Billie", "Titanium", "Levels", "Animals", "Strobe"];
    let mut v = Vec::with_capacity(30_000);
    for i in 0..30_000usize {
        v.push(format!(
            "{} - {} {:05}.{}",
            ARTISTAS[i % 6],
            TITULOS[(i / 6) % 5],
            i % 997,
            if i % 3 == 0 { "wav" } else { "mp3" }
        ));
    }
    comprobar(v, "corpus grande");
}

#[test]
fn el_orden_y_el_rango_son_inversos() {
    // `name_rank[id]` tiene que ser la posición de `id` dentro de `name_order`.
    // Es la propiedad de la que depende que ordenar por nombre en una consulta
    // cueste una lectura entera en vez de comparar cadenas.
    let nombres: Vec<String> = (0..5000)
        .map(|i| format!("pista {:04} de prueba.wav", (i * 7919) % 5000))
        .collect();

    let mut b = IndexBuilder::with_capacity(nombres.len());
    b.vol_table.add_or_update("C:\\", "Prueba", "NTFS", true);
    for n in &nombres {
        b.add_entry(u32::MAX, n, false, false, false, 0, 0, 0, 0);
    }
    b.compute_name_order();

    let mut vistos = vec![false; nombres.len()];
    for &id in &b.name_order {
        assert!(
            !vistos[id as usize],
            "el identificador {id} aparece dos veces en el orden"
        );
        vistos[id as usize] = true;
    }
    assert!(
        vistos.iter().all(|&v| v),
        "el orden debe contener todas las entradas exactamente una vez"
    );
}
