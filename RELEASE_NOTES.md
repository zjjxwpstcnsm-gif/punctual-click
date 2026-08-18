# Punctual 0.1.0-alpha.5

Punctual is a local-first scheduled click assistant built with Rust and GPUI.

## Highlights

- Schedule one or more click tasks with millisecond fields and IANA time zones.
- Automatically infer purchase, submit, checkout and confirmation buttons.
- Validate manually entered button text against a real visible clickable element.
- Re-locate targets before execution and dispatch only one logical click.
- Confirm results from same-page navigation, SPA changes, new tabs and windows.
- Automatically detect Chrome, Edge, Brave, Arc, Vivaldi, Chromium, Opera, Firefox and Safari.
- Prefer Chrome, then a supported system default, then other installed browsers.
- Fall back to a bundled managed browser when no suitable browser is installed.
- Persist tasks and execution logs locally in SQLite.

## Downloads

- `Punctual-0.1.0-alpha.5-macos-arm64.dmg` — Apple Silicon macOS package.
- `Punctual-0.1.0-alpha.5-windows-x64-setup.exe` — per-user Windows installer.
- `Punctual-0.1.0-alpha.5-windows-x64-portable.zip` — portable Windows package.
- `SHA256SUMS.txt` — combined integrity checks.

## Signing status

This Alpha release is not intended to appear as a verified commercial publisher build:

- macOS uses ad-hoc code signing and is not Apple-notarized;
- Windows is not Authenticode-signed.

Users should verify SHA-256 values and may need to explicitly allow the application in their operating-system security settings.

## Safety boundary

Punctual does not bypass CAPTCHA, queues, purchase limits, payment confirmation, website risk controls or access restrictions. It is intended for a single user operating their own visible, authorized browser session.
