# Security Policy

## Supported versions

Only the latest release published on GitHub is supported with fixes.

## Reporting a vulnerability

Open a private security advisory on the GitHub repository, or email the maintainer through the contact listed on the GitHub profile for `stardvn` / `reawa`.

Please include:

- Affected version and platform (Windows x64)
- Steps to reproduce
- Impact (data loss, remote code, privilege escalation, etc.)

Do not open a public issue for unfixed vulnerabilities.

## Scope notes

File Converter shells out to bundled FFmpeg and yt-dlp for media you provide. Use it only on files and streams you have the right to process. Report issues in path handling, updater signature verification, or command injection in job inputs.
