#!/usr/bin/env python3
"""Comprueba que lo que hay para repartir BDJ Studio Search Pro está bien.

Los fallos que llegan a un usuario final rara vez son de código: son de
empaquetado. Un instalador cuya versión no coincide con la del binario que
lleva dentro, o un indexador que no viaja junto a la app, no se ven abriendo la
app en el equipo de desarrollo.

Este script lo verifica todo de una vez:

  1. Coherencia de versión -- `frontend/pubspec.yaml`,
     `distribution/installer.iss` y los nombres de los artefactos deben hablar
     de la misma versión.
  2. Presencia del instalador de Windows y del binario del indexador.
  3. Checksums de los artefactos, para que quien descargue pueda verificar.

Uso:
    python tools/preflight_release.py
    python tools/preflight_release.py --write-checksums

Códigos de salida: 0 = listo para repartir, 1 = hay algo que corregir.
"""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DISTRIBUTION_DIR = REPO_ROOT / "distribution"
OUTPUT_DIR = REPO_ROOT / "output"
PUBSPEC = REPO_ROOT / "frontend" / "pubspec.yaml"
INSTALLER_SCRIPT = DISTRIBUTION_DIR / "installer.iss"
INDEXER_BINARY = REPO_ROOT / "engine" / "target" / "release" / "bdj_search_indexer.exe"
CHECKSUMS_NAME = "checksums.sha256"

_GREEN, _RED, _YELLOW, _RESET = "\033[32m", "\033[31m", "\033[33m", "\033[0m"


class Report:
    """Acumula el resultado de las comprobaciones."""

    def __init__(self) -> None:
        self.failures: list[str] = []
        self.warnings: list[str] = []

    def ok(self, message: str) -> None:
        print(f"  {_GREEN}OK{_RESET}    {message}")

    def fail(self, message: str) -> None:
        print(f"  {_RED}FALLO{_RESET} {message}")
        self.failures.append(message)

    def warn(self, message: str) -> None:
        print(f"  {_YELLOW}AVISO{_RESET} {message}")
        self.warnings.append(message)


def read_pubspec_version(path: Path) -> str | None:
    """Devuelve el nombre de versión de pubspec (`1.0.0` de `1.0.0+1`)."""
    if not path.is_file():
        return None
    for line in path.read_text(encoding="utf-8").splitlines():
        match = re.match(r"^version:\s*([0-9]+(?:\.[0-9]+)*)", line.strip())
        if match:
            return match.group(1)
    return None


def read_installer_version(path: Path) -> str | None:
    """Devuelve `MyAppVersion` del script de Inno Setup."""
    if not path.is_file():
        return None
    match = re.search(
        r'#define\s+MyAppVersion\s+"([^"]+)"', path.read_text(encoding="utf-8")
    )
    return match.group(1) if match else None


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def check_versions(report: Report) -> str | None:
    print("\nCoherencia de version")
    pubspec_version = read_pubspec_version(PUBSPEC)
    installer_version = read_installer_version(INSTALLER_SCRIPT)

    if pubspec_version is None:
        report.fail(f"no se pudo leer `version:` de {PUBSPEC.name}")
        return None
    report.ok(f"pubspec.yaml declara {pubspec_version}")

    if installer_version is None:
        report.warn(f"no se encontro MyAppVersion en {INSTALLER_SCRIPT.name}")
    elif installer_version != pubspec_version:
        report.fail(
            f"installer.iss declara {installer_version} y pubspec {pubspec_version}: "
            f"el instalador de Windows saldria con el numero equivocado. "
            f"Compila con ISCC /DMyAppVersion={pubspec_version} installer.iss"
        )
    else:
        report.ok(f"installer.iss declara {installer_version}")

    return pubspec_version


def check_artifacts(report: Report, version: str | None) -> list[Path]:
    print("\nArtefactos")

    installers = sorted(
        [
            *DISTRIBUTION_DIR.glob("*Setup*.exe"),
            *OUTPUT_DIR.glob("*Setup*.exe"),
        ]
    )
    if not installers:
        report.warn(
            f"no hay instalador de Windows (buscado en {DISTRIBUTION_DIR.name}/ "
            f"y {OUTPUT_DIR.name}/). Compila con ISCC distribution/installer.iss"
        )
    for installer in installers:
        if version and version not in installer.name:
            report.fail(f"{installer.name} no corresponde a la version {version}")
        else:
            report.ok(f"{installer.name} presente")

    if INDEXER_BINARY.is_file():
        report.ok(f"{INDEXER_BINARY.relative_to(REPO_ROOT)} presente")
    else:
        report.fail(
            f"falta {INDEXER_BINARY.relative_to(REPO_ROOT)}: el instalador lo "
            f"incluye, compila el engine primero"
        )

    return installers


def check_checksums(report: Report, artifacts: list[Path], write: bool) -> None:
    print("\nChecksums")
    if not artifacts:
        report.warn("no hay instaladores que resumir")
        return

    checksums_path = DISTRIBUTION_DIR / CHECKSUMS_NAME
    computed = {artifact.name: sha256(artifact) for artifact in artifacts}

    if write:
        checksums_path.write_text(
            "".join(f"{digest}  {name}\n" for name, digest in sorted(computed.items())),
            encoding="utf-8",
        )
        report.ok(f"{CHECKSUMS_NAME} escrito con {len(computed)} entradas")
        return

    if not checksums_path.is_file():
        report.warn(
            f"falta {CHECKSUMS_NAME}; generalo con --write-checksums para que "
            f"quien descargue pueda verificar el archivo"
        )
        return

    recorded: dict[str, str] = {}
    for line in checksums_path.read_text(encoding="utf-8").splitlines():
        parts = line.split()
        if len(parts) == 2:
            recorded[parts[1]] = parts[0]

    for name, digest in computed.items():
        if name not in recorded:
            report.fail(f"{name} no aparece en {CHECKSUMS_NAME}")
        elif recorded[name] != digest:
            report.fail(f"{name}: el checksum no coincide, el archivo cambio")
        else:
            report.ok(f"{name}: checksum correcto")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Verifica que distribution/ esta listo para repartir.",
    )
    parser.add_argument(
        "--write-checksums",
        action="store_true",
        help="regenera el archivo de checksums en vez de verificarlo",
    )
    args = parser.parse_args(argv)

    if not DISTRIBUTION_DIR.is_dir():
        print(f"::error::No existe {DISTRIBUTION_DIR}")
        return 1

    print(f"Preflight de distribucion sobre {DISTRIBUTION_DIR}")
    report = Report()

    version = check_versions(report)
    artifacts = check_artifacts(report, version)
    check_checksums(report, artifacts, args.write_checksums)

    print("\n" + "=" * 72)
    if report.failures:
        print(f"{_RED}NO SE PUEDE DISTRIBUIR{_RESET}: {len(report.failures)} problema(s)")
        for failure in report.failures:
            print(f"  - {failure}")
        return 1

    if report.warnings:
        print(f"{_YELLOW}Listo, con {len(report.warnings)} aviso(s){_RESET}")
    else:
        print(f"{_GREEN}Todo correcto: se puede distribuir.{_RESET}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))