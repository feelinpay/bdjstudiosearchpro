#!/bin/bash
# Script para construir el instalador .pkg de BDJ Studio Search Pro en macOS
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

APP_PATH="${1:-}"
OUTPUT_PKG="${2:-$ROOT_DIR/BDJ_Studio_Search_Pro.pkg}"
VERSION="${3:-1.0.0}"

if [ -z "$APP_PATH" ] || [ ! -d "$APP_PATH" ]; then
    echo "Uso: $0 <ruta_al_app> [ruta_salida_pkg] [version]"
    exit 1
fi

PAYLOAD_DIR="$(mktemp -d /tmp/bdj_pkg_payload.XXXXXX)"
trap 'rm -rf "$PAYLOAD_DIR"' EXIT

echo "Preparando payload para el paquete .pkg..."
mkdir -p "$PAYLOAD_DIR/Applications"
mkdir -p "$PAYLOAD_DIR/Library/Application Support/BDJ Studio/Search Pro"

# 1. Copiar aplicación a /Applications
cp -R "$APP_PATH" "$PAYLOAD_DIR/Applications/"

# 2. Copiar indexador y plist del LaunchDaemon
INDEXER_SRC="$APP_PATH/Contents/MacOS/bdj_search_indexer"
if [ ! -f "$INDEXER_SRC" ]; then
    INDEXER_SRC="$ROOT_DIR/engine/target/release/bdj_search_indexer"
fi
cp "$INDEXER_SRC" "$PAYLOAD_DIR/Library/Application Support/BDJ Studio/Search Pro/bdj_search_indexer"
chmod 755 "$PAYLOAD_DIR/Library/Application Support/BDJ Studio/Search Pro/bdj_search_indexer"

PLIST_SRC="$SCRIPT_DIR/com.bdjstudio.searchpro.indexer.plist"
cp "$PLIST_SRC" "$PAYLOAD_DIR/Library/Application Support/BDJ Studio/Search Pro/com.bdjstudio.searchpro.indexer.plist"
chmod 644 "$PAYLOAD_DIR/Library/Application Support/BDJ Studio/Search Pro/com.bdjstudio.searchpro.indexer.plist"

SCRIPTS_DIR="$SCRIPT_DIR"
chmod +x "$SCRIPTS_DIR/postinstall"

echo "Ejecutando pkgbuild..."
if [ -n "${INSTALLER_SIGNING_IDENTITY:-}" ]; then
    pkgbuild \
        --root "$PAYLOAD_DIR" \
        --scripts "$SCRIPTS_DIR" \
        --identifier "com.bdjstudio.searchpro.pkg" \
        --version "$VERSION" \
        --install-location "/" \
        --timestamp \
        --sign "$INSTALLER_SIGNING_IDENTITY" \
        "$OUTPUT_PKG"
else
    pkgbuild \
        --root "$PAYLOAD_DIR" \
        --scripts "$SCRIPTS_DIR" \
        --identifier "com.bdjstudio.searchpro.pkg" \
        --version "$VERSION" \
        --install-location "/" \
        "$OUTPUT_PKG"
fi

echo "✓ Paquete .pkg generado con éxito: $OUTPUT_PKG"
