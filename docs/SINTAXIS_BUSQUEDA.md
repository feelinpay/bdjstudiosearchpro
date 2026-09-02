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
- **apps / aplicaciones / ejecutables:** `exe msi dll app dmg pkg bat cmd sh`
- **carpetas / carpeta / folder:** Sólo directorios

Los tres grupos con más de un nombre —`documentos`/`docs`/`documento`,
`proyectos`/`dj`/`proyecto`, `comprimidos`/`zip`/`comprimido`— aceptan
cualquiera de sus alias. La lista de extensiones de cada grupo vive en un único
sitio del motor (`query::compiled::file_type_extensions`), de modo que la barra
de filtros de la interfaz y el evaluador de consultas no pueden discrepar.

---

## 4. Qué cuesta cada filtro

El motor reordena los términos de un Y lógico del filtro más barato al más caro,
así que escribir `ruta:"Music" ext:wav` y `ext:wav ruta:"Music"` cuesta lo mismo:
en ambos casos se descarta primero por extensión.

| Filtro | Coste | Por qué |
|---|---|---|
| `ext:`, `tipo:`, `tam:`, `mod:`, `creado:`, `archivo:`, `folder:` | mínimo | Comparan un entero de una columna del índice, sin mirar el nombre |
| Texto suelto, `may:` | bajo | Recorrido vectorizado de la arena de nombres |
| `carpeta:` | medio | Salta a la entrada del padre |
| `regex:` | alto | Autómata sobre cada nombre candidato |
| `ruta:` | el más alto | Reconstruye la ruta completa de cada entrada candidata. **Va siempre el último** |

Un consejo práctico: acompaña siempre `ruta:` y `regex:` de algún filtro barato.
`ruta:"Cabina" ext:wav` recorre una fracción de lo que recorre `ruta:"Cabina"` a
secas.

---

## 5. Refinamiento al teclear

Cada pulsación produce una consulta más restrictiva que la anterior. Cuando el
motor puede demostrar que el resultado nuevo es forzosamente un subconjunto del
anterior, vuelve a filtrar solo ese resultado en lugar de recorrer el índice
entero.

Funciona al añadir letras a un término, al añadir términos, al estrechar un
rango de tamaño o de fecha, y —lo más útil— **al teclear dentro de una consulta
de varias palabras**: pasar de `michael bil` a `michael billie` refina en lugar
de recorrer.

No funciona al borrar letras, y es correcto que no lo haga: la consulta pasa a
ser menos restrictiva y refinar perdería resultados en silencio, que es peor que
tardar.
