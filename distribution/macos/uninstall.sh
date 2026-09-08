#!/bin/bash
# Script de desinstalacion para BDJ Studio Search Pro en macOS
set -e

if [ "$(id -u)" -ne 0 ]; then
    echo "Este script requiere privilegios de administrador. Ejecuta: sudo bash uninstall.sh"
    exit 1
fi

DAEMON_PLIST="/Library/LaunchDaemons/com.bdjstudio.searchpro.indexer.plist"
INSTALL_DIR="/Library/Application Support/BDJ Studio/Search Pro"
LOG_DIR="/Library/Logs/BDJ Studio"
APP_BUNDLE="/Applications/BDJ Studio Search Pro.app"

echo "Cerrando procesos activos de BDJ Studio Search Pro..."
pkill -9 -f "BDJ Studio Search Pro" 2>/dev/null || true
pkill -9 -f "bdj_search_indexer" 2>/dev/null || true

echo "Deteniendo el servicio de indexacion..."
launchctl bootout system/com.bdjstudio.searchpro.indexer 2>/dev/null || true

if [ -f "$DAEMON_PLIST" ]; then
    echo "Eliminando LaunchDaemon..."
    rm -f "$DAEMON_PLIST"
fi

if [ -n "$SUDO_USER" ]; then
    USER_HOME=$(eval echo "~$SUDO_USER")
    USER_AGENT="$USER_HOME/Library/LaunchAgents/com.bdjstudio.searchpro.plist"
    if [ -f "$USER_AGENT" ]; then
        echo "Eliminando inicio automatico de usuario..."
        rm -f "$USER_AGENT"
    fi
fi

if [ -d "$INSTALL_DIR" ]; then
    echo "Eliminando datos del indice y configuracion..."
    rm -rf "$INSTALL_DIR"
fi

if [ -d "$LOG_DIR" ]; then
    echo "Eliminando logs..."
    rm -rf "$LOG_DIR"
fi

if [ -d "$APP_BUNDLE" ]; then
    echo "Eliminando aplicacion..."
    rm -rf "$APP_BUNDLE"
fi

echo "BDJ Studio Search Pro ha sido desinstalado correctamente."
exit 0
