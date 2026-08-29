<#
.SYNOPSIS
    Script de limpieza de archivos temporales, huérfanos y código muerto para BDJ Studio Search Pro.
    Adaptado de los estándares de aseguramiento de calidad de BDJ Studio Stems Music.
#>

param(
    [switch]$DryRun,
    [switch]$CleanAll
)

$ErrorActionPreference = "Stop"

Write-Host "=== BDJ Studio Search Pro -- Limpieza de Huérfanos y Temporales ===" -ForegroundColor Cyan

$targets = @(
    "engine/target",
    "frontend/build",
    "frontend/.dart_tool",
    "*.bdjx",
    "*.bdjx.tmp",
    "*.pdb",
    "*.log"
)

$filesToDelete = @()

foreach ($pattern in $targets) {
    if ($pattern.Contains("/")) {
        if (Test-Path $pattern) {
            $filesToDelete += Get-Item $pattern
        }
    } else {
        $found = Get-ChildItem -Path . -Filter $pattern -Recurse -Depth 4 -ErrorAction SilentlyContinue | Where-Object { $_.FullName -notlike "*\.git\*" }
        if ($found) {
            $filesToDelete += $found
        }
    }
}

if ($filesToDelete.Count -eq 0) {
    Write-Host "[OK] No se encontraron residuos ni archivos huérfanos temporales." -ForegroundColor Green
    exit 0
}

Write-Host "Archivos / directorios identificados ($($filesToDelete.Count)):" -ForegroundColor Yellow
foreach ($item in $filesToDelete) {
    Write-Host "  - $($item.FullName)"
}

if ($DryRun) {
    Write-Host "`n[DRY RUN] No se eliminó ningún archivo." -ForegroundColor Cyan
} elseif ($CleanAll) {
    Write-Host "`nEliminando elementos..." -ForegroundColor Magenta
    foreach ($item in $filesToDelete) {
        Remove-Item -Path $item.FullName -Recurse -Force -ErrorAction SilentlyContinue
    }
    Write-Host "[OK] Limpieza completada con éxito." -ForegroundColor Green
} else {
    Write-Host "`nEjecute con -CleanAll para confirmar la eliminación o -DryRun para simular." -ForegroundColor Yellow
}
