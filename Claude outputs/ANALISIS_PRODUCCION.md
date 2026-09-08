# BDJ Studio Search Pro — análisis de preparación para producción

Fecha del análisis: 3 de septiembre de 2026. Alcance: todo el repositorio tal como está en este espacio de trabajo (`engine/`, `frontend/`, `distribution/`, `.github/workflows/`, `docs/`). No se modificó ningún archivo — esto es solo diagnóstico.

## Aviso antes de leer lo demás

Este espacio de trabajo en la nube es donde hemos estado editando código durante varias sesiones, pero **no es un repositorio git** (no hay carpeta `.git`) y, según el historial de esta conversación, ya sufrió al menos un incidente de pérdida de archivos (un `cp -r` desde una carpeta de staging que sobrescribió el árbol de trabajo). Dos de los hallazgos más graves de este informe — el frontend no compila por archivos que faltan, y no existen las carpetas nativas de Windows/macOS — son coherentes con esa clase de incidente, y también coherentes con que `.github/workflows/macos-build.yml` sí invoca `flutter build macos --release` como si esas carpetas existieran en tu repositorio real.

**Antes de asumir que tu producto está roto, revisa tu repositorio de GitHub** (el que usa `dzapataba/...` según los workflows) y confirma si `frontend/windows/`, `frontend/macos/Runner.xcodeproj` y `frontend/lib/features/search/{widgets/preview_pane.dart, providers/preview_provider.dart}` existen ahí. Si existen allí, el problema es que esta copia de trabajo se quedó atrás y hay que resincronizarla, no que el producto esté roto. Si de verdad no existen en ningún lado, entonces sí son bloqueadores reales y quedan descritos abajo con todo el detalle.

Todo lo demás en este informe — el motor Rust, la lógica de licencia, el instalador, el CI — sí es código real y estable que se puede evaluar tal cual, independientemente de esta duda.

---

## Bloqueadores — no funciona en producción tal como está

### 1. En macOS, el indexador jamás recibe la licencia y por lo tanto nunca indexa

Verificado de punta a punta, leyendo los tres archivos implicados:

- `distribution/macos/postinstall` instala el indexador como **LaunchDaemon del sistema** (`launchctl bootstrap system ...`), es decir, corre como `root` y **sin la variable de entorno `HOME`**.
- `bdj_search_ipc/src/unix_socket.rs:14-26` (`default_unix_socket_path`) calcula la ruta del socket de control como `$HOME/Library/Application Support/BDJ Studio/Search Pro/bdj_search_pro.sock` **si `HOME` existe**, y si no, cae a `/var/run/bdj_search_pro.sock`. El propio comentario en el código dice "en macOS el agente de usuario..." — asume que este proceso corre como *LaunchAgent* por usuario, con `HOME` disponible. No es lo que se instala.
- Tanto el cliente (`bdj_search_ffi/src/service.rs:29-30`, que llama la interfaz de Flutter) como el servidor (`bdj_search_indexer/src/main.rs:23,1400`) llaman a esa misma función sin ningún ajuste adicional.

Resultado: la app de Flutter (corriendo como el usuario, con `HOME` sí definido) calcula una ruta de socket bajo `~/Library/...`; el demonio (corriendo como root, sin `HOME`) escucha en `/var/run/...`. Nunca se encuentran. Y el comentario en `bdj_search_ffi/src/api.rs:1434` dice literalmente: *"Sin esto el servicio no indexa: era el agujero por el que un servicio elevado recorría el disco entero sin comprobar nada"* — es decir, `service_set_license()` es exactamente el mensaje que nunca llega, y sin licencia activada `may_index()` (`bdj_search_core/src/index/settings.rs:100-108`) devuelve `false` para siempre. El índice del archivo sí se resuelve bien en ambos lados gracias a un *fallback* que comprueba si el archivo existe (`api.rs:306-341`), pero el canal de control no tiene ese mismo fallback.

Consecuencia práctica: en un Mac recién instalado, tal como está empaquetado hoy, el buscador nunca indexará nada, aunque la licencia se active correctamente en la interfaz.

### 2. El frontend no compila: dos archivos referenciados no existen en esta copia

`lib/main.dart:30` importa `features/search/widgets/preview_pane.dart` y lo instancia en `lib/main.dart:1082`. `lib/features/search/providers/search_provider.dart:17` y `lib/features/search/widgets/explorer_bar.dart:10` importan `preview_provider.dart` y usan `previewProvider` en varios sitios (`search_provider.dart:823,872`, `explorer_bar.dart:78,83`). Ninguno de los dos archivos existe en `lib/features/search/{widgets,providers}/`, y una búsqueda en todo el disco de este contenedor no encontró ninguna copia de respaldo. Si esto también falta en tu repositorio real, nada compila hasta que se recupere o se reescriba esa función de vista previa.

### 3. Faltan por completo las carpetas nativas de Windows y macOS

`frontend/windows/` no existe en ningún lado de este árbol. `frontend/macos/` solo contiene `Runner/Info.plist` y `Runner/Release.entitlements` — falta `Runner.xcodeproj`, el `Podfile`, `AppDelegate.swift` y todo lo que Flutter genera con `flutter create --platforms=windows,macos .`. Sin esto, `flutter build windows` y `flutter build macos` no pueden ejecutarse desde esta copia. Ver el aviso al principio: esto puede ser un hueco de esta copia de trabajo, no de tu proyecto real — verifícalo en tu repositorio antes de reaccionar.

### 4. El instalador de Windows no garantiza que el DLL del motor viaje con la app

`distribution/installer.iss:31-32` copia la salida de `flutter build windows` completa más `bdj_search_indexer.exe`, pero **no** copia `bdj_search_ffi.dll` explícitamente — asume que ya está dentro de la carpeta de salida de Flutter porque alguien lo copió ahí a mano de antemano. `docs/COMO_COMPILAR.md` (líneas ~139-146 y ~208-210) de hecho documenta ese paso manual y avisa que hay que revisarlo antes de empaquetar — es decir, el propio documento admite que es un punto frágil. Y en `.github/workflows/windows-build.yml`, el paso "Build Rust Indexer & FFI" y el paso "Build Inno Setup Installer" no tienen entre medias ningún comando que copie ese DLL a la carpeta de salida de Flutter. Un build 100% automático de CI empaquetaría un instalador sin el motor de búsqueda dentro. Esto es exactamente la misma familia de error que ya causó el incidente del "DLL obsoleto" descrito en el historial de este proyecto — solo que aquí ni siquiera hay un paso manual documentado que lo cubra en automático.

### 5. El indexador puede morir entero por un solo panic en cualquier parte

`bdj_search_indexer/src/main.rs` usa `.lock().unwrap()` en doce sitios distintos sobre los mismos dos `Mutex` (`overlay` y `watch`), y no hay ningún `catch_unwind` ni `panic::set_hook` en todo el archivo (confirmado, cero apariciones). El indexador corre en un único hilo de larga duración (`run_daemon`, línea 1340, invocado desde `main()` en la línea 2339) pensado para funcionar días seguidos. Si cualquier operación de indexado o de la capa de cambios entra en pánico mientras uno de esos dos `Mutex` está tomado, el `Mutex` queda envenenado y **cada iteración posterior también entra en pánico**, lo que termina el proceso completo del servicio en segundo plano — sin log de crash, sin reinicio automático salvo lo que `KeepAlive`/el Administrador de Servicios de Windows hagan por su cuenta. Contrasta con el patrón correcto que el mismo archivo usa en otros sitios (`if let Ok(mut s) = settings.lock() { ... }`), lo que sugiere que esto no fue una decisión deliberada sino una inconsistencia.

### 6. En macOS, el sandbox de la app choca con lo que la app necesita hacer

`frontend/macos/Runner/Release.entitlements` solo declara `com.apple.security.app-sandbox = true`, sin ningún entitlement de acceso a archivos (`com.apple.security.files.*`) ni ninguna cadena de uso (`NSDesktopFolderUsageDescription` y similares) en `Info.plist`. Una app así empaquetada no puede navegar carpetas arbitrarias del usuario ni pedir Acceso Total al Disco de forma normal. El indexador corre fuera del sandbox (como LaunchDaemon), pero la interfaz sí queda dentro, y nada en el paquete explica al usuario cómo conceder ese permiso.

---

## Importante — hay que resolverlo antes de un lanzamiento serio

**Sin firma de código ni notarización en ningún lado.** No hay `SignTool=`/certificado en `installer.iss`, ni `signtool` en el workflow de Windows, ni `codesign --sign`/`notarytool submit` en macOS (el único uso de `codesign` en `macos-build.yml` es de solo lectura, para comprobar entitlements). Windows mostrará "Editor desconocido" (SmartScreen) y macOS bloqueará o pondrá en cuarentena la app sin notarizar — no es un aviso cosmético, en macOS puede impedir directamente que la app abra.

**No hay mecanismo de actualización automática.** Búsqueda confirmada: no hay Sparkle, Squirrel, MSIX ni ningún "buscar actualizaciones" en el código. Cada corrección de bug exige que el usuario se entere por su cuenta y reinstale a mano.

**No hay reporte de errores en producción.** No hay Sentry, Crashpad ni nada equivalente ni en Flutter ni en los binarios Rust. Si la copia de un usuario falla en su máquina, no hay forma de que tú te enteres salvo que te escriba.

**El paso de generación de bindings (`flutter_rust_bridge_codegen`) nunca se ejecuta en CI.** Ambos workflows instalan la herramienta (`cargo install flutter_rust_bridge_codegen ...`) pero jamás la invocan para regenerar `frb_generated.rs`/`.dart`. Eso significa que CI depende de que los archivos generados ya estén en el repositorio y coincidan con `api.rs` — si algún día cambias una firma en `api.rs` sin regenerar y confirmar los archivos generados, CI puede seguir en verde con bindings desincronizados, y el fallo solo aparecería en tiempo de ejecución. Esto es justo la clase de problema de orden de compilación que ya ocurrió antes en este proyecto.

**Desinstalación incompleta en ambas plataformas.** En Windows, `[UninstallDelete]` borra el índice en `ProgramData`, pero no toca `%LOCALAPPDATA%\BDJ Studio\Search Pro\.license.dat` (la ruta que usa `bdj_search_core/src/license.rs:62-64`) ni nada que Flutter haya guardado con `flutter_secure_storage`. En macOS no existe ningún script de desinstalación: nada da de baja el `LaunchDaemon` ni borra `/Library/Application Support/BDJ Studio`. Una reinstalación limpia hoy no queda realmente limpia.

**El arrastre múltiple hacia fuera de la app puede perder archivos.** En `virtualized_table.dart`, la función que arma el arrastre (`envolverConArrastre`, líneas ~410-432) solo adjunta `Formats.fileUri` para el **primer** archivo seleccionado; el resto de las rutas van solo como `Formats.plainText`. Casi ningún destino real (Explorador, Finder, un DAW como rekordbox o Serato) lee texto plano para resolver una soltada de archivos — leen el formato de lista de archivos. Arrastrar una selección de varias pistas fuera de la app probablemente solo transfiera la primera, sin ningún aviso. Vale la pena confirmar esto en una prueba real de arrastre múltiple.

**35 bloques `catch (_) {}` en el frontend, algunos silencian fallos importantes sin dejar rastro.** Por ejemplo, en `search_provider.dart` (líneas cercanas a 1212 y 1222) un fallo de `ffi.reloadIfChanged()` tras una operación de archivo se traga sin ningún `debugPrint` — si esto empezara a fallar de forma persistente en el campo, la vista dejaría de refrescarse tras copiar/mover/borrar y no quedaría ninguna pista de por qué.

**Hay un diálogo de licencia duplicado y sin usar.** `lib/features/license/license_dialog.dart` no está referenciado en ningún lugar del código (confirmado) — es una implementación paralela abandonada junto al flujo real en `features/licensing/`. No rompe nada, pero es ruido que puede confundir a quien toque ese código después, y probablemente `flutter analyze` se queje de símbolos sin usar dentro de ese archivo.

**Los logs del servicio de Windows no van a ningún lado.** El indexador llama a `tracing_subscriber::fmt::init()` con salida estándar por defecto, pero se compila con `windows_subsystem = "windows"`, que no tiene consola adjunta cuando corre como servicio — esa salida simplemente se pierde. En macOS sí hay redirección a archivo (`StandardOutPath`/`StandardErrorPath` en el `plist`), pero Windows no tiene equivalente en ningún lado del instalador. Si el indexador falla en la máquina de un cliente en Windows, hoy no queda ningún registro que revisar.

**`tools/verify_contracts.py` no está conectado a nada.** Comprueba que ciertas funciones FFI y enums de IPC existan, pero ni `windows-build.yml` ni `macos-build.yml` lo invocan — es un script que alguien tiene que acordarse de correr a mano.

---

## Recomendable — no bloquea el lanzamiento pero conviene resolverlo pronto

Sin rutas extendidas de Windows (`\\?\`): no se encontró ningún manejo de `MAX_PATH`/prefijo `\\?\` en `bdj_search_fsops` ni en `bdj_search_fs/windows` — una biblioteca de DJ con carpetas muy anidadas (más de 260 caracteres de ruta) puede fallar en silencio al renombrar, copiar o crear, salvo que el manifiesto de la app ya declare soporte de rutas largas (no verificado aquí).

Sin protección contra una segunda instancia del propio indexador (más allá de lo que ya hace el gestor de servicios de Windows): el binario soporta un modo de ejecución directa para desarrollo sin ningún candado de instancia única, así que correrlo dos veces a la vez — un desarrollador probando en su máquina, por ejemplo — puede corromper `index.bdjx`/`overlay.bdjo`/`settings.json` por escritura concurrente.

Sin ícono de app en el bundle de macOS (`CFBundleIconFile` está vacío en `Info.plist`).

Sin ningún tipo de internacionalización — todo el texto está en español, escrito directamente en los widgets, sin `flutter_localizations` ni archivos `.arb`. Es consistente (no es un intento a medias), así que solo importa si algún día planeas otro idioma.

Sin persistencia del tamaño o posición de la ventana entre sesiones: `window_manager` se usa para título, mostrar/ocultar y foco, pero no para guardar ni restaurar el tamaño o la posición de la ventana.

Cobertura de pruebas todavía delgada en el frontend: `test/rutas_de_fila_test.dart` cubre bien la lógica de rutas que arreglamos hoy, y `test/widget_test.dart` cubre la pantalla de activación de licencia, pero el resto de `search_provider.dart` (el archivo de estado más grande de la app) no tiene ninguna prueba automática todavía.

---

## Lo que ya está sólido (para que quede constancia, no solo lo que falta)

El motor Rust en sí — el índice mmap, el radix sort, la capa de superposición (`overlay.bdjo`), el resolvedor unificado de rutas, las 159 pruebas que corren hoy en verde con cero warnings de clippy — está en buen estado. La validación de licencia SPP3 es completamente criptográfica y offline (token Ed25519, sin URL de servidor visible en este repositorio, sin bandera de depuración que la salte). El bloqueo de instancia única de la app en Flutter usa un candado de archivo exclusivo a nivel de sistema operativo, correcto y resistente a una instancia anterior que haya muerto de forma abrupta. Los protectores contra operaciones peligrosas (`bdj_search_fsops/src/guards.rs`) — raíz de volumen, carpeta personal, symlinks — están bien pensados y probados. Y sí existe integración continua real (`.github/workflows/`) corriendo pruebas, clippy y `flutter analyze` en cada push a `main`/`master` en corredores de Windows y macOS de verdad — el problema no es la ausencia de CI, es que ese CI no firma, no empaqueta el DLL, y no está conectado a un paso de publicación de versión.
