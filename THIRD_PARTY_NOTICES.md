# Third-Party Notices

## FFmpeg

File Converter bundles or invokes [FFmpeg](https://ffmpeg.org/) for media conversion.

Release builds include a **LGPL** Windows FFmpeg build from
[BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds), pinned to
`autobuild-2026-06-30-13-34` (`ffmpeg-N-125365-g9a01c1cb6a-win64-lgpl-shared`).

If you received a copy of FFmpeg with this application, you may obtain the
corresponding source code and license text from:

- https://ffmpeg.org/download.html
- https://github.com/BtbN/FFmpeg-Builds

Under the LGPL, you may replace the bundled FFmpeg binaries with your own
build as long as you comply with the license terms.

## yt-dlp

Release builds bundle [yt-dlp](https://github.com/yt-dlp/yt-dlp) `2026.08.19` for downloading
supported media links. yt-dlp is distributed under the
[Unlicense](https://github.com/yt-dlp/yt-dlp/blob/master/LICENSE).

## Inter font

The UI uses the Inter font via [@fontsource/inter](https://fontsource.org/fonts/inter),
distributed under the [SIL Open Font License 1.1](https://scripts.sil.edu/OFL).

## Tauri, React, and other dependencies

See `package.json` and `src-tauri/Cargo.toml` for additional dependencies.
Refer to each project's repository for license terms.
