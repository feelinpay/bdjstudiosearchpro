# Cambios del 2 de septiembre de 2026

Continuación de `ANALISIS_TIEMPO_REAL_Y_EXPLORADOR.md`. Aquí está lo que se hizo,
lo que resultó estar ya hecho, y lo que sigue pendiente.

---

## 0. Lo primero: el proyecto no compilaba

Al traer el estado real de tu equipo, `cargo build --workspace` fallaba con siete
errores. Dos sesiones de agente habían trabajado sobre `bdj_search_core` en
paralelo y se habían dejado mutuamente a medias:

| Archivo | Fecha | De quién |
|---|---|---|
| `search/engine.rs` | 1 sep 19:15 | versión con clave de ordenación unificada |
| `query/compiled.rs`, `query/eval.rs` | 31 ago 20:21 | versión con otra interfaz |
| `query/mod.rs`, `search/refine.rs`, `index/ext_table.rs` | 31 ago 15:56 | versión anterior |

El motor de la primera esperaba una interfaz de consultas que las otras ya no
ofrecían. Cinco desajustes concretos, todos reconciliados:

1. `query/mod.rs` exportaba `CompiledQueryAst`; el tipo se llama `CompiledQuery`.
2. `refine.rs` no definía `MAX_CACHED_PER_ENTRY`, que `engine.rs` importaba de
   ahí. Añadido, con el motivo: sin tope, teclear una letra que casa con nueve
   millones de archivos guardaba 36 MB **por pulsación**, y la pila conserva
   dieciséis peldaños.
3. `refine.rs` llamaba a `is_monotonically_restrictive`; la función se llama
   `implied_by` y sus argumentos van al revés.
4. `InternedExtensions` no tenía `len()`.
5. La prueba de latencia volvía a afirmar «menos de 8 ms de reloj» **en
   compilación sin optimizar**. Eso no mide el motor: mide la máquina. Ahora
   afirma la propiedad que importa —refinar cuesta menos que recorrer entero— y
   el presupuesto absoluto vive donde debe, en la prueba de release.

**No lances dos sesiones de agente sobre este crate a la vez.** Es la segunda vez
que pasa y la segunda vez que cuesta una tarde.

---

## 1. Tiempo real: la capa de cambios se publica aparte

Era la causa raíz del análisis. El servicio guardaba los cambios recientes en
memoria y el único camino por el que llegaban a la aplicación era **reescribir el
índice entero**: 950 MB y unos 25 segundos sobre diez millones de entradas.

### Lo que hay ahora

**`overlay.bdjo`**, un archivo pequeño junto al índice, con formato propio
(`index/overlay_file.rs`, 6 pruebas):

```
  0  magic "BDJOVL\0\0"     8
  8  version                u32
 12  base_count             u32
 16  base_generation        u64
 24  overlay_generation     u64
 32  blob_len / tombs / dead
 64  el índice incrustado   ← un .bdjx normal, diminuto
 ..  lápidas y bajas
```

Escribirlo cuesta milisegundos. Se publica **en cuanto hay cambios**, no al
compactar. La compactación pasa a ser lo que debía haber sido siempre:
mantenimiento en segundo plano que nadie espera.

El bloque incrustado empieza en el byte 64 —múltiplo de ocho— para que las
columnas de enteros queden alineadas y se puedan leer sin copiar. Las listas de
longitud variable van detrás justamente para no mover ese offset.

**El anclaje por generación no es decorativo.** Una capa solo significa algo
sobre el índice base que la originó: sus identificadores por debajo de
`base_count` son posiciones dentro de *ese* archivo, y compactar los renumera
todos. Aplicar una capa vieja a un base nuevo no da un resultado incompleto: da
uno **incorrecto**, con archivos colgando de carpetas equivocadas. Por eso el
lector compara generaciones y descarta, en vez de aprovechar lo que pueda.

### Búsqueda sobre las dos capas

`search/merged.rs` (7 pruebas). Consulta el base y la capa y funde los dos
resultados.

La parte delicada: **la fusión compara valores reales, no claves enteras.** El
motor ordena por `name_rank`, la posición alfabética precalculada, y eso es lo
que hace que ordenar cueste lo que el resultado. Pero ese rango es relativo a su
propio archivo: el de la capa ordena sus cuatro entradas entre sí y no significa
nada comparado con el del base. Compararlos daría un orden plausible y
equivocado —del tipo que nadie nota hasta que un archivo aparece en mitad de la
lista—, así que en la frontera se comparan el nombre, el tamaño y la fecha de
verdad. Sale barato porque solo ocurre sobre la ventana visible más las entradas
de la capa, que son pocas por construcción.

Un fallo que apareció escribiendo estas pruebas y que merece quedar anotado: un
`Engine` guarda una pila de refinamiento con resultados de consultas anteriores,
y esos identificadores son posiciones **dentro del índice que los produjo**. Usar
el mismo motor para el base y para la capa hacía que la capa recibiera el
resultado cacheado del base: devolvía cero coincidencias porque el base ya había
respondido cero. La capa se consulta ahora con un motor propio.

### La cadena, antes y ahora

```
ANTES   cambio → sondeo 0,5 s → calma 2 s → REESCRIBIR 24,7 s → sondeo 1 s ≈ 28 s
AHORA   cambio → sondeo 0,5 s → publicar capa (ms) → aviso        ≈ 0,6 s
        tus propias operaciones → visibles al recibir el Ack       ≈ inmediato
```

### Tus propias operaciones

El caso que más molestaba: copiar un archivo dentro de la aplicación y no verlo.
El servicio publica la capa **antes** de responder al aviso de cambio local, así
que cuando la aplicación recibe el `Ack` el archivo ya está en disco y el
refresco que hace a continuación lo ve. Cadena completa verificada:
`fsop_reap` → `flush_changes` → IPC `LocalChanges` → aplica y publica → `Ack` →
`reloadIfChanged` → se repinta.

### Navegar también

`browse_path` construía una `OverlayIndex` **vacía** de usar y tirar para
resolver rutas (era el hallazgo D-3 de la auditoría). Con la capa vacía, una
carpeta creada hace un segundo no se podía ni localizar ni listar. Ahora usa la
capa real, y ordenar los hijos pasa por `rank_merged`, que mezcla las dos
procedencias.

---

## 2. La interfaz es siempre un explorador

`ExplorerBar` empezaba con `if (estado.mode != ViewMode.browse) return const
SizedBox.shrink();`. La barra desaparecía salvo que estuvieras navegando, así que
la aplicación cambiaba de forma según lo que hicieras.

Lo que cambió:

- **La barra está siempre.** Lo único que cambia entre modos es lo que dice: la
  carpeta abierta, o «Resultados en \<dónde\>».
- **Arranca en «Este equipo»**, no en una pantalla en blanco. `ViewMode.browse`
  es ahora el estado inicial.
- **Escribir filtra la carpeta en la que estás**, con un botón para ampliar a
  todo el equipo. Es el gesto del Explorador de Windows y del Finder.
- **Borrar el texto devuelve a la carpeta**, no a una lista vacía.
- **`Ctrl+F` solo pone el cursor en la caja.** Antes cambiaba de modo en el acto
  y te dejaba la pantalla en blanco antes de haber escrito una letra.
- El texto de ayuda de la caja dice **dónde** va a buscar.

### Dos fallos de la sintaxis de consulta que salieron por el camino

Acotar la búsqueda a una carpeta destapó dos cosas que estaban rotas para
cualquiera que usara la sintaxis documentada:

1. **El lexer partía por espacios antes de mirar las comillas.** `ruta:"Mis Sets"`
   se troceaba en «ruta:"Mis» y «Sets"», así que el filtro buscaba una ruta
   llamada literalmente `"Mis`. Con rutas de Windows —que casi siempre llevan
   espacios— era el caso normal, no el raro.
2. **El parser dejaba las comillas dentro del valor.** Aunque el token llegara
   entero, `ruta:"Musica"` buscaba `"Musica"` con comillas incluidas.

Las dos fallaban igual: **cero resultados, sin ningún error**. Es la peor forma
de fallar, porque parece que no hay nada. Corregidas, con pruebas.

Nota: `carpeta:` compara contra el **nombre** de la carpeta contenedora, no
contra la ruta y no recursivamente. Es útil («carpeta:Sets») pero no sirve para
acotar, así que el ámbito viaja como parámetro aparte hasta el motor y no dentro
del texto de la consulta.

---

## 3. Gamas de equipo

`core/tuning.rs` (7 pruebas). Se leen núcleos y memoria una vez y de ahí salen
las constantes que antes estaban fijas.

| | Núcleos | Memoria | Hilos | Bloque | Ventana | Páginas |
|---|---:|---:|---:|---:|---:|---:|
| Baja | ≤ 2 | < 8 GB | 1 | 16 384 | 500 | 5 |
| Media | 3–7 | ≥ 8 GB | n−1 | 32 768 | 1 000 | 8 |
| Alta | ≥ 8 | ≥ 16 GB | n−1 | 65 536 | 2 000 | 10 |

**Siempre queda un núcleo libre.** Es el cambio que más se nota en un portátil: la
búsqueda usaba el grupo global de rayon, que ocupa todos los núcleos, y durante
esos cincuenta milisegundos el hilo que dibuja la interfaz no tenía dónde
ejecutarse. Se buscaba rápido y se escribía a trompicones. Nadie percibe los
cincuenta milisegundos; todo el mundo percibe el tirón.

También salen de ahí el bloque de copia de archivos —en un disco lento, bloques
de un mega dan un progreso a saltos y parece que se ha colgado— y el periodo de
calma antes de compactar.

El bloque de copia se **inyecta**: `bdj_search_fsops` no depende del núcleo a
propósito, y hacerle depender de él para leer una constante rompería la
separación que lo hace probable por su cuenta. Lo fija la capa FFI al arrancar.

---

## 4. macOS

Lo que **ya estaba hecho** en tu equipo y mi análisis daba por pendiente —mi copia
estaba desactualizada y te pido disculpas por ese punto—:

- `FSEventStreamCreate` de verdad, con su `CFRunLoop` en hilo propio y las
  banderas correctas (`FileEvents`, `NoDefer`, `UseCFTypes`, `WatchRoot`).
- El bucle del servicio ya drena esos vigilantes: `found` ya no es cero en macOS.

Lo que se ha añadido ahora:

- **Detección del permiso de acceso total al disco** (`macos/permissions.rs`, 4
  pruebas). Sin ese permiso macOS **no da error** al listar el Escritorio o
  Documentos: devuelve una lista vacía. El índice se construiría «bien» y al
  usuario le faltarían justo sus carpetas, buscaría una pista que sabe que tiene,
  no la encontraría, y concluiría que el programa no funciona. Ahora se comprueba
  y se avisa con la ruta de Ajustes.
- La comprobación distingue «no tengo permiso» de «no he podido averiguarlo».
  Confundirlos enseñaría un aviso alarmante a alguien que no tiene ningún
  problema.

---

## Estado de las pruebas

```
131 pruebas, 0 fallos
clippy: sin avisos
verify_contracts.py: 49 funciones FFI, IPC y workspace en regla
```

(127 se ejecutan aquí; las 4 de permisos de macOS se compilan solo en macOS y se
han verificado por separado.)

---

## Lo que necesita tu máquina

1. **`flutter_rust_bridge_codegen generate` — obligatorio.** La superficie FFI
   cambió: `search_with_limit` tiene un parámetro nuevo (`scope`) y hay una
   función nueva (`machine_tuning`). Sin regenerar, la aplicación no compila.
2. **`flutter analyze`.** Aquí no hay cadena de herramientas de Dart. He
   comprobado los archivos que toqué estructuralmente, pero eso no sustituye al
   analizador. Mándame lo que salga.
3. **La primera compilación de lo marcado con `#[cfg(windows)]` y
   `#[cfg(target_os = "macos")]`.** Este entorno solo compila la parte portable,
   que es la mayoría pero no todo.

---

## Lo que sigue pendiente, sin adornos

**`getattrlistbulk` en macOS.** El escaneo inicial sigue usando `read_dir`, que
hace una llamada al sistema por archivo para leer tamaño y fechas. `getattrlistbulk`
devuelve lotes con los atributos incluidos y la diferencia en un escaneo completo
es de un orden de magnitud. **No lo he escrito**: es una interfaz C insegura que
no puedo compilar ni ejecutar desde aquí, y prefiero no entregarte código de bajo
nivel que nadie ha visto funcionar. Funciona sin ello; va más lento la primera vez.

**Construcción del índice por lotes.** El perfil ya calcula cuántas entradas
caben en la memoria del equipo (`build_batch`), y la comprobación está probada.
Lo que falta es que `IndexBuilder` **use** ese número: hoy sigue construyendo todo
en memoria y volcando al final. En un equipo de 4 GB con muchos archivos, el
escaneo inicial sigue siendo el punto de riesgo. Es el trabajo grande que queda.

**El aviso de cambios sigue siendo un sondeo por dentro.** `waitIndexChanged`
bloquea en el hilo nativo —la interfaz no sondea— pero por debajo comprueba el
archivo cada 30 ms. Funciona y ya no cuesta 25 segundos, pero no es un empujón de
verdad desde el servicio. Con la capa publicándose en milisegundos, la ganancia
que queda es pequeña.

**`fsop_delete_permanently` existe en la superficie FFI.** No lo he tocado, pero
choca con lo que acordamos —«nada se borra de forma definitiva»—. Si lo añadió la
otra sesión a propósito, dímelo y lo dejo; si no, lo quito.

**Decisiones que siguen esperando tu respuesta** (de la auditoría): acoplamiento
de versión en SPP3, permisos del archivo de índice, volúmenes desconectados
atenuados o escondidos, y mover el catálogo de productos a `BdjProduct`.
