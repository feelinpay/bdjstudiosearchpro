# Tiempo de búsqueda y de respuesta — medido, no estimado

Fecha: 3 de septiembre de 2026. Todo lo que sigue son mediciones reales, corridas en esta sesión, sobre la máquina de este entorno (**2 núcleos, 7,8 GB de RAM** — que además es, por sus propias características, un buen sustituto de un equipo de **gama baja** real). No se modificó ningún archivo del proyecto para este análisis: se usó el banco de pruebas que ya existe en el repositorio, `engine/bdj_search_core/examples/bench_corpus.rs`, compilado en modo release.

## Cómo se midió

`bench_corpus` construye un índice sintético con la misma forma que produce el indexador real (nombres de artista/pista/mezcla, extensiones mezcladas de audio y de proyecto) y mide, con un motor recién creado cada vez, el tiempo de: una consulta fría de una sola letra, un filtro de extensión, un filtro de tipo, un filtro de tamaño, una palabra completa, y una secuencia de tecleo progresivo letra a letra (que es donde vive el mecanismo de refinamiento incremental). Se probó con tres tamaños: 2 millones de entradas (la línea base de la sesión anterior), 3,7 millones (el tamaño real de tu disco, según el `index.bdjx` que mediste), y 10 millones (el presupuesto que el propio código se propone como objetivo, documentado en el encabezado de `bench_corpus.rs`).

## Los números

Todo en milisegundos, gama baja real (2 núcleos, hilo de búsqueda = 1 — se explica más abajo por qué):

| Consulta | 2 M entradas | 3,7 M (tu disco) | 10 M (objetivo del producto) |
|---|---:|---:|---:|
| `m` (primera letra, fría) | 70 | 110–156 | 302–325 |
| `michael` (palabra completa, fría) | 52 | 90–103 | 245–258 |
| segunda letra (`mi`) | 59 | 108 | 331 |
| tercera letra (`mic`) | 49 | 95 | 248 |
| cuarta letra en adelante (`mich…michael`) | **7** | **14** | **35–41** |
| `ext:wav` | 14 | 24 | 66 |
| abrir el índice (`mmap`) | 0,07 | 0,06 | 0,07 |

La fila que importa es la última de refinamiento (7 / 14 / 35 ms): ahí es donde vive la promesa de "escribir letra a letra sin esperar", y el motor la cumple con margen de sobra en los tres tamaños — de hecho mejora con el tamaño del índice en proporción, no se dispara. Ese mecanismo —cada pulsación busca solo dentro de lo que sobrevivió a la anterior, no en el índice entero— es el mismo que verificamos la sesión pasada con la prueba `incremental_vs_full_test`, y aquí se confirma con números reales de reloj, no solo con el conteo de entradas examinadas.

El problema está en las primeras filas: **la primerísima letra que se teclea, y cualquier consulta que no tenga nada previo que refinar, cuesta entre 70 ms (2 M) y 325 ms (10 M).** El propio comentario en `bench_corpus.rs` fija el objetivo del producto en 30 ms para esa primera pulsación sobre 10 millones de entradas. La medición real en gama baja lo incumple por un factor de diez.

## Por qué pasa esto — la causa está localizada y es una sola

`bdj_search_core/src/tuning.rs` deriva cuántos hilos puede usar la búsqueda con esta regla: `search_threads = cores - 1`, para dejar siempre un núcleo libre para que la interfaz de Flutter siga dibujando mientras se busca. Es una decisión correcta y bien razonada — está documentada con el motivo exacto: sin ella, buscar ocupa todos los núcleos y el hilo que dibuja la interfaz se queda esperando, y eso se nota mucho más que una búsqueda unos milisegundos más lenta.

Pero en una máquina de **dos** núcleos —que es gama baja real, no un caso raro; probablemente buena parte de tu base de usuarios con equipos viejos cae aquí— esa resta da `search_threads = 1`. La búsqueda deja de ser paralela: se convierte en un recorrido completamente secuencial de todo el índice, en un solo hilo, para cualquier consulta que no tenga un resultado anterior del que partir. El coste crece de forma prácticamente lineal con el tamaño del índice porque no hay ningún otro cuello de botella que lo esté limitando (abrir el índice, como se ve arriba, cuesta microsegundos): 70 ms a 2 M, 110-156 ms a 3,7 M, 300-325 ms a 10 M — el triple de entradas, el triple de tiempo, casi exacto.

Para confirmar que la causa es exactamente esa y no otra cosa, corrí el mismo banco de pruebas tres veces sobre la misma máquina física de dos núcleos, cambiando solo el perfil de hilos que el motor cree tener disponible (`BDJ_TIER`, una variable de entorno que ya existe en el código para poder probar los tres perfiles sin tres máquinas):

| Perfil simulado | Hilos de búsqueda asumidos | `m` sobre 3,7 M |
|---|---:|---:|
| Baja (real, sin forzar) | 1 | 110–156 ms |
| Media (forzando 3 hilos en 2 núcleos reales) | 3 | 73 ms |
| Alta (forzando 7 hilos en 2 núcleos reales) | 7 | 74 ms |

Con solo permitirle a la búsqueda competir por el segundo núcleo (aunque sea de forma forzada, sobre hardware que no lo tiene de sobra), el tiempo baja a menos de la mitad. Y pedirle más hilos todavía de los que hay núcleos reales no ayuda nada más (73 → 74 ms): confirma que el límite es físico, dos núcleos dan como mucho el doble de rendimiento, no que haga falta reservar exactamente uno completo.

Importante: **esto no es una prueba de que gama media/alta vaya bien** — en esta máquina solo hay dos núcleos de verdad, así que "forzar 7 hilos" mide qué tan bien tolera el motor la sobre-suscripción, no el rendimiento real de un equipo de ocho núcleos. Para eso hace falta correr `cargo run --release -p bdj_search_core --example bench_corpus -- 10000000` en una máquina real de gama media y alta — el comando ya funciona tal cual, solo falta el hardware. Lo que sí puedo afirmar con la medición de aquí: en gama media (4 núcleos reales → 3 hilos de búsqueda) y alta (8+ núcleos reales → 7+ hilos), la resta de "un núcleo libre" dejará de sentirse, porque quedan de sobra para que la búsqueda vaya en paralelo de verdad sin arrebatarle nada a la interfaz. El problema medido aquí es específico y está acotado a equipos de dos núcleos.

## Por qué esto no se nota tanto como sugiere la tabla (y por qué a veces sí)

Miré también cómo se dispara la búsqueda desde la interfaz, no solo el motor en aislamiento, y hay dos mecanismos que amortiguan bastante el problema:

Hay una pausa de 120 ms (`search_provider.dart`, `_debounce`) antes de siquiera lanzar la consulta al motor tras cada tecla. Si escribes "m-i-c-h-a-e-l" del tirón, la mayoría de esas letras nunca llegan a generar una búsqueda propia — solo la última, cuando dejas de teclear. Eso significa que el peor caso medido arriba (una letra sola, fría) solo se siente de verdad en dos momentos: la primera letra que escribes al abrir el cuadro de búsqueda tras una pausa, y cualquier letra después de una pausa larga pensando qué escribir a continuación. No se siente en cada tecla de un tecleo fluido.

Cada búsqueda revisa si sigue siendo la más reciente **por bloque**, no al final (`token.is_cancelled(gen_id)` dentro del recorrido paralelo, en `engine.rs`). Con el tamaño de bloque de gama baja (16.384 entradas), eso significa que una búsqueda vieja se aborta en bien menos de un milisegundo en cuanto llega una más nueva — no hay ningún riesgo de que un tecleo rápido acumule búsquedas en cola y el retraso se vaya sumando.

Dicho esto, el caso que sí se siente —la primera letra tras una pausa, sobre una biblioteca de varios millones de archivos, en un equipo de dos núcleos— es precisamente el más común en el uso real: es literalmente cómo empieza cada búsqueda. Con tu disco real (3,7 M) esa primera letra cuesta hoy entre 110 y 156 ms en gama baja. Cien milisegundos es, más o menos, el umbral a partir del cual una persona deja de percibir algo como "instantáneo" — así que aunque no sea un segundo entero, sí es la clase de retraso que se nota y que encaja con lo que describiste ("a veces su tiempo de respuesta no es rápido").

## Lo que no pude medir aquí

No hay Flutter instalado en este entorno, así que todo lo anterior es el motor en Rust puro, sin el viaje de ida y vuelta por el puente FFI ni el tiempo de pintado de la tabla en pantalla. Ese viaje añade algo (normalmente de un dígito de milisegundos para una llamada async simple con `flutter_rust_bridge`), pero no debería ser el factor dominante frente a los 100-300 ms medidos arriba — no tengo forma de confirmarlo con un número real desde aquí. Si quieres, la forma más directa de completar esta foto sería correr la app en modo *profile* (no *debug*) en tu propia máquina de gama baja y mirar el temporizador de Flutter (`DevTools` → *Performance*) durante esa primera letra; el motor ya te dice cuánto le toca a él, así que cualquier diferencia con lo que veas en pantalla sería el resto de la cadena (interfaz + puente).

Tampoco pude probar gama media ni alta con hardware real, por la razón obvia de que esta máquina no lo es — lo de arriba es la mejor aproximación posible sin ese hardware, no un sustituto de probarlo allí.
