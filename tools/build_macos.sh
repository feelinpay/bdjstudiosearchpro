#!/bin/bash
# BDJ Studio Search Pro - Build de la parte nativa (macOS)
#
# Mismo problema que en Windows: libbdj_search_ffi.dylib la compila cargo, no
# Flutter, y debe ir dentro del bundle .app. Sin este paso, "flutter build
# macos" produce una app que no carga el puente (el chequeo de hash de
# flutter_rust_bridge falla o no encuentra la libreria).
#
# Hace los tres pasos juntos para que nunca queden desincronizados:
#   1. Regenera el puente (flutter_rust_bridge_codegen generate).
#   2. Compila el engine en release (FFI + indexador).
#   3. Compila la app y copia libbdj_search_ffi.dylib + bdj_search_indexer
#      dentro de Contents/MacOS del bundle, en Release y (si existe) Debug.
#
# Uso:
#     bash tools/build_macos.sh                # build completo (release)
#     bash tools/build_macos.sh --copy-only    # solo re-copiar libs a los
#                                              # bundles ya compilados (para
#                                              # "flutter run" en debug)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ENGINE="$ROOT/engine"
FRONTEND="$ROOT/frontend"
APP_NAME="bdj_studio_search_pro.app"

copy_libs_into() {
  local contents_macos="$1"
  if [ ! -d "$contents_macos" ]; then
    echo "  (sin bundle en $contents_macos, se omite)"
    return
  fi
  cp "$ENGINE/target/release/libbdj_search_ffi.dylib" "$contents_macos/"
  cp "$ENGINE/target/release/bdj_search_indexer" "$contents_macos/"
  echo "  -> ok en $contents_macos"
}

# Un indexador --standalone de una sesion anterior puede dejar el binario "en
# uso" y abortar la copia. Solo se tocan los que corren desde este arbol de
# build; un servicio instalado del sistema se respeta.
stop_stale_indexers() {
  pkill -f "build/macos/Build/Products" 2>/dev/null || true
  pkill -f "engine/target/debug/bdj_search_indexer" 2>/dev/null || true
  pkill -f "engine/target/release/bdj_search_indexer" 2>/dev/null || true
  sleep 0.3
}

cd "$ROOT"

if [ "${1:-}" = "--copy-only" ]; then
  echo "Copiando libs a los bundles ya compilados..."
  stop_stale_indexers
  for profile in Release Debug; do
    copy_libs_into "$FRONTEND/build/macos/Build/Products/$profile/$APP_NAME/Contents/MacOS"
  done
  echo "Listo."
  exit 0
fi

echo "== 1/4 Regenerando el puente flutter_rust_bridge =="
cd "$FRONTEND"
flutter_rust_bridge_codegen generate

echo "== 2/4 Compilando el engine en release =="
cd "$ENGINE"
cargo build --release -p bdj_search_ffi -p bdj_search_indexer

echo "== 3/4 Compilando la aplicacion (macOS release) =="
cd "$FRONTEND"
flutter build macos --release

echo "== 4/4 Copiando librerias nativas dentro del bundle =="
stop_stale_indexers
for profile in Release Debug; do
  APP_BUNDLE="$(find "$FRONTEND/build/macos/Build/Products/$profile" -maxdepth 1 -type d -name '*.app' -print -quit 2>/dev/null || true)"
  if [ -n "$APP_BUNDLE" ]; then
    mkdir -p "$APP_BUNDLE/Contents/Frameworks" "$APP_BUNDLE/Contents/MacOS"
    cp "$ENGINE/target/release/libbdj_search_ffi.dylib" "$APP_BUNDLE/Contents/Frameworks/" 2>/dev/null || true
    cp "$ENGINE/target/release/libbdj_search_ffi.dylib" "$APP_BUNDLE/Contents/MacOS/"
    cp "$ENGINE/target/release/bdj_search_indexer" "$APP_BUNDLE/Contents/MacOS/"
    echo "  -> ok en $APP_BUNDLE"
  fi
done

echo ""
echo "Listo. El .app lleva libbdj_search_ffi.dylib y el indexador dentro."
echo "Para debug: flutter run -d macos; despues, bash tools/build_macos.sh --copy-only"