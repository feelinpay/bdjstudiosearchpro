# Cambios sobre la auditoría del 31 de agosto de 2026

Este documento recorre lo que se ha hecho, en el orden en que aparece en el plan
de la auditoría, y dice qué queda pendiente y por qué.

---

## Lo que se midió

Todas las cifras están tomadas sobre un corpus sintético de **diez millones de
entradas**, en modo release, en una máquina de **dos núcleos**. Tu equipo tiene
más, así que los tiempos absolutos que verás serán bastante menores; lo que no
cambia con la máquina son las proporciones.

| Consulta | Antes | Ahora |
|---|---:|---:|
| `m` (8,87 M coincidencias) | 307 ms | ~245 ms |
| `ext:wav` (2 M) | 108 ms | 70 ms |
| `tipo:audio` (8 M) | 195 ms | 130 ms |
| `michael` (400 K) | 288 ms | 180 ms |
| `m` ordenado por tamaño | 476 ms | 235 ms |
| Teclear `mich` → `michael` | 58–79 ms por tecla | 22–45 ms |
| **`michael bil` → `michael billie`** | **~300 ms por tecla** | **3,5–8 ms** |

La última fila es la que más importa: escribir artista y título es el gesto real
de un DJ, y era exactamente el caso que no funcionaba.

**Lo que no se ha alcanzado:** el presupuesto de 30 ms para la primera pulsación
sobre diez millones de archivos. Una letra suelta que coincide con el 89 % del
índice cuesta ~245 ms en dos núcleos. El recorrido está repartido y vectorizado;
lo que queda es ancho de banda de memoria. Con ocho núcleos deberían ser unos
60 ms. Prefiero decírtelo que fingir que la cifra está conseguida.

---

## Fase 1 · Emisión temprana de resultados

`Engine::search_with_limit` devuelve el **conteo exacto** de coincidencias más
las mejores `N` filas ya ordenadas. Cada bloque del recorrido paralelo se recorta
a sus mejores `N` antes de fundirse con los demás, así que el coste de ordenar
depende de la ventana pedida y no del número de coincidencias.

Antes se recolectaban todos los identificadores en un vector y se ordenaba el
conjunto entero: para pintar treinta filas se materializaban y ordenaban nueve
millones de enteros.

La interfaz pide 2000 filas de entrada y amplía la ventana solo si el usuario
baja de verdad (`extend_results`).

## Fase 2 · Orden proporcional al resultado

- Columna `NameRank` en el índice: la posición alfabética de cada entrada.
  Ordenar por nombre pasa a ser ordenar por una clave entera de cuatro bytes.
- `RadixSort` es ahora un radix de verdad —cuatro u ocho pasadas de conteo— y
  **materializa las claves una sola vez**, recorriendo la columna en orden
  creciente. Consultar `keys[id]` dentro del comparador eran accesos dispersos a
  un vector de decenas de megas: un fallo de caché por comparación.

## Fase 3 · Correcciones de corrección

| Hallazgo | Qué era | Qué se hizo |
|---|---|---|
| D-7 | El truncado a 255 bytes partía secuencias UTF-8 y el archivo desaparecía de toda búsqueda sin aviso | El corte retrocede a la frontera de carácter. Tres pruebas nuevas |
| D-8 | El refinamiento exigía que los términos viejos apareciesen idénticos | `QueryEvaluator::implied_by`: implicación real, con rangos y funciones |
| D-9 | `ruta:` reconstruía una ruta —dos reservas— por entrada evaluada | `resolve_full_path_into` sobre buffers reutilizados, y el filtro va el último dentro de un Y lógico |
| D-12 | Una consulta por pulsación, sin comprobar si el resultado seguía siendo el pedido | Retardo de 120 ms y número de secuencia: una respuesta atrasada ya no pisa a una reciente |
| Fuga | Se construía un `FocusNode` nuevo dentro de `build` | Es un campo, y se libera |
| Filtro | «Ejecutables» del menú no producía ninguna consulta: mostraba todo | Mapea a `tipo:apps` |
| Errores | Cualquier fallo se mostraba como «Cargando motor…» | Diagnóstico con causa concreta (ver más abajo) |

Además: `SubstringMatcher` ya no recalcula `to_lowercase()` del patrón en cada
entrada, y el barrido de nombres se hace **en bloque sobre la arena** con
`memchr2_iter` en lugar de una llamada por archivo.

## Fase 4 · Ventana de filas acotada

`RowCache` guarda las filas por páginas de 200 y expulsa la menos usada al pasar
de diez. En memoria hay como mucho dos mil filas, tenga el resultado tres o nueve
millones. `itemCount` es ahora el total real —acotado a 200 000 filas
direccionables— en lugar de las filas cargadas: la barra de desplazamiento deja
de mentir sobre su tamaño.

## Fase 5 · Vigilancia de exFAT, FAT32 y USB

`rdcw.rs` estaba escrito, era correcto y **no se instanciaba en ningún sitio**.
Ahora cada volumen sin diario USN tiene su propio hilo de vigilancia —
`ReadDirectoryChangesW` es una llamada bloqueante y no puede vivir en el bucle de
sondeo— que deja lo que ve en una cola.

`poll_volume_changes` ya no filtra por NTFS: un pendrive exFAT conectado en
caliente se recorre, entra en el índice y queda vigilado.

## Fase 6 · Órdenes de control reales

El protocolo de la tubería tiene ahora semántica: excluir un volumen por su
prefijo de montaje, añadir y quitar carpetas, reescanear, comunicar el estado de
la licencia y anunciar cambios locales. Los ajustes se guardan en
`settings.json`, junto al índice, de forma atómica.

Los volúmenes se identifican por **prefijo de montaje** y no por su
identificador numérico interno, porque ese número se reasigna en cada
compactación: guardar «el volumen 3 está excluido» acabaría excluyendo otro
disco.

Un salto del diario USN —es circular, y una copia grande puede darle la vuelta—
ya no se limita a escribir un aviso en el registro: marca el volumen para
reindexar.

## Fases 7 y 8 · Selección y arrastre

`Selection` es un modelo puro, sin dependencias de Flutter ni del motor. Tiene
dos modos para que «seleccionar todo» sobre nueve millones de resultados no
construya nueve millones de enteros: en modo invertido, el conjunto guarda lo
**excluido**.

Soporta Ctrl/Cmd + clic, Mayús + clic con ancla estable, seleccionar todo,
invertir y deseleccionar. Y **todas** las acciones operan sobre el conjunto: antes
se pintaban diez archivos marcados y la acción se aplicaba a uno.

El arrastre entrega al sistema la lista completa de referencias de lo
seleccionado. No copia, no mueve y no crea ningún temporal.

## Fases 9 y 10 · Explorador

El índice guarda ahora la lista de hijos de cada carpeta en formato comprimido
por filas. Abrir una carpeta es leer un rango contiguo de memoria ya mapeada.

En la interfaz: barra con atrás, adelante, subir y miga de pan pulsable, con
historial. Buscar y navegar comparten la misma tabla, el mismo modelo de
selección y el mismo arrastre; lo único que cambia es de dónde salen las filas.

**Pendiente de la fase 10:** el árbol lateral y los favoritos. La navegación
funciona por miga de pan y por doble clic, pero no hay panel lateral todavía.

## Fase 11 · Gestor de operaciones de archivo

Crate nuevo, `bdj_search_fsops`, que no sabe nada del índice ni de la interfaz.
25 pruebas.

- Cola con un hilo por operación, progreso por bytes y por elementos,
  cancelación real entre bloques de un megabyte.
- Conflictos resueltos **dentro de la aplicación**: el hilo se detiene, publica
  el choque con tamaños y fechas de ambos lados, y espera. Reemplazar, omitir,
  mantener ambos, aplicar a todos.
- Copiar, mover, renombrar, duplicar, crear carpeta y enviar a la papelera.
- **Nada se borra de forma definitiva**, ni siquiera con Mayús + Suprimir. El
  original de un movimiento entre volúmenes también va a la papelera.
- Deshacer invierte **acciones concretas** anotadas en un recibo, no la
  intención. Una operación que sobrescribió algo se marca como no reversible,
  porque lo pisado no vuelve.

## Fase 12 · Conciliación con el vigilante

`SuppressionWindow` es una pieza portable con siete pruebas. Al aplicar un cambio
anunciado por la aplicación se abre una ventana de tres segundos sobre esa ruta,
y todo lo que llegue del sistema de archivos para la misma ruta dentro de esa
ventana se descarta.

No se cierra al primer acierto: una sola copia genera varios registros
—creación, extensión de datos, cierre— y cerrarla tras el primero dejaría pasar
los siguientes.

La actualización optimista existe: `resolve_id_by_path` traduce una ruta a su
entrada bajando por la columna de hijos, y el cambio entra en la capa al
instante. Si la ruta no se puede situar, se devuelve `false` y se deja que el
vigilante lo resuelva por su cuenta.

También se corrigió un fallo que apareció al escribir estas pruebas: una entrada
creada en la capa y borrada antes de compactar **se quedaba en el índice para
siempre**, porque `mark_deleted` ignoraba los identificadores de la propia capa.

---

## «Cargando motor…»

Era un error disfrazado de pantalla de carga. `_tryOpenEngine` ponía
`isIndexLoaded = false` ante cualquier excepción y reintentaba cada segundo, para
siempre; la barra solo sabía pintar «Índice listo» o «Cargando motor…».

Ahora `engine_status` devuelve la ruta que se está mirando, si el archivo existe,
su tamaño, y un código de problema: no existe, formato antiguo, dañado, sin
permiso, u otro. La barra de estado lo dice con sus palabras y la ayuda emergente
da la ruta exacta.

---

## Principios que se han seguido

- **Una responsabilidad por pieza.** El motor no conoce el sistema operativo.
  `bdj_search_fsops` no sabe nada del índice. `Selection` y `SuppressionWindow`
  son lógica pura con sus propias pruebas y sin dependencias de interfaz.
- **Abierto a extensión.** Añadir una columna de ordenación es añadir un caso en
  `sort_key`; añadir una operación de archivo es un caso en `worker::run`. Nada
  más cambia.
- **Dependencias hacia dentro.** La interfaz depende del motor; el motor no sabe
  que existe una interfaz. El cruce se hace con tipos simples —enteros, cadenas y
  vectores— y con códigos numéricos en lugar de enumerados, para que un cambio
  de nombre en el motor no rompa la interfaz en silencio.
- **Sustituible.** Toda la parte específica de Windows vive detrás de `#[cfg]`,
  y la lógica que se puede probar sin Windows se ha sacado a piezas portables
  precisamente para poder probarla.

---

## Lo que queda pendiente

**Necesita tu máquina, no se puede hacer desde aquí:**

1. `flutter_rust_bridge_codegen generate` — obligatorio, la superficie FFI cambió.
2. `flutter analyze` — no hay cadena de herramientas de Dart en este entorno.
   Mándame lo que salga y lo arreglo.
3. La primera compilación de todo lo marcado con `#[cfg(windows)]`. Es la parte
   que no he podido compilar: aquí no se puede construir para Windows. El código
   portable —que es la mayoría— está compilado y probado.

**Trabajo que falta de verdad:**

4. Árbol lateral de carpetas y favoritos (resto de la fase 10).
5. Vistas alternativas: iconos, compacta, lista. Ahora solo hay detalles.
6. macOS: verificación en una máquina real. Los componentes —FSEvents, canal de
   control por socket Unix, papelera con restauración— ya están escritos y
   compilan para ambos targets de Apple, pero «compila» no es «funciona»: hace
   falta un Mac para la primera ejecución real del vigilante, `flutter build
   macos`, compilar `bdj_search_ffi` (necesita cadena de herramientas de Apple)
   y la notarización.
7. Restaurar desde la papelera en macOS en máquina real. La papelera propia
   (`trash_path` + `find_trashed_item` con xattr) está implementada y compilada;
   probar la restauración de elementos que el propio Finder tiró es cosa de un
   Mac.

**Decisiones que siguen esperando tu respuesta** (de la auditoría):

- El acoplamiento exacto de versión en SPP3: publicar 1.0.1 invalida las
  licencias de 1.0.0.
- Los permisos del archivo de índice.
- Volúmenes desconectados: atenuados o escondidos.
- Mover el catálogo de productos a `BdjProduct` dentro de `bdj_license_core`.

---

## Pasada de producción · 1 de septiembre de 2026

Cierre de los diez puntos priorizados de la auditoría y dos añadidos:

- **Redo** (`Ctrl+Shift+Z`): la historia guarda la petición original de cada
  operación; deshacer la apila para poder rehacerla, y una operación nueva la
  invalida. FFI `fsop_can_redo`/`fsop_redo_last`.
- **Límite de concurrencia en fsops**: semáforo (RAII) con `MAX_CONCURRENT_FSOPS
  = 3`. Las operaciones sobrantes esperan en `Planning`; la interfaz las ve como
  "contando", no congeladas.
- **Vistas alternativas**: detalles, lista, compacta y **iconos (grid)**. La
  cuadrícula es virtualizada y comparte selección, doble clic, menú contextual,
  arrastre al sistema y sueltas sobre carpetas con las demás vistas.
- **Eliminar permanentemente**: `OpKind::DeletePermanently` (irreversible, fuera
  de la historia de deshacer) + `fsop_delete_permanently`. La UI exige escribir
  «ELIMINAR» en un diálogo. La raíz de un volumen nunca se borra.
- **Limpieza**: retirados `transactions.dart`, el legacy `file_ops`/`file_ops_manager`,
  los adaptadores FFI muertos y `url_launcher`/`cupertino_icons`. Una
  regeneración de bindings dejó la FFI y el motor en un estado que compila de
  verdad con `--all-targets`. El catch silencioso de `serviceStatus` en el
  lateral ahora registra con `debugPrint`.

---

## Paridad macOS · 1 de septiembre de 2026

Windows ha sido la plataforma completa; macOS compilaba pero sin cuatro piezas
clave. Ahora están escritas y compiladas para `x86_64-apple-darwin` y
`aarch64-apple-darwin` (verificado con `cargo check`). La séptima fase del
documento dejaba el alcance explícito: «En macos es el mismo alcance que
windows».

- **Vigilancia en tiempo real.** `FSEventStreamCreate` de verdad
  (`bdj_search_fs/src/macos/fsevents.rs`, `FsEventWatcher`): un hilo con run
  loop propio por ruta, latencia 0,35 s, flags `FileEvents | NoDefer |
  UseCFTypes | WatchRoot`, eventos por un canal crossbeam con tope de 2048, y
  reenganche por el último id de evento. El stream y el array de rutas se
  mantienen vivos hasta el `Drop`, y el apagado despierta el run loop.
- **El servicio `bdj_search_indexer` hace lo mismo que en Windows.** El escaneo
  inicial registra un vigilante por volumen (`/`, `/System/Volumes/Data`,
  `/Volumes/*`), un bucle sondea los eventos y un `must_rescan_subdirs` dispara
  el reescaneo puntual; un volumen conectado en caliente se indexa al momento
  (`poll_volume_changes_mac`). La exclusión y el estado se comparten con el
  resto del servicio. El canal de control es un **socket Unix**
  (`/var/run/bdj_search_pro.sock`) en lugar de la tubería nombrada; las órdenes
  del protocolo no cambian.
- **Papelera con restauración.** La crate `trash` en macOS solo sabe vaciar, no
  restaurar. Ahora la papelera es propia (`bdj_search_fsops/src/macos.rs`): el
  elemento se mueve a `~/.Trash` o `.Trashes/<uid>` del volumen del que venía —
  mover instantáneo en el mismo volumen— con nombre libre estilo Finder
  («foo 2.ext») y se graba la ruta original en el xattr
  `com.bdjstudio.original-path`. Restaurar (`find_trashed_item`) localiza por
  esa marca —o por nombre, eligiendo lo más reciente, para lo que el Finder
  tiró directamente— y devuelve el elemento recreando la carpeta original. El
  cruce entre volúmenes (papelera del volumen ausente) cae a copiar + retirar.
- **El frontal Flutter ya busca el binario** del indexador en las rutas
  normales de macOS (`.../build/.../bdj_search_indexer`,
  `../engine/target/{release,debug}/bdj_search_indexer`) además de las `.exe`.

### Permisos, almacenamiento y rutas · 1 de septiembre de 2026

Revisado contra **BDJ Studio Sample Pad**, que ya está en producción para
Windows y macOS, y sus tres decisiones clave se han aplicado a Search Pro:

- **Sandbox fuera.** Sample Pad publica con `com.apple.security.app-sandbox =
  false` + `files.user-selected.read-write` + `network.client`; con sandbox
  activo una app no puede leer todos los volúmenes ni lanzar el indexador.
  Antes Search Pro llevaba `app-sandbox = true`, lo que hacía imposible
  funcionar. Los dos entitlements (`Release` y `DebugProfile`) ahora calcan la
  configuración probada.
- **Índice por usuario, no de sistema.** La ruta principal en macOS pasa a
  `~/Library/Application Support/BDJ Studio/Search Pro` (la que Sample Pad ya
  usa y donde un agente sin privilegios sí puede escribir); `/Library/...`
  queda como secundario para instalaciones de sistema. Deben coincidir el
  servicio (`get_index_path`) y la FFI (`resolve_default_index_path`), y ambas
  se han cambiado juntas.
- **Socket Unix en la carpeta del usuario.** `/var/run/bdj_search_pro.sock`
  exigía administrador. Ahora `default_unix_socket_path()` lo coloca en
  `~/Library/Application Support/BDJ Studio/Search Pro/bdj_search_pro.sock`,
  y tanto el indexador como el cliente FFI usan la misma resolución.

El almacenamiento seguro ya estaba alineado con Sample Pad: macOS con
`usesDataProtectionKeychain: false` y `first_unlock`, y huella de equipo por
`systemGUID`.

**Lo que sigue pendiendo de una máquina Mac:** la primera ejecución real de
FSEvents sobre el sistema de archivos, `flutter build macos`, compilar
`bdj_search_ffi` (requiere la cadena de herramientas de Apple) y la
notarización. El código queda compilado y comprobado para ambos targets de
Apple.

---

## Explorador completo · 1 de septiembre de 2026

La navegación por carpetas y las acciones de archivo quedan al nivel de un
explorador del sistema, con el mismo código en Windows y macOS.

- **Barra de ruta editable.** El botón de lápiz junto a la miga de pan convierte
  la barra en un campo con la ruta exacta: Enter navega, Esc vuelve, y mientras
  se escribe se autocompletan **las carpetas que ya existen en disco** (con
  `dart:io`, sin tocar el índice), igual en Windows y macOS. Se entienden los
  separadores de ambos (`C:\…`, `/…`), se expande `~` a la carpeta personal y,
  si la ruta apunta a un archivo, se abre su carpeta. `openFolder` ahora
  devuelve si la ruta pudo abrirse (`explorer_bar.dart`, `search_provider.dart`).
- **Nuevo archivo.** `OpKind::CreateFile` + `fsop_create_file` (worker
  `create_file`, gas con el mismo flujo conflicto/undo que crear carpeta) y
  `createFile` en el provider. La UI numera el nombre por defecto como el
  Explorador («Nuevo documento de texto (2).txt»).
- **Menú contextual de fila ampliado:** Cortar, Copiar, Duplicar y Cambiar
  nombre (F2), junto a los ya existentes abrir / abrir con / revelar en
  explorador / copiar ruta y nombre / papelera / borrado definitivo /
  propiedades (`virtualized_table.dart`).
- **Menú del fondo de carpeta:** clic derecho sobre el hueco de la tabla o de
  la cuadrícula ofrece «Nueva carpeta», «Nuevo archivo», «Pegar» (si hay
  portapapeles) y selección, como el Explorador (`_menuDeFondo`).
- **Diálogos compartidos.** `pedirTexto`, `nuevaCarpetaDialog`,
  `nuevoArchivoDialog` y `renombrarDialog` viven en `file_ops_dialogs.dart` y
  los usan por igual el menú Edición y los contextuales (se eliminó la copia
  privada de `top_menu_bar.dart`). Etiqueta del tipo de operación «Creando
  archivo» en el overlay.

**Fuera de alcance en esta pasada:** una vista de papelera que liste y restaure
los borrados (el motor tiene `fsop_restore` y undo, pero no hay FFI para
enumerar el contenido de la papelera); «Abrir con…» en macOS sigue degradado a
«revelar en Finder» (no hay diálogo genérico de elección de aplicación por
consola); pegar fuera del modo carpeta.

---

## Explorador afilado · 2 de septiembre de 2026

Los informes de la primera sesión real con el explorador («Abrir con» no
funcionaba, las propiedades tampoco, copiar/pegar parecía muerto, duplicar no
refrescaba, renombrar desde el Explorer no se reflejaba y «Nueva carpeta»
crasheaba al cerrar) se desglosaron y atacaron por separado:

- **Quitados «Abrir con…» y «Propiedades»** del menú contextual de fila: no hay
  forma portátil de abrirlo de verdad en Windows ni macOS desde el motor, así
  que se eliminan en lugar de simularlos (`virtualized_table.dart`).
- **Ctrl+C ahora copia para PEGAR DENTRO de la app** (como el Explorador). El
  manejador global lo dedicaba a copiar la **ruta** como texto al portapapeles
  del sistema, mientras el menú «Copiar» esperaba el portapapeles interno:
  Copiar + Pegar nunca llegaban a pegar nada. `Ctrl+C` colecciona las rutas
  seleccionadas, `Ctrl+Shift+C` sigue copiando la ruta exacta y el nombre queda
  solo en el menú contextual (`main.dart`, `top_menu_bar.dart`).
- **Crash arreglado al crear carpeta/archivo o renombrar.** El diálogo de nombre
  creaba el `TextEditingController` fuera y llamaba a `dispose()` en cuanto
  `showDialog` devolvía, pero la animación de cierre sigue viva unos fotogramas
  y el campo se volvía a construir con el controller suelto →
  *«A TextEditingController was used after being disposed»*. Ahora un
  `StatefulWidget` **posee** el controller (nace en `initState`, muere en
  `dispose`) en `pedirTexto` y `eliminarPermanentemente`
  (`file_ops_dialogs.dart`).
- **Duplicar/renombrar con refresco inmediato.** Tras una operación la vista se
  repetía antes de que el indexador reescribiera el índice (0,5–2 s de calma),
  así que preguntaba «¿cambió?» cuando todavía no había cambiado. Ahora
  `refreshAfterFileOperation` **reintenta `reloadIfChanged` hasta 4 s** antes de
  repetir la consulta: la fila nueva (o el nombre nuevo) aparece en cuanto el
  índice publica (`search_provider.dart`). Y `file_ops_provider._poll` no se
  apila (guard) si el refresco tarda más que su propio tick.
- **Un solo indexador, siempre el recién compilado.** La app lanzaba
  `--standalone` *detached* a ciegas: los zombies de sesiones anteriores
  bloqueaban la sobrescritura del `.exe` («El proceso no puede obtener
  acceso… en uso») y —peor— dos indexadores competían por el mismo pipe y el
  mismo `index.bdjx`, dejando la **generación congelada** que la app
  consumía: los renombrados hechos desde el Explorador de Windows nunca se
  reflejaban y la búsqueda seguía viendo los nombres viejos. Ahora:
  - `frontend/lib/core/indexador.dart`: al arrancar se mata cualquier indexador
    que viva **dentro del árbol de build de la app** (por ruta, no global: un
    servicio instalado del sistema se respeta) y se lanza el binario actual en
    exclusiva; al salir (**Salir** del menú o bandeja) se detiene el de la
    sesión para que el `.exe` quede libre y no queden daemons huérfanos.
  - `tools/build_native.ps1` y `tools/build_macos.sh`: antes de copiar matan
    los indexadores que corran desde su árbol de build, así la copia nunca
    aborta por un binario en uso.
- **Búsqueda de carpetas.** El motor ya indexa y devuelve carpetas como
  resultados (flag `FLAG_DIR`, mismas pruebas que los archivos); la sensación
  de «solo busca archivos» era consecuencia del índice congelado descrito
  arriba. La UI ya pinta la carpeta con su icono y al abrirla navega dentro,
  en Windows y macOS sin cambios por plataforma. El filtro «Carpetas»
  (`tipo:carpetas`) sigue disponible como filtro avanzado.
- **Actualizar manual.** `F5` (y menú `Archivo > Actualizar` + «Actualizar» en
  el menú contextual) vuelve a consultar el índice y repite la vista actual:
  un botón de respiro cuando el último cambio lo haya hecho otro programa.

**Pendiente:** seguir requiriendo una máquina Mac real para la primera
ejecución de FSEvents y `flutter build macos`; la lógica de indexador único
queda activa también ahí (`pkill -f` por árbol de build).

---

## Tiempo real «empujado» y explorador con ruta pegable · 2 de septiembre de 2026

- **La UI ya no sondea con un temporizador: espera a que el motor le avise.**
  Antes un `Timer.periodic` de 400 ms preguntaba `reloadIfChanged`; ahora el
  motor expone `wait_index_changed(timeout_ms)` (`api.rs`), que bloquea en el
  hilo nativo, se despierta en ~30 ms cuando el servicio reescribe el índice
  (rename atómico) y devuelve `true`. `SearchNotifier` se queda en una espera
  encadenada: la vista se refresca *empujada* por la publicación, como un
  socket, con reintento automático al agotarse el plazo (`search_provider.dart`,
  `_reloadTimeout` de 3 s). El tiempo real ya no depende de la cadencia de un
  `Timer` de Dart.
- **Pegar la ruta completa, en cualquier sitio donde se escriba.** El manejador
  global de teclado secuestraba `Ctrl+V`/`Ctrl+C`/flechas/`Supr`/`Ctrl+A`
  incluso con el foco dentro de un campo: pegar una ruta en la barra («lápiz»)
  hacía «pegar archivos», y copiar texto en el buscador copiaba la selección de
  la tabla. Ahora, si el foco está en un campo editable, los atajos de la tabla
  se ceden al campo (`_escribeEnCampoDeTexto` en `main.dart`). Solo quedan
  reservados `Esc`, `F5` y `Ctrl+F`.
- **El buscador acepta una ruta pegada con Enter.** Al pulsar Enter con un
  texto que parece una ruta (pej. `C:\Users\...` o `~/Música`), la app navega a
  ella como la barra de direcciones del Explorador; sobre un archivo, a su
  carpeta (`_abrirComoRutaSiLoEs` en `main.dart`).
- **Causa raíz del «refresco manual que tampoco actualiza», ya cerrada.** El
  indexador recién lanzado solo vigila y republica si ha recibido la licencia,
  y el aviso se mandaba **una vez** — cuando el daemon aún hacía su escaneo
  inicial y no tenía el pipe, el envío fallaba en silencio y la generación
  quedaba congelada para siempre: ni cambios externos ni `F5`. `_informar
  LicenciaAlServicio` ahora **reintenta hasta 600 veces** hasta que el servicio
  confirma (`main.dart`). Con la licencia dentro, el ciclo completo es
  detección (≤500 ms) + calma (0,25 s) + republish + aviso ~30 ms: se siente
  como el Explorador.
- Internamente, el daemon rebajó su calma de publicación para índices normales
  (≤2 millones de entradas) a 250 ms (`main.rs`).

**Pendiente:** mismo recorrido en macOS (FSEvents) y probar en una máquina Mac
real; la espera `wait_index_changed` es independiente de la plataforma.
