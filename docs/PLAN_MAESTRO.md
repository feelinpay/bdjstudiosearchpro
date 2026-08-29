# BDJ Studio Search Pro — Plan Maestro de Arquitectura y Ejecución

**Versión del documento:** 1.0 · 29 de agosto de 2026
**Autor:** Arquitectura BDJ Studio
**Estado:** Aprobado para implementación, con 3 decisiones abiertas (§14)

---

## 1. Resumen ejecutivo

Search Pro es un buscador de archivos instantáneo para Windows y macOS. El objetivo de diseño es agresivo y medible: **1 a 10 millones de archivos indexados, con la primera fila de resultados pintada en menos de 30 ms desde la pulsación de tecla**, sin botón Buscar, sin tocar disco durante la búsqueda y sin consumo perceptible de CPU en reposo.

### 1.1 Decisiones tomadas

| Decisión | Elección | Razón |
|---|---|---|
| Motor nativo | **Rust** (edición 2024) | Seguridad de memoria sin GC, paralelismo sin *data races*, SIMD portable, y `unsafe` acotado y auditable en los tres puntos que lo requieren (mmap, syscalls Win32, syscalls Darwin). |
| Interfaz | **Flutter** (Windows + macOS) | Una sola UI para ambas plataformas, coherente con el resto de la suite BDJ Studio. |
| Puente | **flutter_rust_bridge 2.13.0** | Generación automática de bindings, llamadas síncronas sub-milisegundo y arrays sin copia. |
| Plataformas v1 | **Windows y macOS en paralelo** | Ambos backends desde el día uno, sobre una capa de abstracción común. |
| Privilegios Windows | **Servicio elevado + cliente sin privilegios** | La MFT y el USN Journal exigen `SeBackupPrivilege`. Es la única vía a una indexación de un millón de archivos en segundos. |
| Índice | **Columnar, mapeado en memoria (`mmap`)** | Arranque instantáneo sin importar el tamaño, RAM reclamable por el sistema operativo, cero *parsing* al abrir. |
| Color | **Blanco** (§9) | Requisito de producto: limpieza. |
| Licencia | **SPP3 vía `bdj_license_core`** | Idéntico a Sample Pad, mismo ID de dispositivo en la misma máquina. |

### 1.2 Lo que copiamos de Everything, y lo que no

Everything es C puro sobre Win32 con un ejecutable de menos de 2 MB. Nosotros **replicamos su núcleo técnico** — lectura de la MFT, escucha del USN Journal, índice residente en RAM, búsqueda que jamás toca el disco — pero **no replicamos su elección de interfaz**. Flutter cuesta entre 25 y 40 MB de binario y unos 60 MB de RAM base. Es el precio explícito y aceptado de tener Windows y macOS con una sola interfaz y con la identidad visual de la suite.

Ese coste es constante, no crece con el número de archivos, y **no está en la ruta caliente de la búsqueda**: la UI de Flutter nunca contiene la lista de resultados. Ver §8.2, que es la decisión de diseño que hace posible cumplir el presupuesto de 30 ms.

Lo que **no** haremos en la v1, y es deliberado: indexación de contenido, análisis de forma de onda, IA, miniaturas, extracción de metadatos pesada, escaneo periódico completo, base de datos relacional, servicios en la nube.

---

## 2. Presupuestos de rendimiento

Son criterios de aceptación, no aspiraciones. Cada uno tiene su prueba automatizada (§12).

| Métrica | Objetivo | Cómo se mide |
|---|---|---|
| Arranque en frío hasta ventana usable | < 400 ms | Marca de tiempo en `main()` hasta el primer *frame* |
| Apertura del índice de 10 M entradas | < 100 ms | `mmap` + validación de cabecera |
| Tecla → primera fila pintada (10 M) | **< 30 ms p95** | Traza de Flutter + contador en Rust |
| Tecla → conteo total exacto (10 M) | < 120 ms p95 | Igual |
| Consulta de refinamiento (añadir carácter) | < 8 ms p95 | Banco de pruebas en Rust |
| Indexación inicial NTFS, 1 M archivos | < 20 s | Cronómetro del servicio |
| Indexación inicial APFS, 1 M archivos | < 30 s | Igual |
| Cambio en disco → visible en la UI | < 500 ms | Prueba de integración |
| RAM del servicio Windows en reposo | < 40 MB | *Working set* privado |
| RAM del cliente, 1 M indexados | < 120 MB | *Working set* privado (sin contar caché de páginas) |
| CPU en reposo, sin actividad de disco | < 0,1 % | Monitor de 10 minutos |
| Tamaño del índice en disco, 1 M entradas | < 60 MB | `stat` del archivo |

**Nota de honestidad sobre la RAM a 10 millones.** El índice completo de 10 M archivos ocupa 571 MB (desglose en §5.6). Al estar mapeado en memoria, esos 560 MB viven en la caché de páginas del sistema operativo, que Windows y macOS reclaman bajo presión de memoria; no son memoria privada del proceso. En una máquina de 8 GB esto es sostenible, pero el usuario debe poder **excluir volúmenes** (§7.4) para no pagarlo si no lo necesita. Un caso típico de DJ — 200 000 a 800 000 archivos — se queda entre 12 y 45 MB.

---

## 3. Arquitectura general

```
┌──────────────────────────── Cliente (sin privilegios) ────────────────────────────┐
│                                                                                   │
│   Flutter UI (blanca)                                                             │
│        │  ventana de filas visibles, nunca la lista completa                      │
│   ─────┼───────────────────────────────────────────────────────────────           │
│   flutter_rust_bridge  ← llamadas síncronas < 1 ms para pintar filas              │
│   ─────┼───────────────────────────────────────────────────────────────           │
│        ▼                                                                          │
│   bdj_search_core  (Rust)                                                         │
│        ├── query::parser      analiza lo que el usuario escribe                   │
│        ├── search::engine     ejecuta, cancela, refina incrementalmente           │
│        ├── index::view        lectura del índice mapeado en memoria               │
│        └── sort               permutaciones precalculadas + radix bajo demanda    │
│                    │                                                              │
└────────────────────┼──────────────────────────────────────────────────────────────┘
                     │ mmap SOLO LECTURA           ▲ tubería nombrada: control y avisos
                     ▼                             │ (jamás en la ruta caliente)
        ┌────────────────────────────┐             │
        │  índice.bdjx  (en disco)   │             │
        │  = el mismo formato que    │             │
        │    tiene en memoria        │             │
        └────────────────────────────┘             │
                     ▲                             │
                     │ escritura                   │
┌────────────────────┼─────────────────────────────┼────────────────────────────────┐
│                    │                             │                                │
│   bdj_search_indexer  (servicio Windows LocalSystem / demonio macOS de usuario)   │
│        ├── index::builder     construye y compacta                                │
│        ├── fs::traits         VolumeScanner + ChangeWatcher                       │
│        │                                                                          │
│        ├── fs::windows::usn        NTFS/ReFS: FSCTL_ENUM_USN_DATA + READ_USN      │
│        ├── fs::windows::walk       exFAT/FAT32/red: recorrido + ReadDirChangesW   │
│        └── fs::macos               getattrlistbulk + FSEvents                     │
└───────────────────────────────────────────────────────────────────────────────────┘
```

### 3.1 La decisión que lo sostiene todo

**El servicio indexa. El cliente busca.** El índice se escribe en un archivo cuyo contenido byte a byte es idéntico a su representación en memoria; el cliente lo mapea en solo lectura y busca directamente sobre ese mapeo. La tubería nombrada transporta únicamente órdenes de control («empieza a indexar», «excluye este volumen») y avisos («el índice cambió, versión 4711»).

La consecuencia: **la latencia de búsqueda no depende de la IPC en absoluto**. No hay serialización, no hay copias, no hay cambio de contexto entre procesos por cada tecla. Es la diferencia entre 30 ms y 300 ms.

---

## 4. Estructura del repositorio

```
BDJ_Studio_Search_Pro/
├─ .github/workflows/
│   ├─ macos-build.yml           calcado de Sample Pad + toolchain Rust
│   └─ windows-build.yml         nuevo: build + servicio + Inno Setup
├─ docs/
│   ├─ PLAN_MAESTRO.md           este documento
│   ├─ SINTAXIS_BUSQUEDA.md      referencia para el usuario final
│   └─ FORMATO_INDICE.md         especificación binaria versionada
├─ engine/                       workspace de Cargo
│   ├─ Cargo.toml                [workspace]
│   ├─ bdj_search_core/          índice, parser, motor de búsqueda  (sin E/S de disco)
│   │   ├─ src/
│   │   │   ├─ index/            layout, view, builder, arena, formato
│   │   │   ├─ query/            lexer, parser, ast, evaluador
│   │   │   ├─ search/           engine, matcher, refinamiento, cancelación
│   │   │   ├─ sort/             permutaciones, radix
│   │   │   └─ lib.rs
│   │   ├─ benches/              criterion: matcher, parser, ordenamiento
│   │   └─ tests/                corpus sintético de 10 M
│   ├─ bdj_search_fs/            backends de sistema de archivos
│   │   ├─ src/traits.rs         VolumeScanner, ChangeWatcher, VolumeInfo
│   │   ├─ src/windows/          usn.rs, walk.rs, volumes.rs
│   │   └─ src/macos/            bulk.rs, fsevents.rs, volumes.rs
│   ├─ bdj_search_ipc/           protocolo de control (postcard), cliente y servidor
│   ├─ bdj_search_indexer/       binario del servicio/demonio
│   └─ bdj_search_ffi/           capa expuesta a Dart (flutter_rust_bridge)
├─ frontend/                     aplicación Flutter
│   ├─ lib/
│   │   ├─ core/
│   │   │   ├─ ffi/              bindings generados + envoltorio idiomático
│   │   │   ├─ licensing/        LicenseManager, LicensingPort  (copiado)
│   │   │   ├─ security/         DeviceFingerprint, SecureStorageImpl  (copiado)
│   │   │   ├─ theme/            AppColors blanco, AppTheme
│   │   │   ├─ errors/           failures.dart
│   │   │   └─ providers/
│   │   ├─ features/
│   │   │   ├─ licensing/        pantalla de activación blanca
│   │   │   ├─ search/           barra, lista virtual, columnas, filtros
│   │   │   ├─ index_config/     selección de volúmenes, drag & drop de carpetas
│   │   │   ├─ actions/          menú contextual, abrir, copiar ruta, renombrar
│   │   │   ├─ hotkey/           atajo global y ventana emergente
│   │   │   └─ settings/
│   │   └─ main.dart
│   ├─ windows/  macos/
│   └─ pubspec.yaml
├─ distribution/
│   ├─ installer.iss             Inno Setup con instalación del servicio
│   └─ setup_assets/
└─ tools/
    ├─ verify_contracts.py       contratos FFI, enums, huérfanos  (de Stems Music)
    ├─ gen_corpus.rs             generador de corpus sintético
    └─ bench_report.py           informe de presupuestos vs. medido
```

**Regla de dependencias:** `bdj_search_core` no conoce el sistema de archivos ni el sistema operativo. Recibe datos y devuelve resultados. Es puro, determinista y comprobable sin tocar disco. Esta separación es lo que permite ejecutar el banco de pruebas de 10 millones de entradas en CI, en cualquier runner, en segundos.

---

## 5. El índice

### 5.1 Modelo de datos

Un archivo o carpeta es una **entrada**. Las entradas se guardan en columnas paralelas (*struct of arrays*), no en registros (*array of structs*). Es la diferencia entre leer 240 MB de nombres contiguos y saltar por 560 MB de memoria entrelazada: en la práctica, un factor de 3 a 5 en la velocidad del recorrido.

```rust
// Vista sobre el mapeo; ninguna de estas columnas se copia jamás.
pub struct IndexView<'a> {
    header:    &'a Header,
    parent:    &'a [u32],   // índice de la entrada del directorio padre; u32::MAX = raíz del volumen
    name_off:  &'a [u32],   // desplazamiento dentro de name_arena
    name_len:  &'a [u8],    // 1..=255  (límite tanto de NTFS como de APFS)
    flags:     &'a [u8],    // bit0 directorio, bit1 oculto, bit2 sistema, bit3 enlace
    ext_id:    &'a [u16],   // extensión interna; 0 = sin extensión
    volume:    &'a [u8],    // índice en la tabla de volúmenes
    size:      &'a [u64],
    mtime:     &'a [u32],   // segundos Unix; válido hasta 2106
    ctime:     &'a [u32],
    name_arena: &'a [u8],   // todos los nombres, UTF-8, concatenados sin separador
    name_order: &'a [u32],  // permutación precalculada: orden alfabético
    ext_table:  &'a [u8],   // extensiones internadas, únicas
    alive:      &'a [u64],  // mapa de bits: 1 = entrada viva, 0 = lápida
}
```

**Por qué el nombre se guarda una sola vez.** La ruta completa de un archivo no se almacena. Se reconstruye subiendo por `parent` hasta la raíz del volumen y concatenando. Un archivo a 8 niveles de profundidad se resuelve en 8 accesos a memoria — microsegundos. Guardar rutas completas costaría del orden de 800 MB a 10 millones de entradas, casi el triple del índice entero. Esto es exactamente lo que hace Everything, y es la razón por la que su índice es tan compacto.

**Por qué `ext_id` está internado.** `*.wav` no requiere mirar ni un solo carácter del nombre: es una comparación de un `u16`. Los filtros por tipo (Audio, Vídeo, Documentos) se convierten en una pertenencia a conjunto de enteros. Para el flujo de trabajo de un DJ, que filtra por extensión constantemente, esto convierte la operación más frecuente en la más barata.

### 5.2 Formato en disco

El archivo `.bdjx` **es** la imagen de memoria. No hay deserialización.

```
Offset  Tamaño  Campo
0       8       Magia: "BDJXIDX\0"
8       4       Versión de formato (entero; se rechaza si no coincide)
12      4       Suma de comprobación de la cabecera (xxh3)
16      8       entry_count
24      8       arena_len
32      8       Marca de tiempo de construcción
40      8       Generación (se incrementa en cada compactación)
48      16      UUID de la instalación (evita usar el índice de otra máquina)
64      N×8     Tabla de secciones: (offset, longitud) por columna
...             Columnas, cada una alineada a 64 bytes (línea de caché)
```

Al abrir: `mmap` en solo lectura → validar magia, versión, UUID y suma de la cabecera → construir los `&[T]` con `bytemuck` (cero copias, alineación verificada). Coste: microsegundos, sin importar si el archivo pesa 60 MB o 600 MB.

**Compatibilidad:** un cambio de formato incrementa la versión y el índice se reconstruye. Nunca se intenta migrar un índice antiguo; reconstruirlo cuesta segundos y migrar cuesta errores.

### 5.3 Cambios incrementales: base + superposición

El mapeo es de solo lectura y no se puede modificar en caliente. La solución es un modelo de dos capas:

- **Base** — el archivo `.bdjx` mapeado. Inmutable.
- **Superposición** — un segmento mutable en el montón del proceso indexador, con las mismas columnas, que acumula las entradas creadas o modificadas desde la última compactación.
- **Lápidas** — el mapa de bits `alive`. Una eliminación o un movimiento apaga el bit de la entrada base; la versión nueva, si la hay, entra en la superposición.

La búsqueda recorre base y superposición, y salta las entradas cuyo bit `alive` está apagado. El coste de la superposición es proporcional a la actividad reciente del disco, que en un uso normal es de decenas o cientos de entradas.

**Compactación.** Se dispara cuando la superposición supera el 5 % de las entradas base, o tras 120 segundos de inactividad de disco, lo que ocurra primero. Escribe un `.bdjx.tmp` nuevo, hace `fsync`, renombra atómicamente sobre el original e incrementa la generación. El cliente recibe el aviso por la tubería, vuelve a mapear y descarta el mapeo anterior. Un cliente que estuviera leyendo el mapeo viejo termina su consulta sin peligro: en macOS el inodo sobrevive al `rename` mientras el mapeo exista, y en Windows el cliente abre el archivo con `FILE_SHARE_DELETE` — sin ese indicador el renombrado del servicio fallaría con violación de uso compartido, así que es obligatorio y va comprobado con una prueba de integración.

### 5.4 Motor de búsqueda

Esta es la pieza donde se gana o se pierde la promesa de «escribe y encuentra».

**Refinamiento incremental — el algoritmo central.** Cuando el usuario escribe `michael`, no se ejecutan siete búsquedas sobre 10 millones de entradas. Se ejecuta una sobre 10 millones y seis sobre conjuntos cada vez menores:

```
"m"        → recorrido completo (10 000 000)  →  480 000 resultados
"mi"       → recorre solo esos 480 000        →   95 000
"mic"      → recorre solo esos  95 000        →   12 000
"mich"     → recorre solo esos  12 000        →    3 100
"micha"    → …                                →    2 900
"michae"   → …                                →    2 850
"michael"  → …                                →    2 840
```

El motor guarda una pila de conjuntos de resultados indexada por el prefijo de la consulta. Añadir un carácter reutiliza la cima de la pila; borrar uno la desapila; pegar un texto entero busca el prefijo común más largo ya calculado. Solo la **primera** pulsación paga el recorrido completo, y ese recorrido está paralelizado y transmite resultados (ver abajo). Todo lo demás cuesta menos de 8 ms.

La condición para que sea correcto: el refinamiento solo es válido si la consulta nueva es **monótonamente más restrictiva** que la anterior. Lo es para la extensión de un término literal. No lo es cuando el carácter añadido cambia la estructura (`rock` → `rock |`, o al cerrar una función `size:>10` → `size:>10m`). El evaluador detecta esos casos comparando los árboles sintácticos y cae a un recorrido completo, que sigue estando dentro de presupuesto.

**Recorrido completo paralelo y transmitido.** El recorrido inicial se reparte entre `rayon` en fragmentos de 64 K entradas. Cada hilo:

1. Comprueba un `AtomicU64` de generación; si la generación cambió, aborta de inmediato. **No hay *debounce*.** Retrasar la búsqueda 100 ms para «esperar a que el usuario acabe de escribir» es exactamente la latencia que estamos intentando eliminar. Cancelamos y relanzamos.
2. Descarta por `ext_id`, tamaño y fecha antes de mirar un solo carácter del nombre. Estos filtros son comparaciones enteras y eliminan la mayoría de candidatos en las consultas típicas de un DJ.
3. Sobre lo que queda, ejecuta el comparador de subcadena sin distinción de mayúsculas.

La UI no espera al recorrido completo. En cuanto se acumulan **200 aciertos** (más de lo que cabe en pantalla), se emiten y se pintan. El conteo total llega después, y la fila de estado muestra «2 840 resultados» cuando el recorrido termina. Percepción del usuario: instantáneo.

**Comparador de subcadena.** Sobre nombres ASCII, que son más del 99 % del corpus real:

- Se precalcula el primer byte del patrón en sus dos cajas (`m` y `M`).
- `memchr2` de la biblioteca `memchr` localiza candidatos a 10–20 GB/s usando AVX2 o NEON.
- En cada candidato se compara el resto con una tabla de plegado ASCII de 256 entradas.
- Los nombres con bytes ≥ 0x80 se marcan con un bit en `flags` durante la indexación y siguen la ruta lenta: plegado Unicode completo con `unicase`. Es el 1 % de los casos y no afecta al presupuesto.

Con este esquema, el recorrido completo de una arena de 240 MB en un portátil moderno cuesta entre 20 y 45 ms repartidos entre los núcleos, y las primeras 200 filas salen mucho antes de terminarlo.

**Expresiones regulares.** Cuando la consulta lleva `regex:`, el motor usa la caja `regex` con `RegexBuilder::size_limit` acotado para que un patrón patológico no consuma memoria sin límite. El refinamiento incremental se desactiva. Es una función avanzada, oculta tras «Búsqueda avanzada», y no está sujeta al presupuesto de 30 ms.

### 5.5 Ordenamiento sin volver a consultar el disco

- **Por nombre y por extensión:** se recorre la permutación `name_order`, precalculada al construir el índice, y se emiten las entradas que pasaron el filtro. Es O(n) en el número de entradas vivas, no O(n log n), y sin comparaciones de cadenas en tiempo de consulta.
- **Por tamaño y por fecha:** `radix sort` de claves `u64`/`u32` sobre el vector de identificadores resultante. Un millón de resultados se ordena en 10–20 ms.
- **Por ruta:** se ordena por `(volume, name_order del padre, name_order propio)`, sin materializar rutas.

El orden inverso es la lectura de la misma secuencia al revés. Cambiar de columna nunca vuelve a buscar; reordena el conjunto ya calculado.

### 5.6 Presupuesto de memoria, cifras honestas

| Columna | Bytes/entrada | 1 M | 10 M |
|---|---:|---:|---:|
| `parent` | 4 | 4 MB | 40 MB |
| `name_off` | 4 | 4 MB | 40 MB |
| `name_len` | 1 | 1 MB | 10 MB |
| `flags` | 1 | 1 MB | 10 MB |
| `ext_id` | 2 | 2 MB | 20 MB |
| `volume` | 1 | 1 MB | 10 MB |
| `size` | 8 | 8 MB | 80 MB |
| `mtime` | 4 | 4 MB | 40 MB |
| `ctime` | 4 | 4 MB | 40 MB |
| `alive` (bitmap) | 0,125 | 0,1 MB | 1,2 MB |
| **Subtotal columnas** | **29,1** | **29 MB** | **291 MB** |
| `name_arena` (24 B de media) | 24 | 24 MB | 240 MB |
| `name_order` | 4 | 4 MB | 40 MB |
| **Total** | **57** | **57 MB** | **571 MB** |

Todo ello mapeado en memoria: no es memoria privada del proceso, es caché de páginas reclamable. En una máquina de 8 GB con 10 millones de archivos indexados, el sistema mantiene en RAM las páginas realmente tocadas — que en un uso normal son las columnas de filtro y la parte de la arena que las consultas recorren.

Si en las mediciones reales la arena supera los 24 bytes de media, la palanca es guardar los nombres de más de 64 bytes en una arena secundaria, dejando la principal densa. Es una optimización de reserva; no la implementamos hasta tener el dato.

---

## 6. Lenguaje de consulta

### 6.1 Gramática

```
consulta   := o_expr
o_expr     := y_expr ( '|' y_expr )*
y_expr     := unario ( ESPACIO unario )*          ← el espacio es Y lógico
unario     := '!' unario | primario
primario   := '(' o_expr ')' | funcion | termino
funcion    := identificador ':' valor
termino    := palabra | '"' texto entrecomillado '"'
```

El espacio como Y lógico y `|` como O lógico coinciden con Everything, que es la referencia que conocen los usuarios que vienen de ahí.

### 6.2 Funciones, con alias en español

| Función | Alias | Ejemplo | Significado |
|---|---|---|---|
| `ext:` | `ext:` | `ext:wav\|flac` | Extensión, lista separada por `\|` |
| `size:` | `tam:` | `tam:>100mb`, `size:1gb..2gb` | Tamaño, con `<` `>` `..` y sufijos `kb mb gb tb` |
| `dm:` | `mod:` | `mod:hoy`, `dm:>2026-01-01` | Fecha de modificación |
| `dc:` | `creado:` | `creado:estasemana` | Fecha de creación |
| `path:` | `ruta:` | `ruta:"Music\Latin"` | Coincidencia en la ruta completa |
| `parent:` | `carpeta:` | `carpeta:Samples` | Nombre del directorio contenedor |
| `type:` | `tipo:` | `tipo:audio` | Grupo predefinido (§6.4) |
| `regex:` | `regex:` | `regex:^DJ.*\.wav$` | Expresión regular |
| `case:` | `may:` | `may:Remix` | Fuerza distinción de mayúsculas |
| `file:` / `folder:` | `archivo:` / `carpeta_solo:` | | Restringe a archivos o a carpetas |

Valores de fecha admitidos: `hoy/today`, `ayer/yesterday`, `estasemana/thisweek`, `estemes/thismonth`, `esteaño/thisyear`, `2026-08`, `2026-08-29`, y los comparadores `<` `>` `..`.

### 6.3 Consultas de ejemplo, tomadas del flujo de trabajo real

```
michael jackson                      ambos términos, en cualquier orden
ext:wav|flac remix                   audio sin comprimir cuyo nombre contiene "remix"
D:\Music ext:mp3                     una ruta absoluta actúa como filtro de ruta
ruta:Latin tipo:audio tam:>50mb      pistas grandes en la carpeta Latin
!remix ext:wav                       WAV que NO son remixes
mod:hoy tipo:audio                   lo que importé hoy
carpeta:Samples ext:wav tam:<5mb     one-shots del directorio Samples
regex:^\d{3}_.*\.wav$                nomenclatura estricta de exportación
```

### 6.4 Filtros rápidos (botones)

Cada botón es una consulta `type:` predefinida, editable por el usuario en Ajustes:

| Botón | Extensiones |
|---|---|
| Todos | — |
| Audio | wav flac aiff aif mp3 m4a ogg opus wma alac ape wv aac |
| Vídeo | mp4 mov avi mkv m4v webm mpg mpeg wmv flv |
| Imagen | jpg jpeg png gif webp heic tiff bmp svg psd |
| Documentos | pdf docx doc txt rtf odt xlsx pptx md |
| Proyectos DJ | als flp ptx cpr logicx rpp nki nkm cue m3u m3u8 xml |
| Comprimidos | zip rar 7z tar gz bz2 xz |
| Aplicaciones | exe msi dll app dmg pkg |
| Carpetas | (solo directorios) |

«Proyectos DJ» es la categoría que Everything no tiene y que aquí sí importa: Ableton, FL Studio, Pro Tools, Cubase, Logic, Reaper, Kontakt y las listas de reproducción y los `.xml` de Rekordbox.

---

## 7. Backends de sistema de archivos

### 7.1 Abstracción común

```rust
pub trait VolumeScanner: Send {
    /// Enumeración completa. Emite lotes para que el constructor
    /// del índice trabaje en paralelo con la lectura.
    fn scan(&mut self, sink: &mut dyn FnMut(&[RawEntry])) -> Result<ScanReport>;
}

pub trait ChangeWatcher: Send {
    /// Bloquea hasta que hay cambios o hasta agotar el tiempo de espera.
    fn poll(&mut self, timeout: Duration) -> Result<Vec<FsChange>>;
    /// Testigo de reanudación: USN en Windows, FSEventStreamEventId en macOS.
    fn cursor(&self) -> Cursor;
    fn resume_from(&mut self, cursor: Cursor) -> Result<()>;
}

pub enum FsChange {
    Created(RawEntry),
    Deleted { file_id: FileId },
    Renamed { file_id: FileId, new_parent: FileId, new_name: String },
    Modified { file_id: FileId, size: u64, mtime: u32 },
    /// El sistema perdió eventos: hay que reexplorar este subárbol y solo ese.
    RescanSubtree { path: PathBuf },
}
```

Cuatro implementaciones detrás de este par de *traits*. El constructor del índice y el motor de búsqueda no saben cuál está activa.

### 7.2 Windows, volúmenes NTFS y ReFS — la ruta rápida

**Enumeración inicial.** Se abre el volumen con `CreateFileW(r"\\.\C:", GENERIC_READ, FILE_SHARE_READ|WRITE, …)` — que exige elevación — y se llama repetidamente a `DeviceIoControl` con **`FSCTL_ENUM_USN_DATA`**. Cada llamada devuelve un lote de `USN_RECORD_V2/V3` con exactamente los cuatro campos que necesitamos: `FileReferenceNumber`, `ParentFileReferenceNumber`, `FileName` y `FileAttributes`.

Esto es importante y conviene subrayarlo: **no hace falta escribir un analizador de la MFT en crudo.** `FSCTL_ENUM_USN_DATA` es la vía documentada por Microsoft, entrega la misma información, y evita todo el trabajo frágil de interpretar registros de la MFT, atributos residentes y no residentes, y listas de atributos. Un millón de entradas se enumera en unos pocos segundos con un búfer de 64 KB.

Lo que `FSCTL_ENUM_USN_DATA` **no** entrega es el tamaño ni las fechas. Se obtienen abriendo cada archivo por su identificador con `OpenFileById` + `GetFileInformationByHandleEx`, que es caro a escala de millones. Estrategia en dos tiempos:

1. **Fase 1, inmediata:** nombres, rutas y extensiones. Con esto la búsqueda por nombre, por extensión y por ruta ya funciona. Es el 90 % de lo que un DJ hace.
2. **Fase 2, en segundo plano y a baja prioridad:** tamaños y fechas, mediante un recorrido de directorios con `FindFirstFileExW` en modo `FindExInfoBasic` con `FIND_FIRST_EX_LARGE_FETCH`, que devuelve tamaño y fechas de todo un directorio por llamada. Mientras esta fase no acaba, los filtros `size:` y `dm:` aparecen atenuados en la interfaz con la indicación de que se están completando.

La distinción es lo que permite que la búsqueda esté viva a los pocos segundos de instalar, en lugar de a los varios minutos.

**Actualización incremental.** `FSCTL_QUERY_USN_JOURNAL` para obtener el `UsnJournalID` y el `NextUsn`, luego `FSCTL_READ_USN_JOURNAL` en un bucle con espera. Se persiste el par `(UsnJournalID, NextUsn)` en cada compactación. Al arrancar:

- Mismo `UsnJournalID` y el `NextUsn` guardado sigue dentro del diario → se reanuda leyendo solo lo ocurrido desde el apagado. **Este es el mecanismo que hace que Search Pro esté al día un segundo después de arrancar, sin reescanear nada.**
- El diario se borró, se recreó o el `NextUsn` quedó fuera de rango (el diario es circular y se sobrescribe) → reenumeración completa de ese volumen.

Consumo en reposo: el hilo espera bloqueado en `DeviceIoControl`. Cero CPU hasta que hay actividad de disco.

**Cajas de referencia.** `windows` (bindings oficiales de Microsoft) para todas las llamadas. `usn-journal-rs` (MIT) sirve como referencia de implementación y prueba cruzada; la implementación propia se justifica por el control sobre el manejo de búferes y la asignación de memoria en el camino caliente, que es donde se gana la velocidad de enumeración.

### 7.3 Windows, exFAT / FAT32 / unidades de red — la ruta imprescindible

**No es un caso marginal: es el caso central de este producto.** Las memorias USB y los discos externos que un DJ lleva a la cabina están formateados en exFAT o FAT32, porque es lo que leen los reproductores Pioneer y lo que funciona en Windows y macOS a la vez. Esos volúmenes **no tienen MFT ni USN Journal**. Un buscador para DJs que solo funcione en NTFS no sirve.

- **Enumeración:** recorrido de directorios con `FindFirstFileExW` en `FindExInfoBasic` + `FIND_FIRST_EX_LARGE_FETCH`, paralelizado por subárbol con `rayon`. Devuelve nombre, tamaño y fechas de una sola pasada, así que aquí no hay fase 2. Es más lento que el USN — del orden de 60 000 a 150 000 entradas por segundo en SSD — pero los volúmenes extraíbles son órdenes de magnitud más pequeños que un disco de sistema.
- **Vigilancia:** `ReadDirectoryChangesW` con `FILE_NOTIFY_CHANGE_FILE_NAME | DIR_NAME | SIZE | LAST_WRITE` sobre la raíz del volumen y `bWatchSubtree = TRUE`, en E/S superpuesta con un puerto de terminación. Cuando el búfer se desborda, el sistema avisa y se reexplora **solo** el subárbol afectado, nunca el volumen entero.
- **Montaje y desmontaje:** se escucha `WM_DEVICECHANGE` (`DBT_DEVICEARRIVAL` / `DBT_DEVICEREMOVECOMPLETE`). Al conectar una unidad ya conocida se valida su firma; si no cambió, el índice guardado se reutiliza al instante en vez de reescanear. Al desconectarla, sus entradas se atenúan en los resultados en lugar de desaparecer, con la etiqueta del volumen visible — así el usuario ve que el archivo está «en el USB negro», que es información útil, no ruido.

Esta última decisión es la que Everything no toma y que aquí aporta valor real.

### 7.4 macOS

APFS y HFS+ no ofrecen ningún equivalente público a la MFT ni al USN Journal. La estrategia es distinta y hay que decirlo con claridad: **en macOS la indexación inicial es intrínsecamente más lenta que en Windows con NTFS.** El objetivo sigue siendo alcanzable, pero por otra vía.

- **Enumeración:** `getattrlistbulk(2)`, que devuelve decenas o cientos de entradas de directorio **con sus metadatos** en una sola llamada al sistema, sobre un búfer de 128 KB. Frente a `readdir` + `lstat` por archivo, elimina cientos de miles de llamadas al sistema. Una referencia pública mide 409 500 archivos en 521 ms con caché caliente, unas 6 veces más rápido que `du`. Con caché fría y paralelizando por subárbol, el presupuesto de 30 s por millón de archivos tiene margen holgado.
- **Vigilancia:** `FSEvents` con `kFSEventStreamCreateFlagFileEvents` (eventos por archivo, no por directorio), `kFSEventStreamCreateFlagWatchRoot` y `kFSEventStreamCreateFlagNoDefer`. Se persiste el último `FSEventStreamEventId`; al arrancar se reanuda desde él, y macOS entrega los eventos ocurridos mientras la aplicación estaba cerrada, gracias al almacén de eventos que el sistema mantiene en disco. Es lo más parecido a un diario que ofrece la plataforma y cumple la misma función que el USN.
- **Pérdida de eventos:** cuando llega `kFSEventStreamEventFlagMustScanSubDirs`, o cuando el identificador persistido ya no está en el almacén, se reexplora exclusivamente el subárbol implicado.
- **Sin ayudante privilegiado.** En macOS no hay un equivalente a leer la MFT, así que no hay nada que ganar con un `SMJobBless`. El demonio corre como agente de usuario (`LaunchAgent`) y el acceso se resuelve por permisos, no por privilegios.
- **Acceso Total al Disco (TCC).** Sin él, macOS oculta el Escritorio, Documentos, Descargas, iCloud Drive y los volúmenes externos. **Es el mayor obstáculo de experiencia de usuario del producto en macOS.** Al primer arranque se muestra una pantalla que explica por qué hace falta y abre directamente el panel correspondiente con `x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles`. Mientras no se conceda, la aplicación funciona sobre lo accesible y muestra un aviso permanente y discreto. No se bloquea el producto por esto.
- **Volúmenes de red y externos:** se detectan con `getfsstat`; los montajes de red se excluyen por defecto y el usuario los añade a mano si quiere.

### 7.5 Selección de volúmenes

El usuario elige exactamente qué se indexa, y esa elección es la palanca principal sobre el consumo de recursos:

```
Índice
  ☑ Disco local (C:)              1 240 000 archivos     71 MB
  ☑ SSD Música (D:)                 380 000 archivos     22 MB
  ☑ USB Cabina (E:)  exFAT           41 000 archivos      2 MB
  ☐ Backup (F:)                    excluido
  ☑ Carpeta concreta:  D:\Samples\Kits
```

Y el requisito 15 del enunciado: arrastrar una carpeta a la ventana la añade al índice. La zona de soltar acepta carpetas y también archivos (en cuyo caso añade la carpeta que los contiene).

---

## 8. Capa FFI y capa Flutter

### 8.1 Contrato FFI

```rust
// bdj_search_ffi — la superficie completa expuesta a Dart.
pub fn engine_open(index_dir: String) -> Result<()>;
pub fn engine_close();

/// Lanza una consulta. Cancela la anterior. Devuelve al instante.
/// El identificador de generación permite a la UI ignorar respuestas obsoletas.
pub fn search(query: String, sort: SortSpec) -> u64;

/// Estado de la consulta en curso o terminada.
pub fn search_status(generation: u64) -> SearchStatus;  // { listos, total, completo }

/// Ventana de filas ya formateadas para pintar. Llamada SÍNCRONA, sub-milisegundo.
pub fn rows(generation: u64, offset: u32, count: u32) -> RowBatch;

/// Acciones sobre un resultado.
pub fn full_path(generation: u64, row: u32) -> String;
pub fn reveal_in_explorer(generation: u64, row: u32) -> Result<()>;

/// Configuración; van al servicio por la tubería de control.
pub fn volumes() -> Vec<VolumeStatus>;
pub fn set_volume_indexed(volume_id: u8, indexed: bool) -> Result<()>;
pub fn add_folder(path: String) -> Result<()>;
pub fn indexing_progress() -> IndexingProgress;

/// Flujo de avisos: índice actualizado, indexación terminada, volumen conectado.
pub fn events() -> Stream<EngineEvent>;
```

`RowBatch` es un búfer plano — desplazamientos y bytes — no una lista de estructuras. `flutter_rust_bridge` lo entrega sin copiar. Pintar 100 filas cuesta una asignación de memoria en Dart, no cien.

### 8.2 La regla que hace que la UI cumpla el presupuesto

**Flutter nunca contiene la lista de resultados.**

Rust es el dueño del vector de resultados. Flutter dibuja un `ListView.builder` cuyo `itemCount` es el total informado por `search_status`, y cada fila que entra en pantalla pide sus datos con `rows(gen, offset, count)`. Se piden en bloques de 100 y se guardan en una caché LRU de unas 20 páginas — dos mil filas, unos pocos cientos de kilobytes.

Consecuencias directas:

- Desplazarse por 10 millones de resultados consume memoria constante.
- Cambiar el orden no mueve datos a Dart: se invalida la caché y se vuelven a pedir las filas visibles.
- No existe el escenario «diez millones de objetos Dart», que es como se destruye esta clase de aplicación.

La lista se actualiza mediante un `ValueNotifier<int>` con la generación de la búsqueda. Un cambio de generación invalida la caché y solicita un repintado; no reconstruye el árbol de *widgets* completo.

### 8.3 Interfaz

Distribución, apoyada en las 16 funciones del enunciado:

```
┌──────────────────────────────────────────────────────────────────────┐
│  🔎  michael jackson                                      ⚙   ─ □ ✕  │  campo siempre enfocado
├──────────────────────────────────────────────────────────────────────┤
│ [Todos] [Audio] [Vídeo] [Imagen] [Docs] [Proyectos] [Carpetas] […]  │  filtros rápidos
├────────────────────────┬──────────────────┬────────┬────────┬────────┤
│ Nombre            ▲    │ Ruta             │ Tipo   │ Tamaño │ Modif. │  columnas ordenables
├────────────────────────┼──────────────────┼────────┼────────┼────────┤
│ ♪ Billie Jean.mp3      │ D:\Music\80s     │ MP3    │ 8,2 MB │ 12 ago │
│ ♪ Beat It.mp3          │ D:\Music\80s     │ MP3    │ 7,9 MB │ 12 ago │
│ ♪ Thriller.wav         │ E:\USB\Sets      │ WAV    │  52 MB │ 03 jul │  ← volumen desconectado, atenuado
├──────────────────────────────────────────────────────────────────────┤
│ 2 840 resultados · 1 621 340 indexados · 12 ms                       │  barra de estado
└──────────────────────────────────────────────────────────────────────┘
```

- **Sin botón Buscar.** El campo tiene el foco al abrir y cada carácter dispara la consulta.
- **Menú contextual:** Abrir · Abrir ubicación · Copiar · Copiar ruta completa · Mover a… · Renombrar · Eliminar · Propiedades. Renombrar y eliminar piden confirmación y actualizan el índice por la vía normal de vigilancia, no por un atajo.
- **Atajo global** `Ctrl+Shift+Espacio` en Windows y `⌘+Shift+Espacio` en macOS, mediante `hotkey_manager`. Abre una ventana emergente centrada, sin marco, al estilo de Spotlight, con el foco puesto. `Esc` la oculta sin cerrar el proceso: la aplicación permanece residente en la bandeja del sistema para que la reaparición sea instantánea.
- **Arrastrar y soltar** para añadir carpetas al índice, con `desktop_drop`.
- **Doble clic** abre el archivo con su aplicación asociada; `Ctrl`/`⌘` + doble clic abre su ubicación.

### 8.4 Presupuesto de la UI

El objetivo de 30 ms se reparte así, y cada tramo tiene su medición:

```
pulsación de tecla
  → 1 ms   Flutter procesa el evento y llama a search()
  → 2 ms   Rust analiza la consulta y compara árboles sintácticos
  → 12 ms  refinamiento o recorrido parcial hasta 200 aciertos
  → 1 ms   rows(0, 40) devuelve las filas visibles
  → 10 ms  Flutter compone y pinta el fotograma
  ────────
   26 ms   objetivo p95 · presupuesto 30 ms
```

Si en las mediciones el tramo de pintado de Flutter se dispara, la palanca es reducir el trabajo por fila: texto plano sin `RichText`, iconos precalculados por `ext_id`, y `RepaintBoundary` en cada fila.

---

## 9. Identidad visual: blanco

El requisito es blanco como señal de limpieza. Eso no significa blanco sobre blanco: significa un fondo blanco con jerarquía construida a base de tipografía y espacio, no de cajas y bordes.

```dart
class AppColors {
  AppColors._();

  // Superficies
  static const Color background   = Color(0xFFFFFFFF);  // lienzo
  static const Color surface      = Color(0xFFF7F8FA);  // barras, cabeceras
  static const Color surfaceAlt   = Color(0xFFFCFCFD);  // filas alternas
  static const Color hover        = Color(0xFFF1F3F7);
  static const Color selected     = Color(0xFFEAE7FF);  // selección, violeta muy claro
  static const Color border       = Color(0xFFE6E8EC);
  static const Color borderStrong = Color(0xFFD3D7DE);

  // Texto  (contraste indicado sobre fondo blanco)
  static const Color textPrimary   = Color(0xFF14181F);  // 15,8:1
  static const Color textSecondary = Color(0xFF5B6472);  // 6,1:1
  static const Color textDisabled  = Color(0xFF9AA2AE);  // 2,8:1 — solo texto inactivo

  // Acento — violeta BDJ ajustado para blanco
  static const Color primary      = Color(0xFF5B4BFF);  // 5,9:1 · el 0xFF7C4DFF de la suite
                                                        //   solo da 4,1:1 sobre blanco
  static const Color primaryHover = Color(0xFF4A3AEE);
  static const Color primarySoft  = Color(0xFFEEECFF);

  // Estados
  static const Color success = Color(0xFF0A7C42);  // 5,3:1
  static const Color warning = Color(0xFF8A5A00);  // 5,4:1
  static const Color error   = Color(0xFFC0192B);  // 6,2:1

  // Color por tipo de archivo (iconos y etiqueta de extensión)
  static const Color kindAudio    = Color(0xFF5B4BFF);
  static const Color kindVideo    = Color(0xFFD1355A);
  static const Color kindImage    = Color(0xFF0A7C42);
  static const Color kindDocument = Color(0xFF2563A8);
  static const Color kindArchive  = Color(0xFF8A5A00);
  static const Color kindFolder   = Color(0xFF6B7280);
  static const Color kindApp      = Color(0xFF7A3E9D);
}
```

**Por qué el acento cambia respecto de la suite.** El `0xFF7C4DFF` de Sample Pad se eligió sobre un fondo casi negro. Sobre blanco da 4,1:1, por debajo del mínimo AA de 4,5:1 para texto normal. `0xFF5B4BFF` conserva el mismo carácter violeta y sube a 5,9:1. Es el mismo color de marca, corregido para el nuevo fondo — no un color distinto.

**El cian queda fuera.** `0xFF00E5FF` sobre blanco es 1,5:1: ilegible. En Search Pro no se usa.

Reglas de composición:

- Fondo blanco puro para el área de resultados; nada de rayado alterno agresivo (`surfaceAlt` es prácticamente blanco y solo guía la vista).
- Una única sombra en toda la aplicación, la de la ventana emergente del atajo global. Todo lo demás separa con espacio o con un borde de 1 px.
- La coincidencia buscada se resalta dentro del nombre con `primarySoft` de fondo y peso semibold, nunca con color de texto.
- La densidad de fila por defecto es de 28 px, con una opción compacta de 22 px en Ajustes: son DJs mirando miles de archivos, y las filas de 48 px de Material desperdician la pantalla.
- La aplicación es de tema claro fijo. No hay tema oscuro en la v1; añadirlo después es solo un segundo `AppColors`, porque ningún color está incrustado en los *widgets*.

---

## 10. Licenciamiento

### 10.1 Lo que se reutiliza tal cual

Se copian sin modificar desde Sample Pad, porque son código probado en producción:

- `core/security/device_fingerprint.dart` — HWID V2. **Sin cambios.** El comentario del propio archivo lo garantiza: el algoritmo y las fuentes de hardware son idénticos en todas las apps BDJ Studio, así que **en la misma máquina Search Pro muestra exactamente el mismo ID de dispositivo que Sample Pad y que Stems Music.** En Windows sale de `Win32_ComputerSystemProduct.UUID` + `Win32_Processor.ProcessorId` + `Win32_BaseBoard.SerialNumber` vía PowerShell asíncrono, con `machineGuid` como reserva; en macOS de `systemGUID`.
- `core/security/secure_storage_impl.dart` y `security_port.dart` — almacenamiento en el Credential Manager de Windows y el Keychain de macOS, con su autoprueba obligatoria.
- `core/licensing/licensing_port.dart` — `LicenseStatus`, `LicenseInfo`, `LicensingPort`.
- `core/errors/failures.dart`.
- `features/licensing/presentation/providers/license_providers.dart` — incluido el presupuesto de 12 segundos para la validación de arranque.

### 10.2 Lo que cambia

```dart
class LicenseManager implements LicensingPort {
  LicenseManager({
    required SecurityPort secureStorage,
    required DeviceFingerprint fingerprint,
    this.productCode = 'bdj_studio_search_pro',   // ← el único cambio funcional
    this.defaultAppVersion = '1.0.0',
  }) : ...
}

class LicenseStorageKeys {
  static const String licenseKey           = 'bdj.search_pro.license_key';
  static const String licenseStatus        = 'bdj.search_pro.license_status';
  static const String deviceId             = 'bdj.search_pro.device_id';
  static const String hardwareFingerprint  = 'bdj.search_pro.hardware_fingerprint';
  static const String lastLicenseCheckUtc  = 'bdj.search_pro.last_license_check_utc';
  static const String installId            = 'bdj.search_pro.install_id';
  static const String lastSyncAt           = 'bdj.search_pro.last_sync_at';
  // Sin claves legadas: es un producto nuevo, no hay nada que migrar.
}
```

`Spp3Token.verify` recibe `expectedProductCode: 'bdj_studio_search_pro'`. El resto del protocolo — verificación Ed25519 del certificado de administrador contra la clave raíz del ecosistema, comprobación de vigencia del certificado en el momento de emisión, acoplamiento exacto de versión, comparación del hash de HWID, caducidad y protección contra retroceso del reloj — funciona sin tocar una línea.

**Nota sobre el nombre del protocolo:** el prefijo es `SPP3`, no `SSP3`. Está así en `Spp3Token.protocolPrefix` y en todos los mensajes al usuario de Sample Pad. Un token es `SPP3.<payload>.<certificado>.<firma>` — cuatro segmentos — y el producto se distingue por el campo `pcode` del payload, no por el prefijo. No hay que inventar un prefijo nuevo para Search Pro.

### 10.3 Pantalla de activación, en blanco

Misma estructura que la de Sample Pad, repintada:

- Logo, «BDJ STUDIO SEARCH PRO», y debajo «BÚSQUEDA INSTANTÁNEA DE ARCHIVOS · ACTIVACIÓN DE LICENCIA».
- Bloque **ID DEL DISPOSITIVO**: fondo `surface`, borde `border`, el código en monoespaciada de 18 px `SelectableText` en color `primary`, y debajo el botón **Copiar ID** que pone el HWID en el portapapeles y muestra un aviso de confirmación. Idéntico comportamiento al que ya conoce el cliente.
- Campo de la llave SPP3, de dos líneas, con el ejemplo `SPP3.<payload>.<certificado>.<firma>` como texto de ayuda.
- Botón **ACTIVAR APLICACIÓN** a todo el ancho.
- Errores en un bloque `error` sobre fondo rosado claro.

Flujo de arranque, igual que en Sample Pad: `LicenseNotifier` valida al construirse; mientras tanto se muestra un indicador de progreso; `licensed` entra en la pantalla principal, `unlicensed` y `error` llevan a la activación. Se revalida al volver del segundo plano.

### 10.4 El servicio y la licencia

El servicio de Windows arranca en estado inactivo y **no indexa nada** hasta que el cliente le envía `EnableIndexing` por la tubería tras una validación satisfactoria. Si la licencia se desactiva o caduca, el cliente envía `DisableIndexing`, el servicio detiene la vigilancia y borra el archivo de índice. Así no queda un servicio elevado escaneando el disco de alguien que ya no es cliente.

### 10.5 BDJ Studio License — el emisor

Search Pro no puede activarse hasta que la aplicación emisora sepa que existe. Hoy el catálogo de productos está **repetido en siete lugares** entre el backend NestJS y la aplicación Flutter de administración, y ninguno de ellos se entera si los demás cambian. Añadir el producto son siete ediciones; ese es el trabajo mínimo. Después propongo eliminar la repetición, porque a la sexta aplicación de la suite esto se rompe solo.

#### Los siete puntos a tocar

| # | Archivo | Línea aprox. | Cambio |
|---|---|---:|---|
| 1 | `backend/src/issuer/dto/issuer.dto.ts` | 13 | Añadir `'bdj_studio_search_pro'` al array `products`. **Sin esto el backend rechaza la sincronización con un 400 de validación**, aunque el token ya esté firmado. |
| 2 | `frontend/lib/main.dart` | 1673 | `productCodes`: añadir `'bdj_studio_search_pro': 6` |
| 3 | `frontend/lib/main.dart` | 1751 | `exactVersion`: añadir `'bdj_studio_search_pro' => '1.0.0'` |
| 4 | `frontend/lib/main.dart` | 2201 | `productLabel`: añadir `=> 'BDJ Studio Search Pro'` |
| 5 | `frontend/lib/main.dart` | 2210 | `appVersion`: añadir `=> '1.0.0'` |
| 6 | `frontend/lib/dashboard.dart` | 3 | `_productLabels`: añadir la entrada |
| 7 | `frontend/lib/dashboard.dart` | 1566 | Lista de `DropdownMenuItem` del selector de producto: añadir la opción |

El `productCode: 6` no es cosmético: `main.dart` lanza `ArgumentError('Producto no soportado.')` si el producto no está en ese mapa, así que sin el número 6 la emisión falla antes de firmar nada.

#### Lo que no hay que tocar

- **El ID de dispositivo.** La validación de `main.dart` acepta `^[A-Z0-9]{4}(?:-[A-Z0-9]{4}){3}$`, que es exactamente el formato que produce `HwidEngine._hashToHwid` (`XXXX-XXXX-XXXX-XXXX`). El HWID de Search Pro pasa sin cambios, y como el algoritmo es común a toda la suite, **el ID que pegue el cliente desde Search Pro es el mismo que ya tienes registrado si te compró Sample Pad**. Ese cliente aparece ya vinculado en la pantalla de clientes; no hay que darlo de alta otra vez.
- **La clave raíz ni el certificado de administrador.** `KeyHierarchy.ecosystemRootPublicKey` es del ecosistema, no del producto. La misma clave firma las licencias de Search Pro.
- **El esquema de Prisma.** `OfflineLicense.product` es un `String` libre y el índice `@@index([customerId, product])` funciona igual con un valor más. No hay migración de base de datos.
- **El flujo de emisión, sincronización, revocación y bloqueo de dispositivo.** Search Pro entra como un producto más.

#### La corrección de fondo: un catálogo único

Siete lugares para un mismo hecho es la definición de código redundante, y ya te ha costado una vez la coherencia entre `_productLabels` y los `DropdownMenuItem` escritos a mano. Propuesta:

```dart
// bdj_license_core/lib/src/catalog/products.dart  — nuevo, compartido por TODAS las apps
class BdjProduct {
  const BdjProduct({
    required this.code,        // 'bdj_studio_search_pro'
    required this.numericCode, // 6
    required this.label,       // 'BDJ Studio Search Pro'
    required this.version,     // '1.0.0'
  });

  final String code;
  final int numericCode;
  final String label;
  final String version;

  static const searchPro = BdjProduct(
    code: 'bdj_studio_search_pro',
    numericCode: 6,
    label: 'BDJ Studio Search Pro',
    version: '1.0.0',
  );
  // … samplePad, synthPro, stemsMusic, waveVideo, voiceSpot

  static const all = <BdjProduct>[
    samplePad, synthPro, stemsMusic, waveVideo, voiceSpot, searchPro,
  ];

  static BdjProduct? byCode(String code) =>
      all.where((p) => p.code == code).firstOrNull;
}
```

Con esto, los puntos 2 a 7 se reducen a **una sola constante**: los mapas y los `switch` se sustituyen por consultas a `BdjProduct`, y el desplegable se construye con `BdjProduct.all.map(...)`. El punto 1, en el backend TypeScript, se mantiene sincronizado generando el array desde el mismo catálogo con un guion en CI que compara ambas listas y **hace fallar la compilación si divergen** — la lista de productos es una decisión de seguridad del backend y debe seguir siendo explícita en él, pero no debe poder desincronizarse en silencio.

Añadir la séptima aplicación de la suite pasaría entonces a ser una línea, no siete ediciones repartidas en tres lenguajes.

**Alcance del cambio:** `bdj_license_core` lo consumen las cinco aplicaciones por ruta relativa `../../bdj_license_core`, así que añadir un archivo nuevo no rompe ninguna — solo BDJ Studio License empezaría a usarlo. Las demás lo adoptan cuando toque tocarlas. Es un cambio aditivo y sin riesgo.

---

## 11. Seguridad

Un servicio que corre como `LocalSystem` y lee el disco entero es la superficie de ataque más delicada del producto. Las reglas no son negociables:

1. **El servicio no analiza datos que no controla.** Solo acepta un conjunto cerrado de órdenes de control por la tubería, codificadas con `postcard`, con longitud máxima acotada. No recibe consultas de búsqueda, no recibe rutas arbitrarias que abrir, no recibe nada que ejecutar.
2. **La tubería tiene un descriptor de seguridad explícito** que concede acceso únicamente al grupo de usuarios interactivos y a Administradores. Se crea con `FILE_FLAG_FIRST_PIPE_INSTANCE` para descartar el secuestro de la tubería, y el servidor llama a `ImpersonateNamedPipeClient` para comprobar quién habla antes de atender.
3. **La ruta caliente es de solo lectura.** El cliente mapea el índice sin permiso de escritura. No hay ninguna vía desde la interfaz para escribir en el índice.
4. **Todo el `unsafe` está aislado.** Vive en tres módulos — `index::mmap`, `fs::windows::raw`, `fs::macos::raw` — cada uno con un envoltorio seguro y su justificación documentada. El resto del código compila bajo `#![forbid(unsafe_code)]`.
5. **`cargo deny` y `cargo audit` en CI**, con la compilación fallando ante cualquier vulnerabilidad conocida o licencia incompatible en el árbol de dependencias.
6. **Firma de código:** certificado EV para el servicio y el ejecutable en Windows — sin él, SmartScreen bloquea un instalador que registra un servicio y el cliente no lo instala. Developer ID + notarización + grapado en macOS.
7. **Sin telemetría, sin red.** Search Pro no abre ningún socket. La validación de licencia es criptográfica y local. Es una afirmación verificable con un cortafuegos, y es un argumento de venta ante quien confía un índice de todo su disco a un programa.

### 11.1 Punto abierto: quién puede leer el índice

El archivo de índice contiene el listado completo de nombres de archivo de la máquina, incluidos los de los perfiles de otros usuarios. En un PC compartido eso es una fuga de información. Everything tiene el mismo problema y lo resuelve con una opción de permisos por carpeta.

Propuesta para la v1: el directorio del índice, bajo `%ProgramData%\BDJ Studio\Search Pro\`, lleva una ACL que concede lectura solo al usuario que instaló y a Administradores. Es correcto para el equipo de un DJ, que es de un solo usuario. Un índice por usuario con filtrado de permisos queda para una v2 y solo si aparece la demanda. **Requiere tu confirmación (§14).**

---

## 12. Pruebas y verificación

| Nivel | Qué cubre | Herramienta |
|---|---|---|
| Unitarias Rust | Parser, evaluador, comparador, formato del índice, aritmética de rutas | `cargo test` |
| *Fuzzing* | Parser de consultas y lector del formato de índice | `cargo-fuzz`, en el nocturno |
| Propiedades | El refinamiento incremental produce el mismo conjunto que el recorrido completo, para consultas generadas al azar | `proptest` |
| Bancos de prueba | Comparador, refinamiento, ordenamiento y apertura del índice sobre corpus sintéticos de 1 M y 10 M | `criterion`, con umbrales que **hacen fallar la compilación** si se supera el presupuesto |
| Integración FS | Crear, renombrar, mover y eliminar en un volumen VHDX de prueba → verificar el índice | `cargo test --features integration` |
| Widget Flutter | Lista virtual, caché de filas, cambio de orden, barra de estado | `flutter test` |
| Integración Flutter | Activación de licencia, atajo global, arrastrar y soltar | `integration_test` |
| Contratos | Firmas FFI, enums y ausencia de archivos huérfanos entre Rust y Dart | `tools/verify_contracts.py` |
| Calidad estática | Deuda técnica, código muerto, duplicación | SonarQube · `cargo clippy -D warnings` · `flutter analyze` |
| Estrés | 24 h con actividad sintética de disco, vigilando fugas de memoria y descriptores | Guion propio |

**El generador de corpus sintético** es la pieza que hace todo esto viable: produce un índice de 10 millones de entradas con una distribución realista de nombres, extensiones, profundidades y tamaños, en memoria, en unos segundos y sin tocar disco. Sin él, los presupuestos de rendimiento no serían comprobables en CI y se convertirían en buenas intenciones.

**La lección de Stems Music se aplica desde el primer día:** antes de dar por buena la eliminación de cualquier símbolo, se barre el árbol completo del repositorio, no solo los archivos tocados en la sesión. `verify_contracts.py` y `limpiar_muertos.ps1` se traen desde Stems Music y se adaptan.

---

## 13. Empaquetado y CI

### 13.1 Windows — `windows-build.yml` (nuevo)

```
checkout → checkout de bdj_license_core (dzapataba/bdjstudiolicensecore)
  → toolchain Rust estable, objetivo x86_64-pc-windows-msvc
  → cargo clippy -- -D warnings · cargo test --release · cargo deny check · cargo audit
  → cargo build --release  (bdj_search_indexer, bdj_search_ffi)
  → Flutter estable · flutter analyze · flutter test
  → flutter build windows --release
  → copiar el .dll del motor y el .exe del servicio junto al runner
  → firma con certificado EV (secreto del repositorio) de exe, dll y servicio
  → Inno Setup 6 → BDJ_Studio_Search_Pro_Setup_<versión>.exe
  → firma del instalador · SHA-256 · subida como artefacto
```

El `installer.iss` parte del de Sample Pad y añade el registro del servicio:

```ini
[Setup]
PrivilegesRequired=admin           ; ya estaba en Sample Pad; aquí es imprescindible

[Files]
Source: "..\frontend\build\windows\x64\runner\Release\*"; DestDir: "{app}"; \
        Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\engine\target\release\bdj_search_indexer.exe"; DestDir: "{app}"; \
        Flags: ignoreversion

[Run]
Filename: "{sys}\sc.exe"; Parameters: "create BDJSearchProIndexer \
    binPath= ""{app}\bdj_search_indexer.exe"" start= delayed-auto \
    DisplayName= ""BDJ Studio Search Pro Indexer"""; Flags: runhidden
Filename: "{sys}\sc.exe"; Parameters: "description BDJSearchProIndexer \
    ""Mantiene el índice de archivos de BDJ Studio Search Pro."""; Flags: runhidden
Filename: "{sys}\sc.exe"; Parameters: "start BDJSearchProIndexer"; Flags: runhidden
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; \
    Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\sc.exe"; Parameters: "stop BDJSearchProIndexer";   Flags: runhidden
Filename: "{sys}\sc.exe"; Parameters: "delete BDJSearchProIndexer"; Flags: runhidden

[UninstallDelete]
Type: filesandordirs; Name: "{commonappdata}\BDJ Studio\Search Pro"
```

`start= delayed-auto` es deliberado: el servicio no compite con el arranque de Windows, y como el USN Journal permite reanudar desde el último punto, no se pierde ni un cambio por arrancar treinta segundos más tarde.

### 13.2 macOS — `macos-build.yml` (calcado del de Sample Pad)

Se mantiene la estructura íntegra del workflow que ya tienes — matriz `arm64` sobre `macos-15` y `x64` sobre `macos-15-intel`, checkout de `bdj_license_core` con `CORE_TOKEN`, auditoría del Keychain del runner, mantenimiento de CocoaPods, compilación *smoke* con `BDJ_KEYCHAIN_CI_SMOKE=true`, validación de entitlements, prueba de Keychain sobre el binario con firma ad-hoc profunda, diagnóstico diferenciado de estados, lectura de informes IPS, limpieza, compilación Release final, verificación de que el modo CI no sobrevive en el binario comercial, *secret scan*, adelgazamiento con `lipo`, empaquetado con `ditto` y SHA-256.

Las tres adaptaciones:

1. **Toolchain de Rust** antes de `flutter pub get`, con los objetivos `aarch64-apple-darwin` y `x86_64-apple-darwin`, y los mismos `clippy` / `test` / `deny` / `audit` que en Windows.
2. **Aislamiento de entitlements.** La comprobación de Sample Pad rechaza el build si aparecen grupos de Keychain de otras apps:
   ```bash
   if echo "$ENTITLEMENTS_OUT" | grep -i -E 'wavevideo|license'; then
   ```
   En Search Pro pasa a `-E 'samplepad|stems|wavevideo|synth|voicespot|license'`. Cada app comprueba que no arrastra los grupos de las demás.
3. **Entitlement de Acceso Total al Disco.** El bundle necesita `com.apple.security.files.user-selected.read-only` y, para la distribución directa fuera de la Mac App Store, se compila **sin** el sandbox de aplicación (`com.apple.security.app-sandbox = false`), que es lo que permite que el TCC de Acceso Total al Disco surta efecto. El workflow verifica que el entitlement esté presente y falla si no.

Ambos flujos siguen el mismo modelo de distribución que el resto de la suite: **directa, fuera de tiendas**, con artefactos de GitHub Actions y Releases.

---

## 14. Decisiones abiertas

Tres cosas necesito de ti antes o durante la fase que corresponda. Ninguna bloquea el arranque.

**1. Acoplamiento de versión de la licencia — la más importante.**
`Spp3Token.verify` exige que `payload.ver` coincida **exactamente** con la versión `major.minor.patch` de la aplicación. Eso significa que **publicar la 1.0.1 invalida todas las licencias emitidas para la 1.0.0**, y cada cliente necesitaría una llave nueva por cada corrección de errores. En Sample Pad esto ya es así y hoy funciona porque publicas poco. En Search Pro, que es un producto que vas a iterar mucho al principio, se convierte en una carga operativa seria. Opciones: (a) dejarlo tal cual y emitir licencias nuevas en cada versión; (b) que el payload lleve un rango de versiones, lo que requiere tocar `bdj_license_core` y afectaría a las tres apps; (c) que Search Pro fije `defaultAppVersion` en la línea `1.0.x` y no lo suba en las correcciones. Me inclino por (b) a medio plazo, pero es una decisión de ecosistema, no de este producto.

**2. Permisos del archivo de índice** (§11.1). Confirmación de que la ACL por instalador para un equipo de un solo usuario es aceptable en la v1.

**3. Comportamiento con volúmenes desconectados.** Mi propuesta es conservar en el índice los archivos de un USB desconectado, mostrarlos atenuados y con la etiqueta del volumen. Es muy útil para un DJ («¿en qué USB tenía este set?») pero difiere de Everything, que los oculta. Confírmame que es el comportamiento que quieres por defecto.

**4. Catálogo de productos en BDJ Studio License** (§10.5). Las siete ediciones mínimas las hago igual. Lo que necesito decidir es si además muevo el catálogo a `BdjProduct` dentro de `bdj_license_core`. Es un cambio aditivo, no rompe ninguna de las cinco aplicaciones, y convierte «añadir un producto» de siete ediciones en tres lenguajes a una constante. Mi recomendación es hacerlo ahora, con Search Pro como el primer producto que lo estrena — cuanto más tarde, más sitios habrá que reconciliar.

---

## 15. Plan de ejecución

Nueve fases. Cada una tiene un entregable comprobable; ninguna se da por cerrada sin sus pruebas en verde y sus presupuestos medidos.

### Fase 0 — Andamiaje y contratos
Repositorio, *workspace* de Cargo con los cinco *crates*, proyecto Flutter, `flutter_rust_bridge` generando y compilando en ambas plataformas, los dos workflows de CI en verde sobre un esqueleto, `verify_contracts.py` adaptado, SonarQube conectado.
**Aceptación:** una función de prueba en Rust se invoca desde Flutter en Windows y en macOS, y ambos CI están en verde.

### Fase 1 — Núcleo del índice
Estructuras columnares, arena de nombres, extensiones internadas, formato binario documentado en `docs/FORMATO_INDICE.md`, constructor, apertura por `mmap`, base + superposición + lápidas, compactación, generador de corpus sintético.
**Aceptación:** un índice de 10 millones de entradas se construye en memoria, se escribe, se abre en **menos de 100 ms** y devuelve rutas completas correctas para una muestra al azar de mil entradas.

### Fase 2 — Motor de búsqueda
Lexer, parser, AST, evaluador, comparador SIMD sin distinción de mayúsculas, pila de refinamiento incremental, cancelación por generación, transmisión de los primeros resultados, ordenamiento por permutación y por radix, todas las funciones de consulta de §6.
**Aceptación:** sobre el corpus de 10 M, la primera pulsación devuelve 200 resultados en **menos de 30 ms** y las siguientes en **menos de 8 ms**, medido con `criterion`. `proptest` confirma que el refinamiento incremental coincide con el recorrido completo en diez mil consultas aleatorias.

### Fase 3 — Backend Windows
Servicio con su ciclo de vida completo, enumeración por `FSCTL_ENUM_USN_DATA`, fase 2 de tamaños y fechas, seguimiento del USN Journal con reanudación persistida, ruta de recorrido para exFAT/FAT32/red con `ReadDirectoryChangesW`, detección de montaje y desmontaje, tubería nombrada con su descriptor de seguridad y su protocolo de control.
**Aceptación:** en un equipo real con más de un millón de archivos, la indexación inicial termina en **menos de 20 s**; crear, renombrar y borrar un archivo se refleja en **menos de 500 ms**; el servicio consume **menos del 0,1 % de CPU** en reposo; tras reiniciar Windows el índice se pone al día sin reescanear.

### Fase 4 — Backend macOS
Enumeración con `getattrlistbulk`, `FSEvents` con eventos por archivo y reanudación por `FSEventStreamEventId`, manejo de `MustScanSubDirs`, `LaunchAgent`, pantalla y flujo de Acceso Total al Disco, detección de volúmenes con `getfsstat`.
**Aceptación:** un millón de archivos indexados en **menos de 30 s**; los cambios se reflejan en **menos de 500 ms**; tras cerrar y reabrir la aplicación, los cambios hechos entretanto aparecen sin reexplorar.

### Fase 5 — FFI y armazón de la interfaz
Contrato FFI completo, lista virtual con caché LRU de páginas, columnas ordenables, filtros rápidos, barra de estado, tema blanco íntegro.
**Aceptación:** el desplazamiento por 10 millones de resultados mantiene 60 fps y memoria constante; el presupuesto de tecla a fila pintada se cumple **medido en la aplicación real**, no solo en el banco de pruebas.

### Fase 6 — Licencia y funciones de escritorio
Integración SPP3 con el código de producto de Search Pro, pantalla de activación blanca con ID de dispositivo y botón de copia, portón de licencia en el arranque, atajo global con ventana emergente, residencia en la bandeja del sistema, menú contextual completo, arrastrar y soltar, selección de volúmenes.

**Trabajo en BDJ Studio License, previo a esta fase** (§10.5): los siete puntos del catálogo de productos, más el catálogo único `BdjProduct` en `bdj_license_core` si se aprueba la refactorización. Sin esto no hay forma de emitir una sola licencia de prueba, así que va primero.

**Aceptación:** BDJ Studio License emite una llave SPP3 para `bdj_studio_search_pro` y el backend acepta su sincronización; esa llave activa la aplicación; el ID de dispositivo mostrado **coincide con el que muestra Sample Pad en la misma máquina**; el atajo global abre la ventana en menos de 100 ms estando la aplicación residente.

### Fase 7 — Empaquetado y firma
Inno Setup con instalación, arranque y desinstalación del servicio; firma EV; workflow de macOS adaptado; Developer ID, notarización y grapado; verificación de instalación limpia y de desinstalación sin residuos en tres máquinas de prueba por plataforma.
**Aceptación:** instalación en un Windows limpio sin aviso de SmartScreen; en un macOS limpio sin aviso de Gatekeeper; la desinstalación no deja servicio, índice ni claves.

### Fase 8 — Endurecimiento
`cargo-fuzz` sobre el parser y el lector de índice, prueba de estrés de 24 h, perfilado y ajuste contra los presupuestos, barrido de código muerto sobre el árbol completo, SonarQube sin problemas críticos ni mayores, `docs/SINTAXIS_BUSQUEDA.md` para el usuario final.
**Aceptación:** todos los presupuestos de §2 medidos y cumplidos en hardware real de gama baja y de gama alta, en ambas plataformas.

### Fase 9 — Capa multimedia (posterior a la v1, ya prevista en el diseño)
Sobre el motor ya construido, una segunda capa que lee metadatos de audio bajo demanda y en segundo plano — artista, título, álbum, género, BPM, duración, frecuencia de muestreo, profundidad de bits — y habilita `bpm:128` y `artist:"David Guetta"`. Se guarda en un archivo lateral con la misma disciplina columnar y de mapeo en memoria, y **no toca el índice principal ni el presupuesto de latencia**. Está prevista en el diseño desde ahora precisamente para que añadirla no obligue a rehacer nada.

---

## 16. Riesgos

| Riesgo | Probabilidad | Impacto | Mitigación |
|---|---|---|---|
| SmartScreen bloquea el instalador por registrar un servicio | Alta sin certificado EV | Crítico: nadie instala | Certificado EV desde la fase 7; es un gasto obligatorio, no opcional |
| El usuario de macOS no concede Acceso Total al Disco | Media | Alto: el producto parece roto | Pantalla explicativa con enlace directo al panel; funcionamiento degradado y visible, nunca bloqueo |
| El presupuesto de pintado de Flutter no se cumple en gama baja | Media | Alto | Medir desde la fase 5, no al final. Palancas: texto plano, iconos precalculados, `RepaintBoundary`, densidad de fila |
| 571 MB de índice a 10 M molesta en máquinas de 8 GB | Media | Medio | Está mapeado y es reclamable; la selección de volúmenes es la palanca; arena secundaria para nombres largos si hace falta |
| El USN Journal se desborda en un equipo con mucha actividad | Baja | Medio | Se detecta el desbordamiento y se reenumera solo el volumen afectado |
| El acoplamiento exacto de versión de SPP3 genera carga de soporte | **Alta** | Alto | Decisión abierta §14.1, a resolver antes de la fase 6 |
| El catálogo de productos de License queda desincronizado entre sus siete lugares | Alta si se deja como está | Medio: licencias que se firman pero el backend rechaza | Catálogo único `BdjProduct` + comprobación en CI que compara la lista de Dart con la de TypeScript (§10.5) |
| Antivirus marca el servicio por leer el volumen en crudo | Media | Alto | Firma EV, y enviar el binario a los principales proveedores para su lista blanca antes del lanzamiento |
| `getattrlistbulk` se comporta distinto en volúmenes de red en macOS | Media | Bajo | Los montajes de red quedan excluidos por defecto; ruta de reserva con `readdir` |

---

## 17. Lo que hace bueno a este producto, en una frase

Un motor nativo que jamás toca el disco durante una búsqueda, alimentado por los mecanismos que cada sistema operativo ya mantiene para sí mismo — el USN Journal en Windows, el almacén de eventos de FSEvents en macOS —, con una interfaz que nunca sostiene más filas de las que caben en pantalla.

Todo lo demás del plan existe para proteger esas tres afirmaciones.
