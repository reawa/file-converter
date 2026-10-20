# Release checklist

## Version

Bump the same version in:

- `package.json`
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`

Tag as `vX.Y.Z` (example: `v1.1.2`) and push the tag to trigger `.github/workflows/release.yml`.

## Updater signing (required for in-app updates)

Private key (local, never commit):

- `%USERPROFILE%\.tauri\file-converter.key`
- Public key is embedded in `src-tauri/tauri.conf.json` → `plugins.updater.pubkey`

Local signed build:

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content "$env:USERPROFILE\.tauri\file-converter.key" -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
npm run tauri build
```

GitHub Actions secrets on `reawa/file-converter`:

- `TAURI_SIGNING_PRIVATE_KEY` — contents of `file-converter.key`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — empty string if the key has no password

NSIS assets are named with dots for spaces, e.g. `File.Converter_1.1.2_x64-setup.exe`.

The workflow publishes the NSIS installer + `.sig`, then commits `updater/latest.json` on `main` with a **public** release download URL:

`https://github.com/reawa/file-converter/releases/download/<tag>/<asset>`

Endpoint used by the app:

`https://raw.githubusercontent.com/reawa/file-converter/main/updater/latest.json`

In-app updates prompt by default (Update button). Opt-in silent install: set `localStorage` key `file-converter.autoUpdate` to `"true"`.

## Windows Authenticode (optional)

Unsigned installers trigger SmartScreen warnings. Updater minisign signing is separate from Authenticode.

### Option A: Certificate thumbprint

Add under `bundle.windows` in `tauri.conf.json`:

```json
"certificateThumbprint": "YOUR_CERT_THUMBPRINT",
"digestAlgorithm": "sha256",
"timestampUrl": "https://timestamp.digicert.com"
```

### Option B: Custom sign command

```json
"signCommand": "signtool sign /fd sha256 /tr https://timestamp.digicert.com /td sha256 /a %1"
```

## yt-dlp updates

The GUI does **not** run `yt-dlp -U`. Ship a new app release, or run:

```bash
fconv update-ytdlp
```

YouTube downloads may fetch yt-dlp's EJS challenge solver from GitHub (`--remote-components ejs:github`) unless `FILE_CONVERTER_YTDLP_REMOTE_EJS=0`.

## FFmpeg / yt-dlp compliance

Release builds fetch LGPL FFmpeg and yt-dlp via `beforeBuildCommand`. Ship `THIRD_PARTY_NOTICES.md` with distributions.
