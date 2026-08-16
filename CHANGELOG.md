# Changelog

All notable changes to InkRiver are documented in this file.

## [0.2.0] - 2026-08-16

### Added

- Manual article archiving with confirmation and automatic retention of read,
  non-favorite articles older than 30 days.
- Safe extraction of complete pages for incomplete articles from `Other` feeds,
  with bounded downloads, retry cooldowns, and RSS fallback preservation.
- A permanent link to each article's original source.
- InkRiver application branding and native icons.

### Changed

- The article header and body now share one continuous scrolling area.
- Reader actions and the empty state use compact graphical controls and the
  InkRiver visual identity.

### Security

- Article-page extraction rejects local and private network destinations,
  validates every redirect, limits response size and duration, and disables
  implicit HTTP proxies.

## [0.1.0] - 2026-08-12

- First tagged Linux release with the Tauri reader, SQLite persistence,
  subscription management, read and favorite states, detailed feed errors, and
  external links opened in the system browser.

[0.2.0]: https://github.com/r0m1-b/inkriver/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/r0m1-b/inkriver/releases/tag/v0.1.0
