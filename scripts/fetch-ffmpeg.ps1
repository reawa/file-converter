# Fetches FFmpeg (LGPL shared build) for Windows x64 into src-tauri/binaries/win64.
# Source: https://github.com/BtbN/FFmpeg-Builds (ffmpeg-master-latest-win64-lgpl-shared.zip)

$ErrorActionPreference = "Stop"

$tripletDir = Join-Path $PSScriptRoot "..\src-tauri\binaries\win64"
$ffmpegExe = Join-Path $tripletDir "ffmpeg.exe"
$ffprobeExe = Join-Path $tripletDir "ffprobe.exe"

if ((Test-Path $ffmpegExe) -and (Test-Path $ffprobeExe)) {
    Write-Host "FFmpeg binaries already present in $tripletDir"
    exit 0
}

$url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-lgpl-shared.zip"
$zip = Join-Path $env:TEMP "ffmpeg-win64-lgpl-shared.zip"
$extractRoot = Join-Path $env:TEMP "ffmpeg-lgpl-shared-extract"

Write-Host "Downloading FFmpeg from GitHub ($url)..."
$ProgressPreference = "SilentlyContinue"
Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing -TimeoutSec 600

if (Test-Path $extractRoot) {
    Remove-Item $extractRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $extractRoot | Out-Null
Expand-Archive -Path $zip -DestinationPath $extractRoot -Force

$binDir = Get-ChildItem -Path $extractRoot -Recurse -Directory -Filter "bin" |
    Where-Object { Test-Path (Join-Path $_.FullName "ffmpeg.exe") } |
    Select-Object -First 1

if (-not $binDir) {
    throw "Could not find ffmpeg.exe in downloaded archive."
}

New-Item -ItemType Directory -Force -Path $tripletDir | Out-Null
Get-ChildItem -Path $binDir.FullName -File | Where-Object {
    $_.Name -eq "ffmpeg.exe" -or $_.Name -eq "ffprobe.exe" -or $_.Extension -eq ".dll"
} | ForEach-Object {
    Copy-Item $_.FullName (Join-Path $tripletDir $_.Name) -Force
}

Write-Host "FFmpeg installed to $tripletDir"
