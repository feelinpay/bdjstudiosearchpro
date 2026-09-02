#![allow(clippy::not_unsafe_ptr_arg_deref)]

pub mod api;
pub mod diagnostics;
pub mod frb_generated;
pub mod rowsrc;
pub mod service;

pub use api::*;

#[cfg(test)]
mod tests {
    use super::*;

    /// Diagnóstico manual sobre el índice real de la máquina.
    ///
    /// Está ignorado a propósito: depende de que el servicio haya publicado un
    /// índice, así que en integración continua fallaría siempre. Antes no lo
    /// estaba y hacía fallar `cargo test` en cualquier equipo limpio.
    ///
    /// Para ejecutarlo:
    /// `cargo test -p bdj_search_ffi -- --ignored --nocapture`
    #[test]
    #[ignore = "necesita un indice publicado por el servicio"]
    fn inspeccionar_el_indice_de_esta_maquina() {
        let estado = api::engine_status();
        println!("ruta:    {}", estado.index_path);
        println!("existe:  {} ({} bytes)", estado.file_exists, estado.file_size);
        println!("mensaje: {}", estado.message);

        let res = api::engine_open(String::new());
        assert!(res.is_ok(), "no se pudo abrir el indice: {res:?}");

        let estado = api::engine_status();
        println!(
            "abierto: generacion {}, {} entradas",
            estado.generation, estado.entry_count
        );

        let g = api::search("spot".to_string(), 0, true);
        let st = api::search_status(g);
        println!(
            "«spot» -> {} coincidencias, {} filas, {} ms",
            st.total_count, st.ready_count, st.elapsed_ms
        );
        let filas = api::rows(g, 0, 20);
        for i in 0..filas.names.len() {
            println!("  {} · {}", filas.names[i], filas.paths[i]);
        }

        for path in [
            r"C:\Users\David Zapata\Downloads",
            r"C:\Users\David Zapata\Documents",
            r"C:\Users\David Zapata",
            r"C:\Users",
            r"C:\",
        ] {
            let bg = api::browse_path(path.to_string(), 0, true, 50);
            let bst = api::search_status(bg);
            println!(
                "BROWSE '{path}' -> {} coincidencias, {} filas",
                bst.total_count, bst.ready_count
            );
            let bfilas = api::rows(bg, 0, 10);
            for i in 0..bfilas.names.len() {
                println!("    [{}] {} ({})", if (bfilas.flags[i] & 1) != 0 { "DIR" } else { "FILE" }, bfilas.names[i], bfilas.paths[i]);
            }
        }
    }
}
