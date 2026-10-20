# File Converter

A lightweight Windows desktop app for fast, quality-preserving conversion of video, audio, and images. Built with [Tauri 2](https://tauri.app/) and [FFmpeg](https://ffmpeg.org/).

Use it only on media you own or have permission to process. This project is not affiliated with Twitch, YouTube, or other platforms.

## Features

- Video, audio, and image conversion via FFmpeg
- Automatic **COPY** (stream copy, no quality loss) when codecs allow it
- **ENCODE HQ** near-lossless presets when re-encoding is required
- Batch queue with per-file progress, elapsed time, and ETA
- Drag-and-drop, file picker, or paste an `http(s)` media / HLS (`.m3u8`) link
- yt-dlp downloads for supported media links (bundled on Windows releases)
- Absolute start/end range for HLS downloads
- CLI (`fconv`) installed next to the app and added to your user PATH
- In-app updates from signed GitHub releases

## Install (Windows)

1. Download the latest `File Converter_*_x64-setup.exe` from [Releases](https://github.com/reawa/file-converter/releases).
2. Run the installer (current-user install under `%LOCALAPPDATA%\File Converter`).
3. Open **File Converter** from the Start menu.

The CLI is available as `fconv` after opening a new terminal (PATH entry: `%LOCALAPPDATA%\File Converter\cli`).

Installers are updater-signed. They may still show a SmartScreen warning until an Authenticode certificate is added — see [docs/RELEASE.md](docs/RELEASE.md).

## CLI

```bash
fconv convert <files|folders|links> [-r] [-f webm] [-o DIR] [-n NAME] [--start 5:03:00 --end 5:05:00] [-y] [--json]
fconv info <inputs> [--json]
fconv formats [--category video|audio|image]
fconv check
fconv update-ytdlp
```

## Requirements

**End users (installer):** Windows x64. FFmpeg and yt-dlp are bundled.

**Development**

- Node.js 22+ and Rust (stable)
- Run `npm run fetch-ffmpeg` and `npm run fetch-ytdlp` once (or rely on `beforeBuildCommand`)

## Development

```bash
npm install
npm run fetch-ffmpeg
npm run fetch-ytdlp
npm run tauri dev
```

Tests:

```bash
cd src-tauri && cargo test --lib
npx tsc --noEmit
```

## Build release installer

```bash
npm run build:installer
```

Produces:

- `src-tauri/target/release/bundle/nsis/File Converter_<version>_x64-setup.exe`

Tagged releases on GitHub (`v*`) build via Actions and publish the updater manifest. Maintainer notes: [docs/RELEASE.md](docs/RELEASE.md).

## Supported formats

**Video inputs:** mp4, mkv, webm, avi, mov, ts, flv, m4v, and more  
**Audio inputs:** mp3, wav, flac, aac, ogg, m4a, wma, opus  
**Image inputs:** jpg, png, webp, gif, bmp, tiff, ico, avif, jxl

Output formats are grouped by category in the UI.

## License

MIT — see [LICENSE](LICENSE). Bundled FFmpeg is LGPL; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
