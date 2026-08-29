# BDJ Studio Search Pro — Manual de Sintaxis de Búsqueda

Search Pro ejecuta búsquedas instantáneas en tiempo real a medida que escribes, sin requerir presionar Enter ni pulsar ningún botón de buscar.

---

## 1. Operadores Básicos

- **Espacio (AND):** `house vocal 128` encuentra archivos cuyos nombres contengan las tres palabras en cualquier orden.
- **Barra vertical `|` (OR):** `wav | flac` encuentra archivos que contengan `wav` o `flac`.
- **Negación `!` (NOT):** `!remix` excluye archivos que contengan la palabra `remix`.
- **Frases exactas `" "`:** `"Billie Jean"` busca exactamente esa secuencia con el espacio incluido.
- **Agrupación `( )`:** `(house | techno) !remix ext:wav` aplica precedencia a las expresiones.

---

## 2. Filtros por Función (con alias en español)

| Filtro | Alias ES | Ejemplo | Descripción |
|---|---|---|---|
| `ext:` | `ext:` | `ext:wav\|flac\|aiff` | Coincidencia de extensión |
| `size:` | `tam:` | `tam:>50mb`, `size:10mb..50mb` | Tamaño de archivo (`kb`, `mb`, `gb`, `tb`, comparadores `<`, `>`, `..`) |
| `dm:` | `mod:` | `mod:hoy`, `dm:>2026-08-01` | Fecha de modificación (`hoy`, `ayer`, `estasemana`, `estemes`, `esteaño`, fechas `YYYY-MM-DD`) |
| `dc:` | `creado:` | `creado:estasemana` | Fecha de creación |
| `path:` | `ruta:` | `ruta:"Music\Latin"` | Filtra dentro de la ruta completa de carpetas |
| `parent:` | `carpeta:` | `carpeta:Samples` | Nombre de la carpeta contenedora directa |
| `type:` | `tipo:` | `tipo:audio`, `type:dj` | Grupos predefinidos de archivos |
| `regex:` | `regex:` | `regex:^DJ_.*\.wav$` | Expresión regular sobre el nombre |
| `case:` | `may:` | `may:Remix` | Distinción estricta de mayúsculas y minúsculas |
| `file:` | `archivo:` | `file:` | Restringe la búsqueda exclusivamente a archivos |
| `folder:` | `carpeta_solo:` | `folder:` | Restringe la búsqueda exclusivamente a directorios |

---

## 3. Tipos Predefinidos (`type:` / `tipo:`)

- **audio:** `wav flac aiff aif mp3 m4a ogg opus wma alac ape wv aac`
- **video:** `mp4 mov avi mkv m4v webm mpg mpeg wmv flv`
- **imagen:** `jpg jpeg png gif webp heic tiff bmp svg psd`
- **documentos / docs:** `pdf docx doc txt rtf odt xlsx pptx md`
- **proyectos / dj:** `als flp ptx cpr logicx rpp nki nkm cue m3u m3u8 xml`
- **comprimidos / zip:** `zip rar 7z tar gz bz2 xz`
- **apps:** `exe msi dll app dmg pkg`
- **carpetas / folder:** Sólo directorios
