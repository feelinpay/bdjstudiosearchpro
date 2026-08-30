# Bloque 1 terminado — qué falta hacer a mano

**Fecha:** 30 de agosto de 2026

Los archivos ya están escritos en tu disco. Quedan tres cosas que esta sesión no puede hacer
por sí sola: **borrar dos archivos** y **ejecutar las compilaciones**.

---

## 1. Borra estos dos archivos (obligatorio, si no la compilación falla)

Ya no los referencia nadie, pero siguen en el disco y `cargo` y `flutter analyze` los verán:

```
C:\Users\David Zapata\Desktop\Aplicacion_para_DJs\BDJ_Studio_Search_Pro\engine\bdj_search_core\src\license.rs
C:\Users\David Zapata\Desktop\Aplicacion_para_DJs\BDJ_Studio_Search_Pro\frontend\lib\features\license\license_dialog.dart
```

La carpeta `frontend\lib\features\license\` queda vacía; bórrala también. La licencia ahora
vive en `frontend\lib\features\licensing\`, con `-ing`.

---

## 2. BDJ Studio License — compilar

El catálogo ya tiene Search Pro (los 7 puntos) y la versión subió a **1.0.3**.

```powershell
cd "C:\Users\David Zapata\Desktop\Aplicacion_para_DJs\BDJ_Studio_License\frontend"
flutter clean
flutter pub get
flutter analyze
flutter build windows --release
flutter build apk --release

cd ..\distribution
& "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" bdj_studio_license_setup.iss
```

El instalador sale en la misma carpeta `distribution` como
`BDJ_Studio_License_Setup_1.0.3.exe`. El APK lo genera Flutter en
`frontend\build\app\outputs\flutter-apk\app-release.apk`; cópialo a `distribution` como
`BDJ_Studio_License_1.0.3.apk`.

**Ojo con la ruta de Inno.** La que me diste
(`AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Inno Setup 6`) es la carpeta de
accesos directos del menú Inicio, no la instalación. El compilador de línea de comandos es
`ISCC.exe` y suele estar en `C:\Program Files (x86)\Inno Setup 6\`. Si ahí no está, ejecuta
esto y usa lo que salga:

```powershell
Get-ChildItem -Path "C:\Program Files (x86)","C:\Program Files" -Filter ISCC.exe -Recurse -ErrorAction SilentlyContinue | Select-Object FullName
```

### Prueba de que el .txt en Android quedó arreglado
Instala el APK, entra al panel, genera una licencia y pulsa **Descargar .txt**. Antes fallaba
porque `FilePicker.saveFile` se llamaba sin el parámetro `bytes`, que es obligatorio desde
file_picker 10: en Android el propio plugin escribe el archivo a través del selector del
sistema y sin bytes devolvía `null`. Ahora se le pasan los bytes y en escritorio se sigue
escribiendo el archivo a mano, que es lo que allí hace falta.

---

## 3. BDJ Studio Search Pro — compilar

**Primero regenera los bindings.** Yo edité a mano los archivos generados por
flutter_rust_bridge para quitar la licencia falsa, porque este entorno no pudo descargar el
SDK de Dart. El motor Rust compila y pasa todas las pruebas así, pero el generador es la
fuente de verdad: si mi edición fue exacta, este paso no cambia nada; si no, lo corrige.

```powershell
cd "C:\Users\David Zapata\Desktop\Aplicacion_para_DJs\BDJ_Studio_Search_Pro\frontend"
cargo install flutter_rust_bridge_codegen --version 2.13.0 --locked   # solo la primera vez
flutter_rust_bridge_codegen generate

flutter pub get
flutter analyze
flutter test

cd ..\engine
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --release
```

Si `flutter_rust_bridge_codegen generate` protesta con `prefix not found`, es porque la línea
`rust_output:` de `frontend\flutter_rust_bridge.yaml` tiene una ruta que el generador no sabe
resolver. Bórrala: el valor por defecto ya es el correcto.

---

## Lo que cambió, en corto

### Search Pro
- **Eliminado el esquema de licencia propietario.** `license.rs`, `LicenseInfoFfi`,
  `get_license_status` y `activate_product_key` ya no existen. Con ellos se va la prueba de
  14 días y el `|| parts.len() == 4` que aceptaba cualquier clave `BDJ6-XXXX-XXXX-XXXX`.
- **Licenciamiento SPP3 real**, portado de Sample Pad: `DeviceFingerprint` (HWID V2 sin tocar
  una línea), `SecureStorageImpl` con autoprueba del llavero, `LicenseManager` con
  `productCode: 'bdj_studio_search_pro'` y claves `bdj.search_pro.*`. Toda la criptografía la
  pone `bdj_license_core`.
- **El ID del dispositivo vuelve a ser el de la suite.** Mismo formato
  `XXXX-XXXX-XXXX-XXXX` y mismo valor que muestra Sample Pad en la misma máquina, así que
  BDJ Studio License ya puede emitirle licencias.
- **Portón de arranque.** `LicenseGate` decide antes de construir nada: cargando → activación
  → aplicación. `SearchHomeScreen` solo existe en la rama con licencia, así que un equipo sin
  activar no llega ni a mapear el índice. Revalida al volver del segundo plano.
- **Pantalla de activación en blanco**, con los dos pasos numerados, el ID en monoespaciada y
  el botón *Copiar ID*. Más un panel de licencia detrás del icono de la barra, para que un
  cliente ya activado pueda leer su ID cuando pida soporte o desactivar el equipo.
- **Arreglado que no compilara en macOS**: `PipeServer` está detrás de `#[cfg(windows)]` y se
  importaba sin condición en el indexador y en `pipe_test.rs`.
- **`clippy -D warnings` limpio** en todo el workspace.
- **Pruebas nuevas** en `widget_test.dart`: que sin licencia la app se queda en la pantalla de
  activación, y que `BDJ6-0000-0000-0000` —la clave que antes entraba— es rechazada. Si esa
  segunda prueba deja de fallar la activación, hemos vuelto atrás.

### BDJ Studio License
- Los 7 puntos del catálogo, con `bdj_studio_search_pro` y código numérico 6.
- Corregida la descarga del `.txt` en Android.
- Versión 1.0.2 → **1.0.3** en `pubspec.yaml` y en el `.iss`.

---

## Verificación que sí pude ejecutar

```
cargo clippy --workspace --all-targets -- -D warnings   → limpio
cargo test --workspace --release                        → 14 pruebas, 0 fallos
```

Las 3 pruebas de la licencia falsa desaparecieron con ella; una de ellas afirmaba que
`BDJ6-AAAA-BBBB-CCCC` era válida.

Lo que **no** pude ejecutar: `flutter analyze`, `flutter test`, `flutter build` ni `ISCC.exe`.
Esta sesión llega a tus archivos pero no puede ejecutar programas en tu PC. Si algo de lo
anterior falla al compilar, pásame el error y lo corrijo.

---

## Siguiente bloque

Con la licencia cerrada, el siguiente por impacto es el **bloque 3 del informe**: los datos
verdaderos del índice. Hoy `parent_file_ref` se descarta y todas las rutas salen como
`C:\C:\archivo.wav`, y los tamaños y fechas están fijados a 0 y a noviembre de 2023. Es lo que
hace que las columnas Ruta, Tamaño y Modificado, y los filtros `ruta:`, `tam:` y `mod:`, no
sirvan para nada. Dime y entro.
