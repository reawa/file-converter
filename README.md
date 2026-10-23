# File Converter

A lightweight Windows desktop app for fast, quality-preserving conversion of video, audio, and images. Built with [Tauri 2](https://tauri.app/) and [FFmpeg](https://ffmpeg.org/).

Use it only on media you own or have permission to process. This project is not affiliated with Twitch, YouTube, or other platforms.

## Features

- Video, audio, and image conversion via FFmpeg
- Automatic **COPY** (stream copy, no quality loss) when codecs allow it
- **ENCODE HQ** near-lossless presets when re-encoding is required
- Batch queue with per-file progress, elapsed time, and ETA
- Drag-and-drop or file picker
- CLI (`fconv`) installed next to the app and added to your user PATH
- Signed in-app updates (prompt + one-click Install)
- Close hides to the tray. Right-click the tray icon to turn **Close to tray** off, or choose Quit

### Downloads

Paste a link only for media you have the right to save.

- **HLS:** any `http` or `https` `.m3u8` playlist, with an optional start and end
- **Other links:** any other `http` or `https` URL that [yt-dlp](https://github.com/yt-dlp/yt-dlp) supports, including YouTube and Twitch

YouTube downloads may fetch yt-dlp’s EJS challenge solver. Set `FILE_CONVERTER_YTDLP_REMOTE_EJS=0` to turn that off (YouTube may then fail).

### Re-encode

Examples: PNG to JPG, MOV to MP4, a video to MP3.

**Video in:** mp4, mkv, webm, avi, mov, ts, flv, m4v, wmv, mpeg, mpg, 3gp, m3u8  
**Video out:** mp4, mkv, webm, avi, mov, ts, gif  
**Audio extract from video:** mp3, wav, flac, aac, ogg, m4a

**Audio in:** mp3, wav, flac, aac, ogg, m4a, wma, opus, aiff  
**Audio out:** mp3, wav, flac, aac, ogg, m4a

**Image in:** jpg, jpeg, png, webp, gif, bmp, tiff, tif, ico, avif, jxl  
**Image out:** jpg, png, webp, gif, bmp, tiff, ico, avif (jxl is input only)

## Install (Windows)

1. Download the latest `File.Converter_*_x64-setup.exe` from [Releases](https://github.com/reawa/file-converter/releases).
2. Run the installer (current-user install under `%LOCALAPPDATA%\File Converter`).
3. Open **File Converter** from the Start menu.

The CLI is available as `fconv` after opening a new terminal (PATH entry: `%LOCALAPPDATA%\File Converter\cli`).

Installers are updater-signed (minisign). They may still show a SmartScreen warning until an Authenticode certificate is added — see [docs/RELEASE.md](docs/RELEASE.md).

## CLI

```bash
fconv convert <files|folders|links> [-r] [-f webm] [-o DIR] [-n NAME] [--start 5:03:00 --end 5:05:00] [-y] [--json]
fconv info <inputs> [--json]
fconv formats [--category video|audio|image]
fconv check
fconv update-ytdlp
```

## Privacy and trust notes

- Pasted `http(s)` links are fetched by this PC (via FFmpeg or yt-dlp). Do not paste untrusted URLs.
- Remote HLS playlists cannot use `file:` URIs. Local playlists still can.
- The app updater is signature-verified. The GUI does **not** auto-run `yt-dlp -U`; use `fconv update-ytdlp` or install a new release.
- YouTube downloads may pull yt-dlp’s EJS challenge solver from GitHub. Disable with `FILE_CONVERTER_YTDLP_REMOTE_EJS=0` (YouTube may then fail).
- Updates show a prompt by default. Silent auto-install: set `localStorage` `file-converter.autoUpdate` to `"true"`.

## Requirements

**End users (installer):** Windows x64. FFmpeg and yt-dlp are bundled. YouTube links need Node.js or Deno on PATH.

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
