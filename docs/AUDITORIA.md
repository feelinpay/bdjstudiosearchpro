# BDJ Studio Search Pro — Auditoría técnica (estado actual)

**Última revisión:** 31 de agosto de 2026
**Alcance:** todo el proyecto tal como está en disco en esta fecha. Esta versión sustituye a la anterior: refleja que ya existen el gestor de operaciones de archivo, el explorador, la navegación y los marcadores.
**Método:** lectura completa de los crates de Rust y de la interfaz Flutter, verificación cruzada con `file:line`.

> **Cambio de alcance del producto confirmado por el propietario:** **los marcadores SÍ se implementan** (y ya están funcionales); **no se implementa** el menú de **Herramientas** ni nada relativo a **Ayuda**.

---

# A. Arquitectura actual

## A.1 Forma general

```
┌──────────────── Flutter (sin privilegios) ────────────────┐
│ main.dart → LicenseGate → SearchHomeScreen                │
│   TopMenuBar (Archivo/Edición/Búsqueda/Marcadores)         │
│   SidebarTree (MIS MARCADORES · EQUIPO Y VOLÚMENES)        │
│   ExplorerBar (atrás/adelante/subir · miga de pan)         │
│   VirtualizedTable (selección múltiple · arrastre saliente)│
│   FileOpsOverlay (progreso · conflictos · cancelar)        │
│                        │                                   │
│              flutter_rust_bridge 2.13  (~50 funciones)     │
│  bdj_search_ffi ───────┘                                   │
│     mmap SOLO LECTURA de index.bdjx + fsops (escritura)    │
└───────────────────────┬────────────────────────────────────┘
                        │            ▲ tubería nombrada (control)
                        ▼            │
                index.bdjx  ◄────────┤
                                     │
┌─────── bdj_search_indexer (servicio) ─────────────────────┐
│  initial_scan: MFT (NTFS) / recorrido (exFAT, macOS)       │
│  USN 500 ms + RDCW (exFAT/FAT32) · FSEvents (macOS)        │
│  poll_volume_changes (~2 s) → alta/baja de volúmenes       │
│  control: tubería nombrada (Windows) · socket Unix (macOS) │
│  compact() al calmarse → republica el índice               │
│  gate de licencia: no indexa sin licencia                  │
└────────────────────────────────────────────────────────────┘
```

## A.2 Los crates de Rust

| Crate | Responsabilidad | Estado |
|---|---|---|
| `bdj_search_core` | Índice columnar, formato, parser, motor, ordenación, **fsops (legacy)** | Sólido; el motor ya emite por top-N |
| `bdj_search_fs` | Backends de sistema de archivos (Windows USN / recorrido / RDCW, macOS) | Windows completo; RDCW **conectado**; macOS **FSEvents real** |
| `bdj_search_fsops` | **Gestor real de operaciones de archivo** (cola + hilos + progreso + conflictos + undo) | Nuevo, es el que usa la FFI |
| `bdj_search_ipc` | Protocolo de control (postcard) + tubería nombrada | Funcional |
| `bdj_search_indexer` | Servicio: escaneo, vigilancia, compactación | El más grande y cargado |
| `bdj_search_ffi` | Superficie expuesta a Dart | Lectura + navegación + escritura + control |

La separación `core` (sin sistema operativo, testeable) se mantiene y es la mejor decisión del proyecto.

## A.3 El índice

Formato `BDJXIDX\0`, **versión 2**. La versión 1 se rechaza y el servicio la reconstruye. Secciones nuevas respecto a la v1: **`ChildOff` + `ChildIdx`** (CSR: hijos por carpeta) y **`NameRank`** (rango alfabético inverso de `name_order`). Total: 17 secciones.

Medido: **64,3 bytes/entrada** (~612 MB para 10 M). La ruta completa no se almacena: se reconstruye por la cadena `parent` (tope de profundidad 128 + corte por ciclo).

## A.4 La interfaz

Riverpod 2 con `StateNotifierProvider`. Providers: `searchProvider`, `fileOpsProvider`, `favoritesProvider` y los de licencia. El explorador se compone de barra de menús, panel lateral (`SidebarTree`, siempre visible), `ExplorerBar`, `VirtualizedTable` y `FileOpsOverlay`.

---

# B. Lo que ya funciona (verificado hoy)

## B.1 Motor y búsqueda

| Funcionalidad | Dónde | Notas |
|---|---|---|
| **Emisión temprana por top-N** | `search/engine.rs:186-303` | Ya **no** materializa ni ordena el conjunto completo. Escanea por trozos, cada trozo conserva sus `limit` mejores (`select_nth_unstable`), fusiona y recorta. `total_count` exacto; bandera `truncated`. `DEFAULT_LIMIT = 2_000` |
| **Orden por nombre en O(resultados)** | `engine.rs:469,490-496` + columna `NameRank` | Ya no recorre el índice entero por tecla; respaldo a comparación de cadenas si falta la columna |
| `name_rank` / `name_order` | `index/` | Son inversas |
| **Refinamiento incremental** | `search/refine.rs` | Caché de supersets hasta 2 M por consulta / 4 M total, profundidad 16, con `implied_by` |
| **Barrido SIMD por nombre** | `search/scan.rs` | `memchr2_iter` sobre la arena de nombres con cursor monótono |
| Parser de consultas | `query/parser.rs` | AND, `\|`, `!`, paréntesis, comillas, `ext:`, `tam:`, `mod:`, `dc:`, `ruta:`, `padre:`, `pid:`, `tipo:`, `regex:`, `case:`, `archivo:`, `carpeta:` |
| Consulta compilada | `query/compiled.rs` | Se compila una vez por consulta |
| Índice de hijos por carpeta | `ChildOff`/`ChildIdx` | Base del explorador; usado por `browse_path` |
| Reconstrucción / miga de rutas | `view.rs` | Correcta, con cortes de seguridad |
| Recarga en caliente | `api.rs:reload_if_changed` | Remapea si cambia la generación |
| Orden por tamaño / mtime / ctime | `sort/radix.rs`, `sort/perm.rs` | |

## B.2 Gestión de archivos (nuevo, via `bdj_search_fsops`)

| Operación | Estado | Dónde |
|---|---|---|
| Crear carpeta | **SÍ** | `fsops/worker.rs:106-145` |
| Renombrar (incl. cambio solo de mayúsculas) | **SÍ** | `worker.rs:147-197` |
| Copiar | **SÍ** (chunk 1 MiB, conserva mtime) | `worker.rs:423-467` |
| Mover (mismo volumen = `rename` instantáneo; otro volumen = copiar+papelera) | **SÍ** | `worker.rs:332,365-383` |
| Duplicar | **SÍ** | `worker.rs:263-278` |
| Enviar a la papelera | **SÍ** (`trash` crate) | `worker.rs:199-222` |
| **Papelera + restauración en macOS** | **SÍ** | `fsops/src/macos.rs` — `trash_path`/`find_trashed_item` con xattr `com.bdjstudio.original-path` |
| **Eliminar permanentemente** | **NO** (por diseño: todo va a la papelera) | `lib.rs:10-12` |
| **Restaurar desde papelera** | **NO** (stub; `Action::Trashed` no reversible) | `file_ops.rs:69-72`, `worker.rs:565-568` |
| Cola de operaciones | **SÍ** (un hilo por operación, `bdj-fsop-<id>`) | `lib.rs:68-88` |
| Progreso (ítems y bytes) | **SÍ** (`AtomicU64`, sondeo) | `state.rs:116-129` |
| Cancelación cooperativa | **SÍ** | `worker.rs:433`, `state.rs:175-187` |
| Conflictos (`Ask/KeepBoth/Skip/Overwrite` + «aplicar a todos») | **SÍ** | `state.rs:201-249`, `worker.rs:491-517` |
| **Undo** (último y por historia) | **SÍ** (solo undo, máximo 64) | `lib.rs:34,142-170`, `worker.rs:521-573` |
| **Redo** | **NO** | — |
| Validación de nombres (reservados, caracteres, longitud, `.`/`..`) | **SÍ** | `naming.rs:44-91` |
| Protección «mover carpeta dentro de sí misma» | **SÍ** | `naming.rs:85-91` |
| Integración con el índice (PathChange → IPC → Overlay) | **SÍ** | `api.rs:902-918` |

## B.3 Vigilancia del sistema de archivos

| Capacidad | Estado | Dónde |
|---|---|---|
| Diario USN incremental | **SÍ** | `usn.rs` |
| **`ReadDirectoryChangesW` conectado** (exFAT/FAT32 y volúmenes no USN) | **SÍ** (antes escrito y muerto) | `main.rs:237-303`, `fs/windows/rdcw.rs` |
| **USB exFAT/FAT32 en caliente indexado** | **SÍ** | `main.rs:689-848` (`index_new_volume`) |
| Reindexado tras pérdida/corrupción del diario | **SÍ** (`request_rescan`) | `main.rs:643-657` |
| Alta/baja de volúmenes | **SÍ** |
| **FSEvents real** (macOS) | **SÍ** | `bdj_search_fs/src/macos/fsevents.rs` — stream por volumen con run loop propio y reenganche por id de evento |
| **Control por socket Unix** (macOS/Linux) | **SÍ** | `bdj_search_ipc::unix_socket` + hilo IPC en el indexador |

## B.4 FFI (superficie a Dart)

~50 funciones públicas: lectura (`search`, `browse_path`, `browse_roots`, `breadcrumb`, `rows`, `full_path`, `paths_for_rows`, `parent_path`, `get_row_by_id`…), escritura (`fsop_create_folder`, `fsop_rename`, `fsop_copy`, `fsop_move`, `fsop_duplicate`, `fsop_trash`, `fsop_resolve_conflict`, `fsop_cancel`, `fsop_undo_last`, `fsop_can_undo`…), control (`service_set_license`, `service_set_volume_indexed`, `service_add_folder`, `service_rescan`, `service_set_indexing`, `service_status`).

## B.5 Interfaz (verificado en Dart)

| Funcionalidad | Estado | Dónde |
|---|---|---|
| **Copiar / cortar / pegar / mover** | **SÍ** | `file_ops_provider.dart:144-226` |
| **Eliminar a papelera** | **SÍ** (solo papelera) | `file_ops_provider.dart:209-215` |
| **Renombrar / crear carpeta / duplicar** | **SÍ** | `file_ops_provider.dart:188-206` |
| **Cancelar operación** | **SÍ** | `file_ops_provider.dart:228-230` + botón en overlay |
| **Undo (Ctrl+Z)** | **SÍ** | `file_ops_provider.dart:245-248`, `top_menu_bar.dart:100` |
| **Redo (Ctrl+Shift+Z)** | **NO** | — |
| **Resolución de conflictos (UI)** | **SÍ** | `file_ops_overlay.dart:72-157` |
| **Progreso en overlay** | **SÍ** (sondeo 200 ms) | `file_ops_overlay.dart:13-68` |
| **Confirmación de papelera** | Menú **sí**; teclado (Supr) **no** | `top_menu_bar.dart:353-381` vs `main.dart:471-474` |
| **Navegación atrás/adelante/subir** | **SÍ** | `explorer_bar.dart:32-49`, `search_provider.dart:376-395` |
| **Migaja de pan** | **SÍ** | `explorer_bar.dart:84-139` |
| **Panel lateral visible** | **SÍ** (siempre) | `main.dart:611-614` |
| **Marcadores en el lateral** | **SÍ** | `sidebar_tree.dart:53-108` |
| **Volúmenes/unidades en el lateral** | **SÍ** (con estado conectado/desconectado) | `sidebar_tree.dart:112-172` |
| **Árbol de carpetas jerárquico** | **NO** (solo listas planas) | — |
| **Selección múltiple (Ctrl+click, Shift+rango, invertir, todo)** | **SÍ** | `selection.dart`, `search_provider.dart:491-527` |
| **Operaciones sobre la selección completa** | **SÍ** | `selectedPaths(max:5000)` etc. |
| **Arrastre saliente de la selección entera** | **SÍ** | `virtualized_table.dart:92-100,275-288` (Formats.fileUri) |
| **Arrastre interno (mover entre carpetas)** | **NO** | — |
| **Soltar carpeta externa para navegar** | **SÍ** | `main.dart:592-603` |
| **Añadir/Organizar Marcadores** | **SÍ** (funcional) | `top_menu_bar.dart:163-284` |
| **Menú Herramientas** | **NO existe** (eliminado, fuera de alcance) | — |
| **Menú Ayuda** | **NO existe** (fuera de alcance) | — |
| Robusteza FFI: `rethrow` en init | **SÍ** | `main.dart:87-93` |
| Fugas de `FocusNode` | **No** (corregidas) | `main.dart:289-316` |

## B.6 Seguridad / licencia / servicio

- **Gate de licencia en el servicio:** el bucle principal **no indexa si no hay licencia activa** (`settings.rs:100-108`, `main.rs:934-938`). Corregido el agujero de elevación.
- `SetVolumeIndexed`, `AddFolder`, `RescanVolume` **ya ejecutan resecans reales** (`main.rs:1340-1375`), no son meros logs.

---

# C. Lo que está incompleto o pendiente

Clasificación por tu taxonomía sobre el **estado actual**.

## C.1 Búsqueda

| Función | Estado |
|---|---|
| **Comodines** (`*.mp3`, `Kick*.wav`) | **NO IMPLEMENTADO** — el lexer no tiene `*`/`?`; solo substring y `regex:` |
| **Ordenar por extensión / tipo / ruta** | **NO IMPLEMENTADO** — `sort_col` solo 0 (nombre), 3 (tamaño), 4 (mtime), 5 (ctime) |
| Filtros por atributos (oculto, sistema, solo lectura) | **NO IMPLEMENTADO** — los bits están en el índice, no hay sintaxis |
| Búsqueda acotada a la carpeta actual | **PARCIAL** — se navega, pero la consulta no se acota por el ámbito |
| Historial de consultas / consultas guardadas | **NO IMPLEMENTADO** |
| Resultados en tiempo real mientras se indexa (ver overlay) | **NO VERIFICADO** — la búsqueda usa la vista base; el overlay lo aplica el servicio |
| Fechas con calendario exacto | **PARCIAL** — meses aproximados (`parser.rs:207-219`) |

## C.2 Explorador / navegación

| Función | Estado |
|---|---|
| **Árbol de carpetas jerárquico en el lateral** | **IMPLEMENTADO** — carga bajo demanda vía `browse_subdirs` |
| Vistas alternativas (lista / detalles / compacta / iconos) | **COMPLETO** — **detalles / lista / compacta / iconos (grid)** en el menú Ver |
| Doble clic para abrir | **NO VERIFICADO / faltante** — la apertura es por Enter/tecla |

## C.3 Gestión de archivos

| Función | Estado |
|---|---|
| **Restaurar desde la papelera** | **IMPLEMENTADO** (motor + FFI `fsop_restore` + UI) |
| **Eliminar permanentemente** | **IMPLEMENTADO** — `fsop_delete_permanently` (irreversible, fuera de deshacer) + diálogo que exige escribir «ELIMINAR» |
| **Redo (Ctrl+Shift+Z)** | **IMPLEMENTADO** — pila de rehacer en `fsops` (`fsop_redo_last`), atajo y menú |
| **Confirmación al teclear Supr** | **IMPLEMENTADO** — el atajo confirma igual que el menú |
| Undo de ciertos `Action::Overwrote` | **NO IMPLEMENTADO** (irreversible por diseño) |
| Concurrencia real en `fsops` | **ACOTADA** — semáforo (RAII) con `MAX_CONCURRENT_FSOPS = 3`; el resto espera en `Planning` |

## C.4 Arrastrar y soltar

| Función | Estado |
|---|---|
| **Arrastre interno (mover/copiar entre carpetas, también al panel lateral)** | **IMPLEMENTADO** — `DropRegion` en filas carpetas y en el árbol lateral (mover) |
| Destinos de soltado dentro de la tabla | **IMPLEMENTADO** |

## C.5 Otras

| Función | Estado |
|---|---|
| `transactions.dart` (modelos Rename/Move/CopyTransaction) | **ELIMINADO** (era código muerto) |
| Legacy `bdj_search_core::file_ops` + `file_ops_manager` | **ELIMINADO** — archivos huérfanos (no declarados) y adaptadores FFI legacy retirados |
| `restore_from_trash`, `delete_permanently` en FFI | **ELIMINADO** — quedaron fuera de alcance (al no existir `mod file_ops`, ni siquiera compilaban) |
| Dependencias sin usar en `pubspec.yaml` | **ELIMINADO `url_launcher` y `cupertino_icons`** — quedan las que se usan de verdad |
| Errores engullidos | 16× `catch (_) {}`; la mayoría aceptables; **`sidebar_tree.dart`** ya registra con `debugPrint` si `serviceStatus` falla |
| `ping`, `indexGeneration`, `fullPath` | Expuestas por FFI y poco/invariablemente usadas |

---

# D. Problemas técnicos (estado actual)

## CRÍTICO

- **Ninguno detectado activo en esta revisión.** Los dos críticos de la auditoría anterior (materializar+ordenar todo, orden proporcional al índice) **están resueltos** con la emisión top-N y la columna `NameRank`. Requiere confirmación con medición en el corpus de 10 M (la prueba `latency_budget_test.rs` existe).

## ALTO

- **D-A. Restaurar desde la papelera no existe en ningún punto del stack.** → **RESUELTO** (`Restore` nativo + undo).
- **D-B. Sin confirmación al borrar con `Supr`.** → **RESUELTO** (el atajo confirma igual que el menú).
- **D-C. Sin límite de operaciones simultáneas en `fsops`.** → **RESUELTO** (semáforo, máx. 3).

## MEDIO

- **D-D. Arrastre interno inexistente.** → **RESUELTO** (tabla + lateral, mover).
- **D-E. Árbol de carpetas jerárquico ausente** del panel lateral. → **RESUELTO**.
- **D-F. Código muerto**: `transactions.dart`, legacy `file_ops`/`file_ops_manager`, `restore/delete` expuestos sin uso, dependencias `cupertino_icons`/`url_launcher`. → **RESUELTO** (eliminado y regenerado).
- **D-G. `sidebar_tree.dart:33`** falla en silencio al cargar `serviceStatus`. → **RESUELTO** (registra con `debugPrint`).

## BAJO

- **D-H.** Sin `*.mp3` comodín (solo substring + `regex:`). → **RESUELTO** (comodines `*`/`?`).
- **D-I.** Fechas de calendario aproximadas para `thismonth`/`thisyear`. → **PENDIENTE**.
- **D-J.** `delete_permanently` existe en legacy pero no se expone por la UI. → **REPLANTEADO** e implementado de forma segura: `fsop_delete_permanently` (irreversible) con confirmación explícita en la UI. Protección: la raíz de un volumen nunca se borra.

---

# E. Funciones de Everything que faltan (por área)

## Búsqueda
- **Comodines** (`*.mp3`, `Kick*.wav`). **HECHO**.
- **Orden por extensión, tipo y ruta** (hoy filtros, no columnas). **HECHO** — columnas Tipo/Ext/Ruta ordenables.
- **Atributos** (oculto, sistema, solo lectura).
- Búsqueda acotada al ámbito de la carpeta actual.
- Historial y consultas guardadas.

## Explorador
- Árbol de carpetas jerárquico en el lateral. **HECHO**.
- Vistas alternativas (detalles/lista/compacta/iconos). **HECHO** (todas).

## Gestión de archivos
- **Restaurar desde la papelera.** **HECHO**.
- **Eliminar permanentemente** con confirmación explícita. **HECHO** (diálogo que exige escribir «ELIMINAR»).
- **Redo.** **HECHO**.

## Selección
- Ya completa (rango Mayús, Ctrl+clic, invertir, todo, operaciones sobre la selección). **OK.**

## Arrastrar y soltar
- **Arrastre interno** (mover entre carpetas, y hacia el panel lateral). **HECHO**.
- Destinos dentro de la tabla. **HECHO**.

## Teclado
- Ya están: `Ctrl+A/C/X/V/Z`, `Supr`, `F2`, `Inicio/Fin/AvPág/RePág`, `Backspace`. **Confirmación en `Supr` HECHA **y **`Redo` (Ctrl+Shift+Z) HECHO **.

## Indexación
- Selección de volúmenes/carpetas y exclusiones: **implementado** (controles reales). Persistencia de números de referencia para no reescanear al reiniciar: **no verificado/pendiente**.

---

# F. Funciones nuevas pendientes de añadir (priorizadas)

1. **Restaurar desde la papelera** (motor + FFI + UI). (Alta) — **HECHO**
2. **Confirmación al borrar con `Supr`** (coherencia de UX). (Alta) — **HECHO**
3. **Arrastre interno** para mover/copiar entre carpetas y al panel lateral. (Alta — P0 del producto) — **HECHO**
4. **Árbol de carpetas jerárquico** con carga bajo demanda en el lateral. (Media) — **HECHO**
5. **Comodines `*`/`?`** en el parser. (Media) — **HECHO**
6. **Orden por extensión/tipo/ruta.** (Media) — **HECHO**
7. **Redo** (Ctrl+Shift+Z). (Media) — **HECHO**
8. **Límite/throttling** de operaciones simultáneas en `fsops`. (Media) — **HECHO** (máx. 3 concurrentes)
9. **Vistas alternativas.** (Baja) — **HECHO** (detalles/lista/compacta + iconos en grid)
10. **Limpieza de código muerto** (`transactions.dart`, legacy `file_ops`, dependencias sin uso, FFI sin uso). (Baja — mantenibilidad) — **HECHO**

**Excluidos por decisión del propietario:** menú **Herramientas** y todo **Ayuda**.

---

# G. Arquitectura propuesta (para lo pendiente)

El principio se mantiene: **no tocar el núcleo que funciona** (formato, mmap, MFT, motor top-N). Todo lo nuevo se cuelga alrededor.

- **Restaurar papelera** → añadir `OP`/`Action` de `Restore` en `bdj_search_fsops` usando una API nativa (Windows `IFileOperation`/`SHFileOperation` con `FOF_ALLOWUNDO`, macOS `NSFileManager.trashItem(restore:)`), e integrarlo como reversible para el undo.
- **Arrastre interno** → en la UI, `DropZone` por fila-carpeta y sobre las entradas del `SidebarTree`, que al soltar llame a `fsop_move`/`fsop_copy` con las rutas seleccionadas. Se reutiliza la infraestructura de conflictos/progreso existente.
- **Árbol lateral** → cargar hijos bajo demanda vía `browse_path`/`get_row_by_id`/`get_parent_id` ya expuestos; no expandir todo de golpe (evitar miles de nodos).
- **Comodines** → ampliar el lexer del parser para emitir patrones glob y un `GlobMatcher` en `compiled.rs` (el resto del pipeline queda intacto).
- **Orden por ext/tipo/ruta** → nuevas columnas `sort_col` (5+), con `ext_id`/`volume`/ruta como claves; se reutiliza `RadixSort`/`PermutationSort`.
- **Redo** → extender `historia` para guardar la inversa y permitir reaplicarla.

---

# H. Flujo de datos (actual y objetivo)

```
UI (Flutter)
  │  SelectionModel / FileOpsNotifier / SearchNotifier
  ▼
FFI (bdj_search_ffi: lectura + fsop_*)
  ▼
 ┌────────────┐  ┌─────────────┐  ┌──────────────────┐
 │ Search/    │  │ IndexNaviga-│  │ FileOperation    │
 │ Explorer   │  │ tor (hijos, │  │  Manager (fsops) │
 │ engine     │  │ ruta, miga) │  │  cola + hilos +  │
 └─────┬──────┘  └──────┬──────┘  │  progreso + undo │
       │                │         └────────┬─────────┘
       ▼                ▼                  ▼
┌────────────────────────────────  Filesystem (native ops) ──┐
│ Memory Index (mmap v2 + overlay) ◄─ PathChange ─────────────│
└───────────────▲─────────────────────────────────────────────┘
                │ cambios del vigilante (USN / RDCW / FSEvents)
       Filesystem Monitor
```

Regla: la UI nunca toca el sistema de archivos directamente; el índice tiene una puerta de escritura (fsops + overlay) y se reconcilia con el vigilante vía `PathChange`/`LocalChange`.

---

# I. Rendimiento: cuellos de botella

**Resueltos desde la auditoría anterior:**
- Materialización + orden completo → top-N acotado.
- Orden por nombre proporcional al índice → `NameRank`.

**Medidos / a confirmar (pendiente):**
1. Presupuesto `< 30 ms` primera tecla y `< 8 ms` refinamiento sobre 10 M (hay `latency_budget_test.rs`; **no relanzado**).
2. **fsops** ~~sin límite de operaciones concurrentes~~: **ACOTADO** (máx. 3 concurrentes vía semáforo). Medir futuramente si el throttling de I/O conviene ajustar el número.
3. **Árbol lateral** cargado completo (riesgo de miles de nodos) → carga bajo demanda. **HECHO** (`browse_subdirs`).
4. **Ventana de filas**: la UI pide páginas (`rowAt`/`_loadPage`), no acumula (corregido respecto a `visibleRows`).

---

# J. Riesgos

| Riesgo | Gravedad | Mitigación |
|---|---|---|
| **Borrar con `Supr` sin confirmación** | Alta | **CERRADO** — el atajo confirma igual que el menú |
| **Papelera sin restaurar** | Alta | **CERRADO** — `Restore` nativo integrado al undo |
| **Corrupción del índice por escritura concurrente** | Media | Una puerta de escritura (fsops) + publicación atómica |
| **Doble evento del vigilante** | Media | Reconciliación vía `PathChange`/`LocalChange`; validar que no haya duplicados |
| **Deshacer que destruye datos** | Media | Solo operaciones reversibles entran en `historia`; `Overwrote` y `Deleted` excluidos |
| **Operaciones simultáneas sin límite** | Media | **CERRADO** — semáforo con máx. 3 concurrentes |
| **Borrado definitivo accidental** | Media | **MITIGADO** — irreversible + fuera de deshacer + diálogo que exige escribir «ELIMINAR»; la raíz de un volumen nunca se borra |
| **Presión de memoria con millones de resultados** | Media | Ventana de filas acotada (vigente) |
| **Nombres Unicode largos corrompidos** | Media | Truncado por frontera de carácter (verificar que sigue tal cual) |
| **Registro fantasma tras borrar** | Media | Actualización optimista vía receipt + overlay |

---

# K. Plan de implementación sugerido (fases pequeñas, sin tocar el núcleo)

**Bloque I — Cerrar UX de borrado (riesgo alto, poco código)** — **COMPLETADO**
1. Confirmación al enviar a la papelera con `Supr` (coherente con el menú).
2. Implementar **Restaurar desde la papelera** (nativo) y conectarlo al undo.

**Bloque II — Arrastre interno (P0)** — **COMPLETADO**
3. `DropZone` por carpeta en tabla y en `SidebarTree` → `fsop_move`/`fsop_copy`.

**Bloque III — Explorador** — **COMPLETADO**
4. Árbol de carpetas jerárquico con carga bajo demanda.
5. Vistas alternativas (lista/compacta). **ampliado** a las cuatro: detalles/lista/compacta/iconos (grid).

**Bloque IV — Búsqueda** — **COMPLETADO**
6. Comodines `*`/`?` en el parser.
7. Orden por extensión/tipo/ruta.
8. Redo.

**Bloque V — Robustez y limpieza** — **COMPLETADO**
9. Límite/throttle en `fsops`.
10. Eliminar código muerto (`transactions.dart`, legacy `file_ops`, dependencias y FFI sin uso).

> Los marcadores quedan **completos y funcionales** (no requieren acción). Herramientas y Ayuda **fuera de alcance** por decisión del propietario.

---

*Documento de diagnóstico. Estado reflejado a la fecha: los 10 pendientes priorizados están implementados y verificados, además de la vista de iconos (grid) y el borrado permanente con confirmación explícita. Desde el 1 de septiembre de 2026, **macOS queda al mismo alcance que Windows** a nivel de código (FSEvents, socket Unix, papelera con restauración), compilado y comprobado para `x86_64-apple-darwin` y `aarch64-apple-darwin`; la verificación en máquina real (ejecución, `flutter build macos`, notarización) sigue a cargo de un Mac. En permisos/rutas/almacenamiento se adoptó el modelo ya probado en producción por Sample Pad: sandbox desactivado, índice en `~/Library/Application Support/BDJ Studio/Search Pro` y socket Unix en la misma carpeta por usuario.*
