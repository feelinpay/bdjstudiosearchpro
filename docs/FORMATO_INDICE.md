# BDJ Studio Search Pro — Especificación Binaria del Formato de Índice (.bdjx)

**Versión del formato:** 1  
**Extensión:** `.bdjx`  
**Alineación:** Todas las secciones de columnas están alineadas a 64 bytes (tamaño de línea de caché x86_64 / ARM64).  
**Endianness:** Little-endian nativo.  
**Cero copias (`zero-copy`):** El archivo se mapea en memoria mediante `mmap(2)` en macOS o `CreateFileMappingW` / `MapViewOfFile` en Windows. La lectura se realiza proyectando `&[T]` usando `bytemuck` sin deserialización intermedia.

---

## 1. Encabezado (Header) — 64 bytes fijos

| Offset | Tamaño | Tipo | Campo | Descripción |
|---|---|---|---|---|
| 0 | 8 | `[u8; 8]` | `magic` | Identificador de 8 bytes: `"BDJXIDX\0"` (0x42, 0x44, 0x4A, 0x58, 0x49, 0x44, 0x58, 0x00) |
| 8 | 4 | `u32` | `version` | Versión del formato. Actualmente `1`. Si difiere, se rechaza y se reconstruye. |
| 12 | 4 | `u32` | `header_crc` | Checksum XXH3 (32-bit truncado) de los bytes 16..64. |
| 16 | 8 | `u64` | `entry_count` | Número total de entradas indexadas (filas). |
| 24 | 8 | `u64` | `arena_len` | Tamaño en bytes de la arena contigua de nombres UTF-8. |
| 32 | 8 | `u64` | `created_at` | Timestamp Unix de construcción (segundos). |
| 40 | 8 | `u64` | `generation` | Contador de generación; se incrementa en cada compactación. |
| 48 | 16 | `[u8; 16]` | `install_id` | UUID de 128 bits de la instalación (evita mapear índices foráneos). |

---

## 2. Tabla de Secciones — Desplazamientos y Longitudes (Offset & Lengths)

Inmediatamente tras el encabezado (offset 64), se sitúa la tabla de descriptores de sección.  
Cada descriptor contiene:
- `offset: u64` (desde el byte 0 del archivo, siempre múltiplo de 64).
- `len: u64` (en bytes).

Columnas indexadas paralelamente (Struct of Arrays):

1. `parent` (`u32` × `entry_count`): Índice del registro padre. `u32::MAX` indica raíz de volumen.
2. `name_off` (`u32` × `entry_count`): Offset inicial dentro de `name_arena`.
3. `name_len` (`u8` × `entry_count`): Longitud del nombre en bytes (1..=255).
4. `flags` (`u8` × `entry_count`):
   - bit 0: Directorio (`1`) vs Archivo (`0`)
   - bit 1: Oculto
   - bit 2: Sistema
   - bit 3: Enlace simbólico / Junction
   - bit 4: No-ASCII presente (requiere fallback Unicode en comparación)
   - bit 5..7: Reservados
5. `ext_id` (`u16` × `entry_count`): ID internado de extensión. `0` = sin extensión.
6. `volume` (`u8` × `entry_count`): Índice en la tabla de volúmenes adjunta (0..=255).
7. `size` (`u64` × `entry_count`): Tamaño de archivo en bytes.
8. `mtime` (`u32` × `entry_count`): Fecha de modificación en segundos Unix.
9. `ctime` (`u32` × `entry_count`): Fecha de creación en segundos Unix.
10. `alive` (`u64` × `ceil(entry_count / 64)`): Mapa de bits donde `1` = vivo, `0` = lápida / borrado.
11. `name_order` (`u32` × `entry_count`): Permutación precalculada en orden alfabético.
12. `name_arena` (`[u8]` × `arena_len`): Bloque denso contiguo de nombres UTF-8 sin delimitadores nulos.
13. `ext_table` (`[u8]`): Bloque de extensiones internadas (pares `[u16 len, bytes]` terminados en tabla).
14. `vol_table` (`[u8]`): Bloque de metadatos de volúmenes montados/desmontados (UUID, letra/punto de montaje, tipo FS, etiqueta).

---

## 3. Resolución de Rutas sin Materialización

La ruta completa nunca se almacena contigua en disco para ahorrar cientos de megabytes.  
Para resolver la ruta de la entrada `i`:
1. Leer `vol = volume[i]` -> obtener prefijo de volumen (ej: `"C:\"` o `"/Volumes/Cabina/"`).
2. Recorrer la cadena de ancestros: `p = parent[i]`.
3. Subir recursivamente hasta que `p == u32::MAX`.
4. Reconstruir la ruta concatenando los segmentos de nombres leídos desde `name_arena[name_off[k] .. name_off[k] + name_len[k]]`.
Para una profundidad típica de 5 a 8 carpetas, esto toma < 2 microsegundos.
