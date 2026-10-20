# Removes regenerable dev/build artifacts from the project.

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent

$paths = @(
    (Join-Path $root "dist"),
    (Join-Path $root "src-tauri\target")
)

foreach ($path in $paths) {
    if (Test-Path $path) {
        Remove-Item $path -Recurse -Force
        Write-Host "Removed $path"
    }
}

$ffplay = Join-Path $root "src-tauri\binaries\win64\ffplay.exe"
if (Test-Path $ffplay) {
    Remove-Item $ffplay -Force
    Write-Host "Removed bundled ffplay.exe (not used by the app)"
}

Write-Host "Clean complete."
