//! Ajustes derivados de la máquina donde se está ejecutando.
//!
//! # El problema
//!
//! El motor venía calibrado, sin decirlo, para un equipo de sobremesa con
//! muchos núcleos: bloques de 65 536 entradas, todos los núcleos ocupados
//! buscando, dos mil filas pedidas de entrada. En un portátil de dos núcleos eso
//! tiene una consecuencia que el usuario nota enseguida: mientras dura la
//! búsqueda **no queda ni un núcleo para dibujar**, así que escribir de corrido
//! da tirones. Se busca en cincuenta milisegundos y se escribe a trompicones,
//! que es el peor reparto posible: nadie percibe los cincuenta milisegundos y
//! todo el mundo percibe el tirón.
//!
//! Aquí se leen las características reales del equipo una sola vez y de ahí
//! salen las constantes. No hay una lista de modelos ni una detección de marca:
//! solo núcleos y memoria, que es lo que de verdad cambia el reparto.
//!
//! # Los tres perfiles
//!
//! | | Núcleos | Memoria | Hilos de búsqueda | Bloque |
//! |---|---:|---:|---:|---:|
//! | Baja | ≤ 2 | < 8 GB | 1 (deja el otro para la interfaz) | 16 384 |
//! | Media | 3–7 | ≥ 8 GB | núcleos − 1 | 32 768 |
//! | Alta | ≥ 8 | ≥ 16 GB | núcleos − 1 | 65 536 |
//!
//! **Siempre se deja un núcleo libre.** Es la decisión que más se nota: la
//! interfaz de Flutter dibuja en su propio hilo, y si la búsqueda ocupa todos
//! los núcleos ese hilo se queda esperando. Buscar un diez por ciento más lento
//! y escribir con fluidez es mejor trato para quien usa el programa.

use std::sync::OnceLock;

/// Gama del equipo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Low,
    Mid,
    High,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Low => "baja",
            Tier::Mid => "media",
            Tier::High => "alta",
        }
    }
}

/// Constantes derivadas de la máquina.
#[derive(Debug, Clone, Copy)]
pub struct Tuning {
    pub tier: Tier,
    pub cores: usize,
    pub memory_mb: u64,

    /// Cuántos hilos puede usar la búsqueda.
    ///
    /// Nunca todos: uno se reserva para que la interfaz siga dibujando.
    pub search_threads: usize,

    /// Entradas por bloque del recorrido paralelo.
    ///
    /// Un bloque tiene que caber holgadamente en la caché de segundo nivel. En
    /// equipos con caché pequeña, bloques más chicos rinden más aunque haya más
    /// bloques que repartir.
    pub chunk_size: usize,

    /// Filas que la interfaz pide de entrada.
    ///
    /// Pedir dos mil para pintar treinta es trabajo tirado en un equipo lento.
    pub initial_limit: usize,

    /// Páginas de filas que se mantienen en memoria.
    pub cached_pages: usize,

    /// Bloque de copia de archivos.
    ///
    /// En un disco mecánico, bloques de un mega dan un progreso a saltos; más
    /// pequeños, la barra avanza de forma continua y el usuario no cree que se
    /// ha colgado.
    pub copy_chunk: usize,

    /// Cuánto espera el servicio a que se calme el disco antes de compactar.
    pub quiet_period_ms: u64,

    /// Cuántas entradas se construyen en memoria antes de volcar a disco.
    pub build_batch: usize,

    /// Cuántas filas coincidentes se pueden guardar en la pila de refinamiento.
    pub max_cached_refine: usize,
}

impl Tuning {
    /// Deriva los ajustes de un número de núcleos y una memoria dados.
    ///
    /// Se pasa como argumento —en vez de leerlo aquí dentro— para poder probar
    /// las tres gamas sin necesidad de tres máquinas.
    pub fn derive(cores: usize, memory_mb: u64) -> Self {
        let cores = cores.max(1);
        let tier = if cores <= 2 || memory_mb < 7 * 1024 {
            Tier::Low
        } else if cores >= 8 && memory_mb >= 14 * 1024 {
            Tier::High
        } else {
            Tier::Mid
        };

        // Siempre un núcleo libre para la interfaz. Con un solo núcleo no hay
        // nada que repartir y la búsqueda va en el mismo, que es lo correcto:
        // más hilos que núcleos solo añade cambios de contexto.
        let search_threads = if cores <= 1 { 1 } else { cores - 1 };

        let (chunk_size, initial_limit, cached_pages, copy_chunk) = match tier {
            Tier::Low => (16_384, 500, 5, 256 * 1024),
            Tier::Mid => (32_768, 1_000, 8, 512 * 1024),
            Tier::High => (65_536, 2_000, 10, 1024 * 1024),
        };

        // La memoria disponible manda sobre el tamaño del lote de construcción.
        //
        // Se reserva **una octava parte** de la RAM, no una cuarta: durante el
        // primer escaneo el equipo no está haciendo solo esto —hay un sistema
        // operativo, un navegador y probablemente el propio programa abierto—, y
        // una cuarta parte de 4 GB ya empuja a paginar. Cada entrada ocupa unos
        // 65 bytes entre todas sus columnas.
        //
        // El techo de cinco millones es deliberado incluso en equipos grandes:
        // por encima de eso el beneficio de un lote mayor es marginal y el
        // riesgo de un pico de memoria no lo es.
        const BYTES_POR_ENTRADA: u64 = 65;
        let presupuesto = (memory_mb.max(1024) * 1024 * 1024) / 8;
        let build_batch =
            ((presupuesto / BYTES_POR_ENTRADA) as usize).clamp(250_000, 5_000_000);

        Self {
            tier,
            cores,
            memory_mb,
            search_threads,
            chunk_size,
            initial_limit,
            cached_pages,
            copy_chunk,
            quiet_period_ms: match tier {
                Tier::Low => 1_500,
                Tier::Mid => 750,
                Tier::High => 400,
            },
            build_batch,
            max_cached_refine: match tier {
                Tier::Low => 500_000,
                Tier::Mid => 1_500_000,
                Tier::High => 3_000_000,
            },
        }
    }

    /// Deriva los ajustes de un perfil concreto, ignorando el hardware real.
    pub fn derive_for_tier(tier: Tier) -> Self {
        match tier {
            Tier::Low => Self::derive(2, 4 * 1024),
            Tier::Mid => Self::derive(4, 8 * 1024),
            Tier::High => Self::derive(8, 16 * 1024),
        }
    }

    /// Los ajustes de esta máquina. Se calculan una sola vez.
    pub fn current() -> &'static Tuning {
        static ACTUAL: OnceLock<Tuning> = OnceLock::new();
        ACTUAL.get_or_init(|| {
            if let Ok(val) = std::env::var("BDJ_TIER") {
                match val.to_lowercase().as_str() {
                    "low" | "baja" => return Self::derive_for_tier(Tier::Low),
                    "mid" | "media" => return Self::derive_for_tier(Tier::Mid),
                    "high" | "alta" => return Self::derive_for_tier(Tier::High),
                    _ => {}
                }
            }
            let cores = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(2);
            Self::derive(cores, total_memory_mb())
        })
    }
}

/// Memoria física total, en megabytes.
///
/// Si no se puede averiguar se devuelve una cifra conservadora: equivocarse por
/// abajo hace que el programa vaya un poco más despacio, y equivocarse por
/// arriba hace que se quede sin memoria durante el primer escaneo.
pub fn total_memory_mb() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(texto) = std::fs::read_to_string("/proc/meminfo") {
            for linea in texto.lines() {
                if let Some(resto) = linea.strip_prefix("MemTotal:")
                    && let Some(kb) = resto.split_whitespace().next()
                    && let Ok(v) = kb.parse::<u64>()
                {
                    return v / 1024;
                }
            }
        }
    }

    #[cfg(windows)]
    {
        // `GlobalMemoryStatusEx` sin dependencias: la estructura y la llamada.
        #[repr(C)]
        struct MemoryStatusEx {
            length: u32,
            memory_load: u32,
            total_phys: u64,
            avail_phys: u64,
            total_page_file: u64,
            avail_page_file: u64,
            total_virtual: u64,
            avail_virtual: u64,
            avail_extended_virtual: u64,
        }
        unsafe extern "system" {
            fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
        }
        let mut estado = MemoryStatusEx {
            length: std::mem::size_of::<MemoryStatusEx>() as u32,
            memory_load: 0,
            total_phys: 0,
            avail_phys: 0,
            total_page_file: 0,
            avail_page_file: 0,
            total_virtual: 0,
            avail_virtual: 0,
            avail_extended_virtual: 0,
        };
        // Seguridad: la estructura es local, está bien dimensionada y su campo
        // `length` declara su tamaño, que es lo que la API exige.
        if unsafe { GlobalMemoryStatusEx(&mut estado) } != 0 {
            return estado.total_phys / (1024 * 1024);
        }
    }

    #[cfg(target_os = "macos")]
    {
        // `sysctl hw.memsize`, por el binario, para no depender de libc.
        if let Ok(salida) = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            && let Ok(texto) = String::from_utf8(salida.stdout)
            && let Ok(bytes) = texto.trim().parse::<u64>()
        {
            return bytes / (1024 * 1024);
        }
    }

    // Suposición prudente: 8 GB.
    8 * 1024
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_portatil_de_dos_nucleos_es_gama_baja() {
        let t = Tuning::derive(2, 8 * 1024);
        assert_eq!(t.tier, Tier::Low);
        assert_eq!(
            t.search_threads, 1,
            "con dos núcleos, uno tiene que quedar libre para dibujar"
        );
        assert_eq!(t.chunk_size, 16_384);
        assert_eq!(t.initial_limit, 500);
    }

    #[test]
    fn poca_memoria_baja_de_gama_aunque_sobren_nucleos() {
        // Un equipo con dieciséis núcleos y 4 GB no es de gama alta: el cuello
        // de botella es la memoria, y el escaneo inicial es lo que la consume.
        let t = Tuning::derive(16, 4 * 1024);
        assert_eq!(t.tier, Tier::Low);
    }

    #[test]
    fn una_maquina_grande_usa_todo_menos_un_nucleo() {
        let t = Tuning::derive(16, 32 * 1024);
        assert_eq!(t.tier, Tier::High);
        assert_eq!(t.search_threads, 15);
        assert_eq!(t.chunk_size, 65_536);
    }

    #[test]
    fn con_un_solo_nucleo_no_se_reserva_ninguno() {
        // Reservar el único núcleo dejaría la búsqueda sin dónde ejecutarse.
        let t = Tuning::derive(1, 2 * 1024);
        assert_eq!(t.search_threads, 1);
    }

    #[test]
    fn el_lote_de_construccion_cabe_en_la_memoria_del_equipo() {
        // La regla: como mucho una cuarta parte de la RAM para el constructor.
        for gb in [4u64, 8, 16, 64] {
            let t = Tuning::derive(8, gb * 1024);
            let pico_bytes = t.build_batch as u64 * 65;
            let ram_bytes = gb * 1024 * 1024 * 1024;
            assert!(
                pico_bytes <= ram_bytes / 4 || t.build_batch == 250_000,
                "con {gb} GB el lote de {} entradas pide {} MB, más de la cuarta parte",
                t.build_batch,
                pico_bytes / (1024 * 1024)
            );
        }
    }

    #[test]
    fn un_equipo_de_cuatro_gigas_no_intenta_construir_diez_millones_de_golpe() {
        // Es el caso que tumbaba el servicio: 10 M de entradas son unos 650 MB
        // de pico, y con 4 GB el sistema empieza a paginar durante el escaneo.
        let t = Tuning::derive(4, 4 * 1024);
        assert!(
            t.build_batch < 10_000_000,
            "con 4 GB el lote no puede ser de diez millones (era {})",
            t.build_batch
        );
    }

    #[test]
    fn test_perfiles_tier_y_max_cached() {
        let low = Tuning::derive_for_tier(Tier::Low);
        assert_eq!(low.tier, Tier::Low);
        assert_eq!(low.max_cached_refine, 500_000);
        assert_eq!(low.chunk_size, 16_384);

        let mid = Tuning::derive_for_tier(Tier::Mid);
        assert_eq!(mid.tier, Tier::Mid);
        assert_eq!(mid.max_cached_refine, 1_500_000);
        assert_eq!(mid.chunk_size, 32_768);

        let high = Tuning::derive_for_tier(Tier::High);
        assert_eq!(high.tier, Tier::High);
        assert_eq!(high.max_cached_refine, 3_000_000);
        assert_eq!(high.chunk_size, 65_536);
    }

    #[test]
    fn los_ajustes_de_esta_maquina_son_coherentes() {
        let t = Tuning::current();
        assert!(t.cores >= 1);
        assert!(t.search_threads >= 1);
        assert!(t.search_threads <= t.cores);
        assert!(t.chunk_size >= 1024);
        assert!(t.initial_limit >= 100);
    }
}
