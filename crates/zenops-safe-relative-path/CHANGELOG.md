# Changelog

## [0.5.3] - 2026-04-24

### Added
- Optional `schemars` feature that implements `JsonSchema` for `SafeRelativePath`, `SafeRelativePathBuf`, and `SinglePathComponent`, letting downstream crates include these types in auto-generated JSON Schema bundles.

## [0.5.2] - 2026-04-22

### Changed
- Publish `repository` metadata so crates.io links back to the GitHub repo.

## [0.5.1] - 2026-04-22

### Added
- `SafeRelativePath` now implements `serde::Serialize` directly on the owned type (previously only `&SafeRelativePath` did). Existing call sites that serialized a reference continue to work via serde's blanket `impl<T: Serialize> Serialize for &T`.

## [0.5.0] - 2026-04-21

### Added
- `SinglePathComponent::try_new` — fallible constructor that validates a string is a single path component with no traversal, usable outside of serde.
- `Error::NotASinglePathComponent(String)` variant, returned when a value is multi-component or contains traversal.

### Changed
- Adding `Error::NotASinglePathComponent` is a breaking change for downstream code that exhaustively matches `Error` without a wildcard arm.
