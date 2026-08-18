# Third-Party Notices

Punctual is licensed under the Apache License 2.0. Release packages can include third-party software that remains subject to its own license and terms.

## Chrome for Testing

Punctual uses Chrome for Testing as a managed fallback browser when no suitable installed browser is available.

- Upstream project: GoogleChromeLabs/chrome-for-testing
- Purpose: stable browser binary for testing and automation
- Upstream metadata/tooling license: Apache License 2.0
- Binary source: official Chrome for Testing download service

Google Chrome, Chrome for Testing, Chromium and related names and logos are trademarks of Google LLC. Punctual is not affiliated with or endorsed by Google. Any license or notice files included inside the downloaded browser distribution remain authoritative for that binary.

## geckodriver

Punctual can include geckodriver to automate Mozilla Firefox through W3C WebDriver.

- Upstream project: mozilla/geckodriver
- Purpose: Firefox WebDriver proxy
- License: Mozilla Public License 2.0

Mozilla, Firefox and related names and logos are trademarks of the Mozilla Foundation. Punctual is not affiliated with or endorsed by Mozilla.

## Rust dependencies

The Rust dependency graph contains software under Apache-2.0, MIT, MPL-2.0, BSD and other compatible licenses. The definitive list for a build is the dependency graph resolved by `Cargo.lock` together with each crate's distributed license metadata.

Before redistributing a modified build, review the licenses of all bundled crates and runtimes and preserve their required notices.
