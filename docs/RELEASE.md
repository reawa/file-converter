# Release checklist

## Version

Bump the same version in:

- `package.json`
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`

Tag as `vX.Y.Z` (example: `v1.1.0`) and push the tag to trigger `.github/workflows/release.yml`.

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

The workflow publishes the NSIS installer + `.sig`, then commits `updater/latest.json` on `main` for the private-repo endpoint:

`https://raw.githubusercontent.com/reawa/file-converter/main/updater/latest.json`

### Private-repo update token (installed apps)

While the GitHub repo is private, place a GitHub PAT (Contents read) at:

`%APPDATA%\com.stardvn.file-converter\update-token`

Example: `gh auth token` output, one line, no quotes. When the repo is public, switch asset URLs to public download links and remove the token requirement.

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

## FFmpeg / yt-dlp compliance

Release builds fetch LGPL FFmpeg and yt-dlp via `beforeBuildCommand`. Ship `THIRD_PARTY_NOTICES.md` with distributions.
