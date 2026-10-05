# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/).

## [0.2.0] - 2026-10-05

### Added

- In-app auto-updater: the update banner offers "Update now", which downloads
  the signed release, verifies it against the bundled public key, installs it
  and restarts the app, with download progress. The "Release page" button
  stays available as a fallback. Releases now publish the installer, its
  `.sig` and a `latest.json` manifest.
- Backend errors for settings, backup/restore and updates are returned as
  stable error codes and shown in Thai or English, falling back to the
  original English message.
- Quota notification titles follow the configured language.

### Fixed

- The scheduler no longer deadlocks when thresholds or other settings change
  while a collection is running; settings reach the running loop through a
  watch channel.
- Restore runs in place without closing the connection pool, validates the
  backup file first and reloads the deduplication cache afterwards.
- The update check points at the correct GitHub repository.
- The opener permission is scoped so the release page can be opened.
- Notification thresholds are checked against the stored values, so warning
  always stays below critical.

### Changed

- Settings: sliders save once on release instead of on every step, the real
  saved thresholds are shown, and every control has an accessible label.
- Autostart launches (`--minimized`) stay in the tray.

### Security

- Removed unused capabilities and plugins (fs, http, shell, autostart) and
  tightened the Content Security Policy.
- Frontend log messages are size-limited and newline-escaped.
- CI runs `npm audit` and `cargo audit`; Dependabot is enabled.
- Updated dependencies for security advisories in h2, rustls and sqlx.
- The release workflow checks that `Cargo.toml` matches the tag version.

## [0.1.0]

- First public release.
