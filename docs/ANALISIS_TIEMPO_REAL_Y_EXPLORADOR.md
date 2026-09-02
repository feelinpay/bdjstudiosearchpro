# Análisis: tiempo real, interfaz de explorador, arranque y gamas de equipo

2 de septiembre de 2026. **No he tocado nada del código para escribir esto.**

Cinco cosas me has señalado. Las he verificado una por una contra el código que
hay ahora mismo en disco, no contra lo que creo recordar que escribí. Las cinco
son ciertas, y cuatro de ellas tienen la misma raíz o una muy parecida.

| Lo que dices | Verdicto | Dónde está |
|---|---|---|
| «No hace cambios en tiempo real» | **Cierto.** Peor de lo que parece: hasta ~28 s | §1 |
| «No tiene la interfaz de un explorador salvo que la elija» | **Cierto.** Está escrito así a propósito, y fue una decisión mía equivocada | §2 |
| «Al inicio tiene un pestañeo de error» | **Cierto.** Es un fallo mío de una línea | §3 |
| «Todo debe estar veloz y funcional para Windows y macOS» | **En macOS no hay detección de cambios.** Ninguna | §4 |
| «Debe estar disponible para todas las gamas» | **Ahora mismo no.** Hay tres constantes fijas que asumen tu equipo | §5 |

---

# §1 · Por qué no ves los cambios en tiempo real

## 1.1 La cadena completa, medida

Cuando aparece un archivo nuevo en el disco, esto es lo que tiene que pasar
antes de que tú lo veas en pantalla:

```
  archivo nuevo en disco
        │
        │  ①  el servicio sondea el diario USN cada 500 ms
        ▼
  entra en la CAPA DE CAMBIOS (overlay), en la RAM del servicio
        │
        │  ②  el servicio espera a que el disco se calme:
        │      2 s si el índice tiene más de 2 M entradas, 500 ms si no
        ▼
  compactación: se REESCRIBE EL ÍNDICE ENTERO a disco
        │
        │  ③  medido: 950,8 MB en 24,7 s para 10 M de entradas
        ▼
  index.bdjx nuevo, con generación N+1
        │
        │  ④  la aplicación pregunta cada 1 s si cambió la generación
        ▼
  la aplicación remapea y repite la consulta → aparece en pantalla
```

Sumando, con un índice de diez millones de entradas:

**0,5 s + 2 s + 24,7 s + 1 s ≈ 28 segundos.**

Con un índice pequeño (100 000 entradas) la reescritura baja a unos 300 ms y el
total queda en ~2,3 s. Sigue sin ser tiempo real, pero no se nota tanto. Por eso
esto se ve mucho peor en tu equipo que en cualquier prueba.

Y hay un caso peor todavía. El paso ② no dispara solo por calma: también dispara
si la capa crece mucho. El umbral es `base_count / 20`:

```rust
let threshold = (base_count / 20).max(500);   // overlay.rs:135
```

Con 10 M de entradas eso son **500 000 cambios**. Mientras estés copiando una
biblioteca —es decir, mientras el disco no se calma—, no se publica nada hasta
llegar a medio millón de archivos. Durante una importación larga el índice se
queda congelado.

## 1.2 La causa de verdad: la aplicación no ve la capa de cambios

Lo anterior son síntomas. La causa es una sola línea, y es un hallazgo de la
auditoría (D-3) que no llegué a arreglar.

El servicio tiene dos cosas: el índice base en disco (`index.bdjx`) y una **capa
de cambios** en RAM con todo lo que ha pasado desde la última publicación. La
aplicación solo mapea el archivo. Lo comprobé buscando el tipo en toda la
superficie FFI:

```
$ grep -rn "OverlayIndex" engine/bdj_search_ffi/src/
api.rs:523:  let overlay = OverlayIndex::with_base_count(view.entry_count() as u32);
```

Un solo resultado, dentro de `browse_path`, y es una capa **vacía** que se crea
en ese momento solo para resolver una ruta. No es la capa del servicio. No hay
ningún camino por el que la aplicación pueda leerla.

Consecuencia: **el único mecanismo que tiene la aplicación para enterarse de algo
es que se reescriba el archivo entero.** De ahí los 24,7 segundos. No es que el
sondeo sea lento; es que el sondeo está esperando a la operación más cara del
sistema.

## 1.3 El caso que más molesta: tus propias operaciones

Esto es lo que hace que se note tanto. Copias un archivo *desde dentro de la
aplicación*. El gestor de operaciones termina, avisa al servicio, el servicio lo
mete en su capa, y la aplicación llama a:

```dart
Future<void> refreshAfterFileOperation() async {
  await ffi.reloadIfChanged();     // remapea el archivo base... que no ha cambiado
  await _repeatCurrentView();      // repite la consulta sobre el índice viejo
}
```

`reloadIfChanged` solo mira si el archivo base tiene otra generación. No la
tiene. Así que se repite la consulta sobre el mismo índice de siempre y **el
archivo que acabas de copiar tú mismo no aparece**. Reaparece veinticinco
segundos después, cuando compacte.

Lo mismo con navegar: `browse_path` lee la lista de hijos del índice base. Creas
una carpeta y no está.

Un usuario que acaba de pulsar «pegar» y no ve el archivo asume que la operación
falló. Es el peor fallo de los cinco.

## 1.4 Preguntas: «no sé si usas sockets o algo»

Sí hay un canal, y no se usa para esto. Hay una tubería con nombre en Windows
(`\\.\pipe\...`) y un socket unix en macOS, con codificación postcard. Pero es
**unidireccional en el sentido que importa**: la aplicación manda órdenes
(excluir un volumen, reescanear, comunicar la licencia) y el servicio responde.
El servicio no tiene forma de decirle nada a la aplicación por iniciativa propia.
Por eso la aplicación sondea cada segundo.

Un socket no arregla esto por sí solo. Aunque el servicio pudiera gritar «ha
cambiado algo», la aplicación tendría que ir a leer el cambio a algún sitio, y
ese sitio hoy solo puede ser el archivo base reescrito. **El canal de aviso y el
canal de datos son dos problemas distintos y hay que resolver los dos.**

## 1.5 Cómo lo arreglaría

Tres piezas. La primera sola ya baja el peor caso de 28 s a menos de un segundo.

### Pieza A — Publicar la capa como un segundo archivo pequeño

El servicio escribe `overlay.bdjx` junto al índice: mismo formato columnar, pero
solo con lo que ha cambiado desde la última compactación. Son kilobytes, no
megabytes; escribirlo cuesta milisegundos, no veinticinco segundos.

La aplicación mapea **dos** archivos y consulta sobre la unión: el base más la
capa, menos las lápidas. Esa lógica ya existe y está probada —es `OverlayIndex`
con su `dead_overlay`—; lo que falta es que el objeto viva del lado de la
aplicación en lugar de solo del lado del servicio.

La compactación pasa a ser lo que debería haber sido siempre: mantenimiento en
segundo plano que ocurre cuando la capa crece, no el camino por el que viajan los
cambios.

- Frecuencia de publicación de la capa: cada 200–300 ms mientras haya cambios.
- Coste de leerla en la aplicación: un `mmap` de un archivo pequeño.
- Riesgo: hay que escribirla de forma atómica (temporal + `rename`) para que la
  aplicación nunca mapee un archivo a medio escribir. El mismo patrón que ya usa
  `settings.json`.

### Pieza B — Un aviso por el canal que ya existe

En vez de sondear cada segundo, la aplicación abre la tubería y se queda
esperando. El servicio manda un `IndexUpdated { generation, overlay_generation }`
en cuanto publica.

Esto quita la latencia del sondeo (hasta 1 s) y, más importante, **quita el coste
de sondear cuando no pasa nada**: ahora mismo la aplicación hace una llamada FFI
por segundo para siempre, esté el usuario haciendo algo o no. En un portátil eso
es batería.

Requiere un hilo lector en el lado Dart —o un `Stream` de flutter_rust_bridge,
que es lo natural— y reconexión con reintento cuando el servicio se reinicia.

### Pieza C — Aplicación optimista de tus propias operaciones

Para lo que hace el usuario dentro de la aplicación no hace falta esperar a
nadie. Cuando `bdj_search_fsops` termina una copia, la aplicación ya sabe la ruta
exacta del archivo nuevo: puede meterlo en su propia capa local **en el mismo
fotograma** y luego dejar que el servicio confirme.

La `SuppressionWindow` de tres segundos que ya escribí está pensada exactamente
para esto: evita que el cambio se aplique dos veces cuando el vigilante del
sistema de archivos anuncie lo mismo. Lo que faltaba era el otro extremo.

Con las tres piezas, la cadena queda así:

```
  tu operación         → visible en el fotograma siguiente  (~16 ms)
  cambio externo       → 500 ms de sondeo + publicación de capa + aviso  (~600 ms)
  compactación         → sigue costando 24 s, pero ya no la espera nadie
```

**Esfuerzo estimado:** pieza A, dos días; pieza B, un día; pieza C, medio día.
La A es la que hay que hacer sí o sí; las otras dos son mejoras encima.

---

# §2 · Por qué no siempre ves un explorador

## 2.1 Qué hace hoy

Lo escribí literalmente así:

```dart
// explorer_bar.dart:19
if (estado.mode != ViewMode.browse) return const SizedBox.shrink();
```

La barra de navegación —atrás, adelante, subir, miga de pan— **desaparece del
todo** salvo que estés navegando una carpeta. Hay dos modos, `search` y `browse`,
y la interfaz cambia de forma entre ellos.

Monté el explorador como una función añadida al buscador. Tú me estás diciendo
que la relación es la inversa:

> «Es tal cual, sino que a diferencia es la búsqueda rápida con filtros
> avanzados.»

Es un explorador de archivos. La búsqueda es lo que lo distingue, no lo que lo
define. Y visto así mi diseño está del revés.

## 2.2 Qué debería ser

Un solo marco, siempre presente, con el mismo aspecto haga lo que haga el
usuario:

```
┌──────────────────────────────────────────────────────────────────┐
│  ←  →  ↑    │  Este equipo › Música › Sets › 2026        │ 🔍   │  ← siempre
├─────────────┼────────────────────────────────────────────────────┤
│ Acceso rápido│  Nombre        Ruta      Tamaño   Modificado  Tipo│  ← siempre
│  Escritorio  │  ─────────────────────────────────────────────────│
│  Descargas   │  kick_01.wav   …\Sets    2,1 MB   ayer      Audio │
│  Música ★    │  bass_hi.wav   …\Sets    1,4 MB   ayer      Audio │  ← lo único
│              │  ...                                             │     que cambia
│ Este equipo  │                                                  │
│  C:  D:  E:  │                                                  │
├─────────────┴────────────────────────────────────────────────────┤
│ 1.204 objetos (8 ms)              C:\Música\Sets\2026\kick_01.wav│  ← siempre
└──────────────────────────────────────────────────────────────────┘
```

La miga de pan siempre está. Cuando buscas, dice de dónde salen los resultados
(«Resultados en Este equipo», o la carpeta en la que has acotado la búsqueda).
Las columnas siempre están. El panel lateral siempre está. Lo único que cambia es
de dónde vienen las filas: de `browse_path` o de `search_with_limit`.

Escribir en la caja de búsqueda **filtra la carpeta en la que estás**, como en el
Explorador de Windows, y con una opción de ampliar a todo el equipo. Ese es el
gesto que ya conoce cualquiera.

## 2.3 Lo bueno: el motor ya está preparado

Esto es principalmente trabajo de interfaz, no de motor. Las dos partes ya
comparten lo que hay que compartir:

- La misma tabla virtualizada.
- El mismo modelo de `Selection`.
- El mismo arrastre.
- El mismo menú contextual y las mismas operaciones de archivo.
- `Engine::rank()` ya ordena un conjunto conocido de identificadores para
  navegar, sin pasar por la pila de refinamiento de búsqueda.

Lo que hay que hacer:

1. Quitar el `SizedBox.shrink()` y hacer la barra permanente, cambiando su
   contenido según el modo en lugar de su existencia.
2. Arrancar en `browse` sobre «Este equipo» en vez de en `search` con la lista
   vacía. Hoy la primera pantalla está en blanco hasta que escribes algo: eso
   ya de por sí no parece un explorador.
3. El panel lateral con volúmenes, favoritos y árbol de carpetas — es lo que
   quedó pendiente de la fase 10.
4. La caja de búsqueda acotada a la carpeta actual por defecto, con un
   interruptor a «todo el equipo».
5. Vistas alternativas (iconos, lista, mosaicos). Hoy solo existe «detalles».

**Esfuerzo:** los puntos 1, 2 y 4 son un día. El panel lateral, dos o tres. Las
vistas alternativas, dos.

---

# §3 · El pestañeo de error al arrancar

Este es un fallo mío de una línea y tienes toda la razón en que asusta.

En `engine_health.dart`, el estado inicial es:

```dart
const EngineHealth({
  this.isOpen = false,
  ...
  this.problem = IndexProblem.unknown,        // ← aquí
  this.message = 'Comprobando el índice…',
});

static const EngineHealth checking = EngineHealth();
```

El mensaje dice «Comprobando el índice…», que es lo correcto. Pero el `problem`
por defecto es `unknown`, y la barra de estado no mira el mensaje para decidir el
color: mira el problema.

```dart
// status_bar.dart
} else if (salud.problem.resolvesItself) {
  color = Colors.orange; icono = Icons.sync;
} else {
  color = Colors.red;    icono = Icons.error_outline;   // ← cae aquí
}
```

Y `shortLabel` traduce `unknown` a **«No se pudo abrir el índice»**.

Así que entre que arranca la aplicación y que responde la primera llamada a
`engineStatus()` —abrir y mapear un archivo de casi un giga: decenas o cientos de
milisegundos— la barra muestra, **en rojo y con un icono de error**, que no se
pudo abrir el índice. Cada vez que abres el programa. Y no es verdad: todavía no
se ha intentado.

**El arreglo** es añadir un estado `IndexProblem.checking` que sea el valor por
defecto, se pinte en gris y neutro, y no se considere ni error ni aviso. Es media
hora de trabajo.

Y de paso, dos cosas del mismo estilo que conviene arreglar juntas:

- El primer fotograma con `entryCount = 0` hace que la barra diga «0 objetos
  (0 ms)» antes de haber consultado nada. Mismo problema: un cero que parece un
  resultado y es una respuesta que aún no ha llegado.
- Si el servicio está reconstruyendo el índice tras instalar (que es lo normal la
  primera vez), lo correcto no es una barra de estado en naranja sino una
  pantalla que diga qué está pasando y cuánto lleva. Ahora mismo el primer
  arranque después de instalar parece que el programa está roto.

---

# §4 · macOS: la paridad no existe hoy

Me pides que todo sea «veloz y funcional para Windows y macOS». En macOS hoy no
es funcional en lo que a tiempo real se refiere, y quiero decirlo claro.

El bucle del servicio:

```rust
#[cfg(windows)]
let found = {
    let (aplicados, sin_diario) = self.poll_usn_changes();
    ...
};
#[cfg(not(windows))]
let found = 0usize;              // ← main.rs
```

**En macOS `found` es siempre cero.** Nunca. El servicio hace el escaneo inicial
con `read_dir`, publica el índice, y a partir de ahí no se entera de
absolutamente nada que pase en el disco. Un archivo nuevo no aparece jamás hasta
que reinicies el servicio.

Y `fsevents.rs`, que en la auditoría di por «pendiente», son 72 líneas que
decodifican banderas y guardan dos campos en una estructura:

```
pub struct MacFsChangeEvent   — el tipo de evento
pub struct FsEventStreamWatcher
pub fn new(path, start_event_id) -> Self    — guarda los argumentos
pub fn parse_event(...)                     — traduce banderas a un enum
```

No hay ni una llamada a `FSEventStreamCreate`. No hay `extern "C"`. Es el
esqueleto correcto de una pieza que no está construida.

Lo que falta en macOS, por orden de importancia:

1. **`FSEventStreamCreate` de verdad**, con su `CFRunLoop` en un hilo propio,
   `kFSEventStreamCreateFlagFileEvents` para tener eventos por archivo y no por
   carpeta, y persistir el `eventId` para poder retomar tras un reinicio sin
   reescanear.
2. **`getattrlistbulk`** para el escaneo inicial. `read_dir` hace una llamada al
   sistema por archivo para leer tamaño y fechas; `getattrlistbulk` devuelve
   lotes con los atributos incluidos. La diferencia en un escaneo completo es de
   un orden de magnitud.
3. **Permiso de acceso total al disco.** Sin él, macOS oculta al servicio el
   escritorio, documentos y descargas del usuario **sin dar error**: simplemente
   no aparecen. Hay que detectarlo y pedirlo explícitamente, no descubrirlo
   porque faltan archivos.
4. La firma y notarización del agente para que arranque sin que Gatekeeper lo
   bloquee.

**Esfuerzo:** el punto 1 son dos días —el FFI a CoreFoundation es engorroso—, el
2 es un día, el 3 medio día. Es la mayor bolsa de trabajo pendiente que queda.

Aviso honesto: **no puedo compilar ni probar nada de esto desde aquí.** Este
entorno no tiene ni toolchain de macOS ni de Windows. Puedo escribirlo con
cuidado, pero la primera compilación real la tienes que hacer tú o el runner de
GitHub Actions.

---

# §5 · Gamas de equipo: baja, media y alta

Hoy el sistema está calibrado, sin decirlo, para un equipo como el tuyo. Estos
son los puntos donde se rompe en un equipo modesto.

## 5.1 Memoria durante el escaneo inicial

El constructor del índice mantiene **todo en RAM** hasta que escribe:

| Columna | Bytes por entrada |
|---|---:|
| padre, offset de nombre, orden alfabético, rango | 16 |
| tamaño | 8 |
| mtime, ctime | 8 |
| extensión, longitud, banderas, volumen | 5 |
| arena de nombres (media) | ~24 |
| lista de hijos | ~4 |
| **Total** | **~65 B** |

Medido: el archivo publicado son **950,8 MB para 10 M de entradas** (~95 B por
entrada, con las tablas). El pico de RAM durante la construcción es del mismo
orden, más el buffer de escritura.

**En un equipo de 4 GB con un disco de 10 M de archivos, el escaneo inicial puede
tumbar el servicio o hacer que el sistema empiece a paginar.** No hay ningún
control sobre esto: `IndexBuilder` reserva y crece hasta terminar.

Hace falta construcción **por lotes**: volcar a disco cada N entradas y fundir al
final, o construir por volumen y unir. Es el cambio más importante de esta
sección.

## 5.2 Constantes fijas que deberían depender de la máquina

| Constante | Valor hoy | Problema en gama baja |
|---|---|---|
| `CHUNK_SIZE = 65_536` (engine.rs) | fijo | Con 2 núcleos y caché pequeña, bloques más chicos aprovechan mejor la L2 |
| `_initialLimit = 2000` (Dart) | fijo | Pedir 2000 filas para pintar 30 es trabajo tirado en un equipo lento |
| `maxPages = 10` × `pageSize = 200` | 2000 filas en RAM | Razonable, pero podría ser 500 en gama baja |
| `CHUNK = 1 MB` (fsops) | fijo | En un disco lento, bloques de 1 MB dan progreso a saltos |
| `quiet_period` | 2 s o 500 ms | Solo mira el tamaño del índice, no la velocidad del disco |
| Hilos de rayon | todos los núcleos | En un portátil de 2 núcleos deja la interfaz sin CPU al buscar |

Ninguna de estas es difícil de arreglar. Lo que falta es una pieza que decida:
leer `available_parallelism()`, la RAM total y si el disco es SSD o mecánico al
arrancar, y derivar de ahí un perfil —bajo, medio, alto— que fije estas
constantes. Un archivo, unas cien líneas, y todas las constantes pasan a leerse
de ahí.

## 5.3 El caso concreto que peor se comporta

Un portátil de 2 núcleos y 8 GB con 2 M de archivos, buscando una sola letra:

- Búsqueda: ~50 ms. Bien.
- Pero **rayon usa los dos núcleos**, así que durante esos 50 ms la interfaz de
  Flutter no tiene CPU. A 120 ms de retardo entre teclas, eso es un tirón visible
  al escribir.

En gama baja hay que dejar **un núcleo libre** para la interfaz. Se busca un poco
más lento y se escribe con fluidez, que es lo que el usuario percibe.

## 5.4 Modo de bajo consumo

Un portátil con batería no debería tener un servicio sondeando el diario USN cada
500 ms indefinidamente. Con la pieza B de §1 —avisos en lugar de sondeo— gran
parte de esto se arregla solo, pero el servicio también debería espaciar su
sondeo cuando lleva minutos sin ver cambios, y volver al ritmo normal en cuanto
vea uno.

---

# §6 · Plan que propongo, en orden

Está ordenado por «cuánto arregla de lo que has señalado, dividido por lo que
cuesta». No he empezado nada: espero tu aprobación como la otra vez.

| # | Qué | Arregla | Esfuerzo |
|---|---|---|---|
| 1 | `IndexProblem.checking` neutro + primer fotograma honesto | §3 completo | 0,5 día |
| 2 | Capa optimista local: tus operaciones se ven al instante | §1.3, lo que más molesta | 0,5 día |
| 3 | Marco de explorador permanente + arranque en «Este equipo» | §2, la mayor parte | 1 día |
| 4 | `overlay.bdjx`: la aplicación lee la capa del servicio | §1, la causa raíz | 2 días |
| 5 | Avisos por la tubería en vez de sondeo | §1 latencia + batería | 1 día |
| 6 | Perfil de máquina y constantes derivadas | §5.2, §5.3 | 1 día |
| 7 | Panel lateral: volúmenes, favoritos, árbol | §2 resto | 2–3 días |
| 8 | FSEvents real + `getattrlistbulk` + permiso de disco | §4 | 3–4 días |
| 9 | Construcción por lotes del índice | §5.1 | 2 días |
| 10 | Vistas alternativas (iconos, lista, mosaicos) | §2 resto | 2 días |

Los tres primeros son día y medio en total y se llevan por delante el pestañeo,
la sensación de que las operaciones no funcionan, y la forma de la interfaz. Si
solo quieres que haga una cosa, que sea del 1 al 3.

---

# §7 · Antes de empezar, cosas que siguen pendientes de ti

Estas venían de la tanda anterior y siguen sin resolverse:

1. **`flutter_rust_bridge_codegen generate`** — obligatorio, la superficie FFI
   pasó de 12 a 42 funciones. Sin esto no compila.
2. **`flutter analyze`** — no tengo Dart aquí. Mándame la salida.
3. **Borrar tres archivos huérfanos.** No tengo permiso de borrado en tu equipo:
   - `engine/bdj_search_core/src/file_ops.rs`
   - `engine/bdj_search_core/src/file_ops_manager.rs`
   - `engine/bdj_search_core/src/sort/perm.rs`

   Ya no están referenciados desde `lib.rs` ni desde `Cargo.toml`, así que no
   rompen la compilación, pero conviene que desaparezcan.
4. **No lances dos sesiones de agente sobre `bdj_search_core` a la vez.** El
   fallo de compilación que me pasaste vino de ahí: otra sesión reintrodujo
   `file_ops.rs` con dependencias que no están declaradas, y revirtió la
   corrección de la consulta vacía.

Y cuatro decisiones de la auditoría que siguen esperando: el acoplamiento exacto
de versión en SPP3, los permisos del archivo de índice, si los volúmenes
desconectados se atenúan o se esconden, y si el catálogo de productos se mueve a
`BdjProduct` dentro de `bdj_license_core`.
