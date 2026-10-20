# Fetches/updates yt-dlp for Windows x64 into src-tauri/binaries/win64.
# Source: https://github.com/yt-dlp/yt-dlp/releases/latest
# Always refreshes so YouTube 403s from stale builds are less common.

$ErrorActionPreference = "Stop"

$tripletDir = Join-Path $PSScriptRoot "..\src-tauri\binaries\win64"
$ytdlpExe = Join-Path $tripletDir "yt-dlp.exe"
$url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
$tmp = Join-Path $env:TEMP "yt-dlp-download.exe"

Write-Host "Downloading latest yt-dlp from GitHub ($url)..."
$ProgressPreference = "SilentlyContinue"
New-Item -ItemType Directory -Force -Path $tripletDir | Out-Null
Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing -TimeoutSec 300

if (-not (Test-Path $tmp) -or (Get-Item $tmp).Length -lt 1MB) {
    throw "Failed to download yt-dlp.exe"
}

Move-Item -Path $tmp -Destination $ytdlpExe -Force
$ver = & $ytdlpExe --version 2>$null
Write-Host "yt-dlp installed to $tripletDir (version $ver)"
