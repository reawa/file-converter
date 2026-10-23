# Fetches yt-dlp for Windows x64 into src-tauri/binaries/win64.
# Pinned release (do not use /latest/). Bump this URL on purpose.
# https://github.com/yt-dlp/yt-dlp/releases/tag/2026.08.19

$ErrorActionPreference = "Stop"

$tripletDir = Join-Path $PSScriptRoot "..\src-tauri\binaries\win64"
$ytdlpExe = Join-Path $tripletDir "yt-dlp.exe"
$url = "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp.exe"
$tmp = Join-Path $env:TEMP "yt-dlp-download.exe"

Write-Host "Downloading pinned yt-dlp from GitHub ($url)..."
$ProgressPreference = "SilentlyContinue"
New-Item -ItemType Directory -Force -Path $tripletDir | Out-Null
Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing -TimeoutSec 300

if (-not (Test-Path $tmp) -or (Get-Item $tmp).Length -lt 1MB) {
    throw "Failed to download yt-dlp.exe"
}

Move-Item -Path $tmp -Destination $ytdlpExe -Force
$ver = & $ytdlpExe --version 2>$null
Write-Host "yt-dlp installed to $tripletDir (version $ver)"
