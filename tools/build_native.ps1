param(
    [switch]$SkipCodegen
)

<#
    BDJ Studio Search Pro - Build de la parte nativa (Windows)

    Evita el error "Content hash on Dart side ... different from Rust side":
    la libreria nativa (bdj_search_ffi.dll) la compila cargo, no Flutter, y si se
    regenera el puente pero no se recompila y copia esa DLL, la app carga una DLL
    vieja y el chequeo de hash de flutter_rust_bridge la rechaza.

    Este script hace los tres pasos juntos para que nunca queden desincronizados:
      1. Regenera el puente (flutter_rust_bridge_codegen generate).
      2. Compila el engine en release (FFI + indexador).
      3. Copia la DLL y el indexador a las carpetas Debug y Release donde los
         carga la aplicacion.

    Uso (desde cualquier directorio):
        powershell -ExecutionPolicy Bypass -File tools\build_native.ps1

    Con -SkipCodegen si el puente ya esta regenerado y solo quieres recompilar y
    copiar:
        powershell -ExecutionPolicy Bypass -File tools\build_native.ps1 -SkipCodegen

    Despues: flutter run / flutter build windows --release.
#>

$ErrorActionPreference = 'Stop'

$Root       = Split-Path -Parent $PSScriptRoot
$Engine     = Join-Path $Root 'engine'
$Frontend   = Join-Path $Root 'frontend'
$FFIDebug   = Join-Path $Frontend 'build\windows\x64\runner\Debug'
$FFIRelease = Join-Path $Frontend 'build\windows\x64\runner\Release'

if (-not (Test-Path (Join-Path $Engine 'Cargo.toml'))) {
    throw "No se encontro el engine en $Engine. Ejecuta el script desde tools/"
}

# 1. Puente FFI
if (-not $SkipCodegen) {
    Write-Host '== 1/3 Regenerando el puente flutter_rust_bridge =='
    Push-Location $Frontend
    try {
        & flutter_rust_bridge_codegen generate
        if ($LASTEXITCODE -ne 0) { throw "flutter_rust_bridge_codegen fallo (exit $LASTEXITCODE)" }
    }
    finally { Pop-Location }
} else {
    Write-Host '== 1/3 Puente FFI: omitido (-SkipCodegen) =='
}

# 2. Engine (release)
Write-Host '== 2/3 Compilando el engine en release =='
Push-Location $Engine
try {
    & cargo build --release -p bdj_search_ffi -p bdj_search_indexer
    if ($LASTEXITCODE -ne 0) { throw "cargo build fallo (exit $LASTEXITCODE)" }
}
finally { Pop-Location }

# 3. Copia a Debug y Release
Write-Host '== 3/3 Copiando bdj_search_ffi.dll y el indexador a Debug y Release =='
# Un indexador --standalone de una sesion anterior puede dejar el .exe "en
# uso" y abortar la copia. Solo se tocan los que corren desde este arbol de
# build; un servicio instalado del sistema (otro directorio) se respeta.
Get-Process bdj_search_indexer -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -like '*frontend\build\windows*' } |
    Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 300

$artifacts = @(
    (Join-Path $Engine 'target\release\bdj_search_ffi.dll'),
    (Join-Path $Engine 'target\release\bdj_search_indexer.exe')
)
foreach ($dir in @($FFIDebug, $FFIRelease)) {
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    foreach ($bin in $artifacts) {
        if (-not (Test-Path $bin)) { throw "Falta artefacto: $bin" }
        $dst = Join-Path $dir (Split-Path $bin -Leaf)
        try {
            Copy-Item $bin $dir -Force
            Write-Host "   -> $dst"
        }
        catch {
            $destinoEnUso = $_.Exception.Message -match 'est[aá] siendo utilizado|being used by another process'
            Write-Warning "No se pudo sobrescribir $dst : $($_.Exception.Message)"
            if ($destinoEnUso) {
                Write-Warning "   El destino esta en uso (indexador o app en ejecucion)."
                Write-Warning "   Cierralo y vuelve a ejecutar el script para llevar la copia al dia."
            }
        }
    }
}

Write-Host ''
Write-Host 'Listo. Ya puedes ejecutar "flutter run" (usara la DLL recien compilada).'