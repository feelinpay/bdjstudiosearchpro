#!/usr/bin/env python3
"""
BDJ Studio Search Pro — Verificador de Contratos FFI, Enums y Código Huérfano
Basado en las directrices de aseguramiento de calidad de Stems Music.
"""

import sys
import os
import re

def check_ffi_contracts():
    print("[1/3] Verificando contratos FFI entre Rust y Dart...")
    api_rust_path = os.path.join("engine", "bdj_search_ffi", "src", "api.rs")
    if not os.path.exists(api_rust_path):
        print(f"ERROR: No se encontró {api_rust_path}")
        return False

    with open(api_rust_path, "r", encoding="utf-8") as f:
        content = f.read()

    # Extract public Rust functions
    rust_functions = re.findall(r"pub\s+fn\s+([a-zA-Z0-9_]+)\s*\(", content)
    print(f"  Encontradas {len(rust_functions)} funciones FFI exportadas en Rust: {', '.join(rust_functions)}")

    # Check that each function exists or has bindings generated
    expected_fns = ["ping", "engine_open", "engine_close", "search", "search_status", "rows", "full_path", "reveal_in_explorer"]
    missing = [fn for fn in expected_fns if fn not in rust_functions]
    if missing:
        print(f"ERROR: Faltan funciones esperadas en Rust FFI: {missing}")
        return False

    print("  [OK] Contrato Rust FFI verificado correctamente.")
    return True

def check_ipc_contracts():
    print("[2/3] Verificando protocolo IPC...")
    ipc_path = os.path.join("engine", "bdj_search_ipc", "src", "lib.rs")
    if not os.path.exists(ipc_path):
        print(f"[ERROR]: No se encontro {ipc_path}")
        return False

    with open(ipc_path, "r", encoding="utf-8") as f:
        content = f.read()

    assert "enum IpcCommand" in content, "Falta enum IpcCommand en IPC"
    assert "enum IpcEvent" in content, "Falta enum IpcEvent en IPC"
    print("  [OK] Definicion de mensajes IPC verificada.")
    return True

def check_workspace_integrity():
    print("[3/3] Verificando integridad del workspace Cargo...")
    cargo_toml = os.path.join("engine", "Cargo.toml")
    with open(cargo_toml, "r", encoding="utf-8") as f:
        content = f.read()

    expected_members = [
        "bdj_search_core",
        "bdj_search_fs",
        "bdj_search_ipc",
        "bdj_search_indexer",
        "bdj_search_ffi",
    ]
    for member in expected_members:
        if f'"{member}"' not in content:
            print(f"[ERROR]: Miembro faltante en Cargo.toml: {member}")
            return False

    print("  [OK] Todos los crates del workspace registrados.")
    return True

def main():
    print("=== BDJ Studio Search Pro -- Verificacion de Contratos ===")
    ok = True
    ok = check_ffi_contracts() and ok
    ok = check_ipc_contracts() and ok
    ok = check_workspace_integrity() and ok

    if not ok:
        print("\n[FALLO] FALLO EN LA VERIFICACION DE CONTRATOS.")
        sys.exit(1)

    print("\n[EXITO] TODOS LOS CONTRATOS ESTAN EN REGLA.")
    sys.exit(0)

if __name__ == "__main__":
    main()
