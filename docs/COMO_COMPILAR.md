# Cómo compilar y arrancar BDJ Studio Search Pro

Última actualización: 31 de agosto de 2026.

Este documento asume que partes del proyecto tal y como está ahora en disco, con
los cambios de las fases 1 a 12 ya aplicados.

---

## 0. Lo que tienes que saber antes de empezar

Tres cosas han cambiado y afectan al primer arranque:

**El formato del índice pasó a la versión 3.** El tamaño de archivo ocupa ahora
cuatro bytes en vez de ocho, con una tabla aparte para los que pasan de 4 GiB.
Sobre diez millones de archivos son cuarenta megas menos de memoria durante el
escaneo y cuarenta menos en el archivo; lo segundo también acelera cada
búsqueda, porque hay menos que recorrer. Un `index.bdjx` de la versión 2 se
rechaza y se reconstruye solo.

**Antes de eso, el formato ya había pasado a la versión 2.** Trae dos columnas nuevas: el rango
alfabético de cada entrada —lo que hace que ordenar por nombre cueste lo que el
resultado y no lo que el índice— y la lista de hijos de cada carpeta, que es lo
que permite entrar en una carpeta sin tocar el disco. Un `index.bdjx` de la
versión 1 **se rechaza a propósito**. El servicio lo reconstruye entero la
primera vez que arranca: ese primer escaneo tarda, y a partir de ahí es
instantáneo. Si quieres forzarlo, borra el archivo:

```powershell
Remove-Item "C:\ProgramData\BDJ Studio\Search Pro\index.bdjx" -ErrorAction SilentlyContinue
```

**El servicio ya no indexa sin licencia.** Era el fallo D-6 de la auditoría: un
servicio con privilegios de sistema recorría el disco entero sin comprobar nada.
Ahora la aplicación le comunica el estado de la licencia al arrancar y el
servicio obedece. La consecuencia práctica es que **si la aplicación no se abre
nunca, no hay índice**. Es lo correcto para un producto con licencia, pero
conviene saberlo antes de verlo.

**La superficie FFI ha cambiado**, así que hay que regenerar los enlaces
(paso 2). Sin ese paso la aplicación no compila.

**Hay un archivo nuevo junto al índice: `overlay.bdjo`.** Es la capa de cambios
recientes. El servicio la publica en cuanto ocurre algo en el disco —cuesta
milisegundos, porque solo contiene lo que ha cambiado— y la aplicación la lee
junto al índice. Es lo que hace que un archivo copiado hace un segundo aparezca
en la lista sin esperar a que se reescriba el índice entero. Se puede borrar sin
miedo: se vuelve a crear sola, y mientras no esté simplemente se ven los cambios
más tarde.

---

## 1. Requisitos

| Herramienta | Versión mínima | Comprobación |
|---|---|---|
| Rust | 1.85 (edición 2024) | `cargo --version` |
| Flutter | 3.24 | `flutter --version` |
| `flutter_rust_bridge_codegen` | 2.13.0 exacta | `flutter_rust_bridge_codegen --version` |
| Inno Setup 6 | 6.x | solo para el instalador de Windows |

Si te falta el generador:

```powershell
cargo install flutter_rust_bridge_codegen --version 2.13.0 --locked
```

La versión tiene que coincidir **exactamente** con la del `Cargo.toml` de
`bdj_search_ffi` (`flutter_rust_bridge = "=2.13.0"`). Una versión distinta genera
enlaces incompatibles con el tiempo de ejecución.

---

## 2. Regenerar los enlaces entre Rust y Dart

**Obligatorio**: `engine/bdj_search_ffi/src/api.rs` ha cambiado.

```powershell
cd "C:\Users\David Zapata\Desktop\Aplicacion_para_DJs\BDJ_Studio_Search_Pro\frontend"
flutter_rust_bridge_codegen generate
```

Reescribe cuatro archivos, que **no** hay que editar a mano:

- `frontend/lib/core/ffi/api.dart`
- `frontend/lib/core/ffi/frb_generated.dart`
- `frontend/lib/core/ffi/frb_generated.io.dart`
- `engine/bdj_search_ffi/src/frb_generated.rs`

Si el generador se queja de que no encuentra el crate, comprueba
`frontend/flutter_rust_bridge.yaml`: debe apuntar a `../engine/bdj_search_ffi/src/api.rs`.

---

## 3. Compilar el motor

```powershell
cd "..\engine"
cargo build --release
cargo test --workspace
```

`cargo test` debe terminar sin fallos. La prueba de latencia sobre diez millones
de entradas está marcada como ignorada porque construye un corpus enorme; para
ejecutarla:

```powershell
cargo test --release -p bdj_search_core --test latency_budget_test -- --ignored --nocapture
```

Y para medir sobre tu propia máquina, con cifras comparables:

```powershell
cargo run --release -p bdj_search_core --example bench_corpus -- 10000000 C:\Temp\corpus.bdjx
```

Los artefactos quedan en `engine\target\release\`:

- `bdj_search_ffi.dll` — la biblioteca que carga la aplicación
- `bdj_search_indexer.exe` — el servicio

---

## 4. Compilar la aplicación

```powershell
cd "..\frontend"
flutter pub get
flutter analyze
flutter build windows --release
```

`flutter analyze` tiene que salir limpio. Si aparece algo, mándamelo y lo
arreglo: no he podido ejecutarlo desde aquí porque en este entorno no hay
cadena de herramientas de Dart.

El ejecutable queda en `frontend\build\windows\x64\runner\Release\`.

Copia la biblioteca del motor junto al ejecutable:

```powershell
Copy-Item "..\..\engine\target\release\bdj_search_ffi.dll" `
          "build\windows\x64\runner\Release\"
Copy-Item "..\..\engine\target\release\bdj_search_indexer.exe" `
          "build\windows\x64\runner\Release\"
```

---

## 5. Poner el servicio en marcha

En desarrollo, en primer plano y con registro en pantalla:

```powershell
cd "..\engine\target\release"
.\bdj_search_indexer.exe --standalone
```

Como servicio de Windows, en una consola **como administrador**:

```powershell
sc.exe create BDJSearchProIndexer binPath= "C:\Ruta\Completa\bdj_search_indexer.exe" start= auto
sc.exe start BDJSearchProIndexer
```

Comprobar que está vivo y que ha publicado algo:

```powershell
sc.exe query BDJSearchProIndexer
dir "C:\ProgramData\BDJ Studio\Search Pro\"
```

Deberías ver `index.bdjx` y `settings.json`.

### Si la aplicación dice que no hay índice

La barra de estado ahora dice el motivo concreto en lugar de «Cargando motor…».
Los cuatro casos:

| Lo que dice | Qué pasa | Qué hacer |
|---|---|---|
| «Sin índice: el servicio no está en marcha» | No existe `index.bdjx` | Arrancar el servicio (paso 5) |
| «Reconstruyendo el índice…» | El índice es de la versión 1 | Esperar a que el servicio termine |
| «Índice dañado: se reconstruirá» | Fichero truncado o corrupto | Esperar, o borrarlo para forzar |
| «Sin permiso para leer el índice» | Permisos de `C:\ProgramData\...` | Revisar los permisos de esa carpeta |

Pasa el ratón por encima del texto: la ayuda emergente da la ruta exacta que se
está mirando.

---

## 6. El instalador

La ruta de Inno Setup en tu equipo es:

```
C:\Users\David Zapata\AppData\Local\Programs\Inno Setup 6\ISCC.exe
```

(El acceso directo del menú Inicio apunta ahí; `ISCC.exe` no está en la carpeta
del menú Inicio, sino en `Programs`.)

```powershell
& "C:\Users\David Zapata\AppData\Local\Programs\Inno Setup 6\ISCC.exe" `
  "C:\Users\David Zapata\Desktop\Aplicacion_para_DJs\BDJ_Studio_Search_Pro\distribution\installer.iss"
```

Antes de generarlo, comprueba que `installer.iss` incluye los tres artefactos:
el ejecutable de la aplicación, `bdj_search_ffi.dll` y `bdj_search_indexer.exe`,
y que registra el servicio.

---

## 7. macOS

El flujo de trabajo de GitHub Actions (`.github/workflows/macos-build.yml`) hace
todo lo anterior en un runner de macOS y sube el `.dmg` como artefacto. Para
compilar a mano:

```bash
cd frontend
flutter_rust_bridge_codegen generate
cd ../engine && cargo build --release
cd ../frontend && flutter build macos --release
```

El agente de usuario se instala con `distribution/macos/postinstall`, que copia
el `.plist` a `/Library/LaunchAgents`.

---

## 8. Comprobación rápida de que todo está bien

1. La barra de estado dice «Índice listo · gen N».
2. Escribir tres letras devuelve resultados sin parpadeo.
3. El contador de la izquierda da un número grande y un tiempo pequeño.
4. Marcar cuatro archivos con Ctrl y arrastrarlos a Ableton mete los cuatro.
5. Con una carpeta abierta, `Ctrl + C` y `Ctrl + V` copian de verdad y aparece el
   panel de progreso abajo a la derecha.
6. `Ctrl + Z` deshace la última operación reversible.
