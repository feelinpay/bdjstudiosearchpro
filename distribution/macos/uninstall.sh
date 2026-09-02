#!/bin/bash
# Desinstalación completa de BDJ Studio Search Pro en macOS.
#
# El instalador registra un LaunchDaemon de sistema y su índice vive en
# /Library/Application Support/BDJ Studio/Search Pro. La carita de los datos
# del usuario (si instaló con su carpeta particular o es el daemon) está en
# ~/Library/Application Support. La política del producto es que NADA
# sobreviva a la desinstalación, así que se borra ambos.
#
# Requiere permisos de administrador (descarga el daemon y borra rutas de
# sistema). Uso:
#     sudo bash distribution/macos/uninstall.sh

set -eu

DAEMON_LABEL="com.bdjstudio.searchpro.indexer"
DAEMON_PLIST="/Library/LaunchDaemons/com.bdjstudio.searchpro.indexer.plist"
SYSTEM_INSTALL_DIR="/Library/Application Support/BDJ Studio/Search Pro"
LOG_DIR="/Library/Logs/BDJ Studio"
BUNDLE_ID="com.bdjstudio.frontend"
SUPPORT="$HOME/Library/Application Support"

if [ "$(id -u)" -ne 0 ]; then
  echo "Este script necesita privilegios de administracion: ejecutalo con sudo." >&2
  exit 1
fi

remove_if_present() {
  if [ -e "$1" ] || [ -L "$1" ]; then
    echo "  -> borrando $1"
    rm -rf "$1"
  fi
}

echo "Desinstalando BDJ Studio Search Pro..."

# 1. Descargar y retirar el LaunchDaemon del indexador.
if /bin/launchctl print "system/$DAEMON_LABEL" >/dev/null 2>&1; then
  echo "  -> descargando el daemon $DAEMON_LABEL"
  /bin/launchctl bootout "system/$DAEMON_LABEL" 2>/dev/null || true
fi
remove_if_present "$DAEMON_PLIST"
remove_if_present "$SYSTEM_INSTALL_DIR"
remove_if_present "$LOG_DIR"

# 2. Datos del frontend y del usuario (soporte de path_provider, cachés y
#    preferencias). Solo rutas de la propia app.
for dir in \
  "$SUPPORT/BDJ Studio/Search Pro" \
  "$SUPPORT/BDJ Studio Search Pro" \
  "$SUPPORT/bdj_studio_search_pro" \
  "$SUPPORT/$BUNDLE_ID" \
  "$HOME/Library/Caches/BDJ Studio Search Pro" \
  "$HOME/Library/Caches/$BUNDLE_ID"
do
  remove_if_present "$dir"
done
remove_if_present "$HOME/Library/Preferences/$BUNDLE_ID.plist"

echo "Listo. El servicio ya no arranca y no queda ningún dato de la aplicación."