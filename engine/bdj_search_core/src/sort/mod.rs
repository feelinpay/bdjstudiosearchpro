//! Ordenación de resultados.
//!
//! Hay **un solo** ordenador, y es deliberado: `search::engine::SortKey` traduce
//! cada columna a una clave entera de 64 bits y `RadixSort::sort_pairs` la
//! ordena. Con dos ordenadores distintos —uno para elegir las mejores filas y
//! otro para ordenarlas— un resultado grande enseñaba unas filas y las ordenaba
//! con otro criterio, sin ningún aviso.

pub mod radix;

pub use radix::RadixSort;
