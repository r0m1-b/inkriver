# Changelog

All notable changes to InkRiver are documented in this file.

## [0.3.1] - 2026-08-22

### Fixed

- Restored complete article sizing, image zoom, link handling, and text-size
  controls in packaged Linux applications.

### Security

- The isolated article frame bridge is authorized by its exact SHA-256 CSP
  hash instead of enabling arbitrary inline scripts.

## [0.3.0] - 2026-08-22

### Added

- Individual feed refresh actions from subscription management, with scoped
  collection, extraction, retention, and detailed error reporting.
- Cached website logos for `Other` feeds, discovered securely from feed
  metadata or public site icons and displayed throughout the application.
- Timeline actions to archive articles without opening them.
- An unread-only timeline filter, a smooth scroll-to-top action, and repeated
  favorite, archive, and source actions at the end of each article.
- Three persistent article text sizes and an image lightbox with keyboard and
  backdrop dismissal.

### Changed

- Action feedback now appears as fading, dismissible floating notifications
  without shifting the application layout.

### Security

- Website logos are downloaded with bounded size, duration, concurrency, and
  redirects; local and private destinations are rejected, and raster or SVG
  inputs are normalized to cached 64 × 64 PNG images.

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

[0.3.1]: https://github.com/r0m1-b/inkriver/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/r0m1-b/inkriver/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/r0m1-b/inkriver/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/r0m1-b/inkriver/releases/tag/v0.1.0
