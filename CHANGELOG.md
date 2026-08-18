# Changelog

## 0.1.0-alpha.5 - 2026-08-17

### Added

- Automatic browser discovery on macOS, Windows and Linux.
- Chrome-first selection with the supported system default browser as the next priority.
- CDP support for Chrome, Edge, Brave, Arc, Vivaldi, Chromium and Opera.
- WebDriver adapters for macOS Safari and Mozilla Firefox.
- Automatic fallback across detected browsers when the preferred browser cannot be automated.
- Support for an app-bundled Chrome for Testing runtime as a no-install fallback.
- Browser name is recorded in execution result messages.

### Changed

- The header now displays the selected/connected browser instead of a generic Chromium status.
- Each browser family receives an isolated persistent profile; the legacy Chrome profile is preserved.
- Safari automation limitations and fallback decisions are surfaced directly in the application status.

## 0.1.0-alpha.4 - 2026-08-17

### Fixed

- Result verification now tracks browser targets created after the scheduled click.
- Direct `target=_blank` / `window.open()` result tabs and new browser windows are preferred over the original page when confirming success.
- New pages opened with `noopener` are still detected through the pre-click target snapshot.
- Transient `about:blank` popup state is ignored until the destination URL is committed.
- Result URLs and success evidence now explicitly identify when confirmation came from a newly opened tab/window.
- Existing unrelated tabs are excluded from result verification, reducing false positives.

## 0.1.0-alpha.3 - 2026-08-17

### Added

- Independent vertical scrolling for the task list, task details, editor and candidate list.
- One-click clipboard actions for task IDs, URLs, target names, execution results and execution logs.
- Minimum window dimensions and responsive wrapping for narrow-window layouts.

### Changed

- Long URLs, result messages, candidate context and log content now wrap instead of forcing panels off-screen.
- Editor columns, option rows, action bars and detail cards can wrap when horizontal space is limited.
- Header status text is constrained so it no longer expands the main layout beyond the window.

### Fixed

- Content below the visible viewport can now be reached with the mouse wheel or scrollbar.
- Important diagnostic values no longer require non-existent browser-style text selection to be copied.

## 0.1.0-alpha.2 - 2026-08-17

### Added

- GPUI real task editor for URL, millisecond local time, timezone and completion text.
- Automatic candidate selection and manual button-text validation.
- Browser highlight and candidate context/availability display.
- Dedicated Tokio engine thread with typed commands and events.
- Lazy visible Chromium session with an isolated user profile.
- Multi-task scheduler, startup recovery and per-task workers.
- Execution-log SQLite migration and atomic task/result persistence.
- Strict and wait-until-clickable execution modes.
- Browser workflow fixtures for duplicate, delayed, SPA and Shadow DOM cases.
- GitHub Actions jobs for core Rust, GPUI app and real Chromium fixtures.

### Changed

- A task cannot enter `Pending` until a target fingerprint has been verified.
- `Preparing` and `Armed` tasks safely recover to `Pending` after restart.
- Interrupted `Executing` tasks become `Uncertain` instead of being clicked again.
- The alpha.1 demonstration dashboard was replaced by the real create/edit workflow.
- Target selections are bound to the inspected URL; manual selections are also bound to the exact validated text.
- Tasks in `Executing` cannot be edited, rescheduled or deleted.
- Completion evidence is transition-based and compared with a baseline captured immediately before the native click.
- Targets are re-relocated and pre-scrolled at `T-1s`; the exact-deadline probe no longer performs layout-changing scroll work.
- Save/delete commands guard the `Executing` state both before and after worker cancellation.
- Duplicate `data-testid`, `id` or `name` attributes fall back to a selector that uniquely identifies the chosen element.

### Known limitations

- Rust compilation was not available in the delivery container; see `VALIDATION.md`.
- No iframe scanning, screenshot persistence or packaged desktop installer yet.

## 0.1.0-alpha.1 - 2026-08-17

- Initial domain, SQLite, browser scanner, target fingerprinting, timing primitives and GPUI dashboard skeleton.
