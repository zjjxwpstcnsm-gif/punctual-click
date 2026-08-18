# Security Policy

## Supported versions

Punctual is currently in Alpha. Security fixes are applied to the latest published Alpha release only.

| Version | Supported |
|---|---|
| 0.1.0-alpha.5 | Yes |
| Earlier Alpha versions | No |

## Reporting a vulnerability

Please do not disclose a suspected vulnerability in a public issue before the maintainer has had a reasonable opportunity to investigate it.

Use GitHub's private vulnerability reporting feature for the repository when available. Include:

- affected Punctual version and operating system;
- browser and browser version;
- reproducible steps;
- expected and observed behavior;
- potential impact;
- logs with passwords, cookies, access tokens, personal data and payment information removed.

## Security boundaries

Punctual deliberately does not implement CAPTCHA bypass, queue bypass, anti-detection, credential collection, payment automation, multi-account abuse or access-control circumvention.

The application launches an isolated browser profile and stores task data locally. Users remain responsible for protecting the operating-system account, application data directory and logged-in browser session.

## Third-party runtimes

Release packages may contain a managed Chrome for Testing runtime and geckodriver. Their security updates are controlled by their upstream publishers. New Punctual releases should refresh bundled runtimes and record exact versions and checksums.
