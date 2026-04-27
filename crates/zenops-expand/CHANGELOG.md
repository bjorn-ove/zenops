# Changelog

## [0.5.2] - 2026-04-24

### Added
- Optional `schemars` feature that implements `JsonSchema` for `ExpandStr`, letting downstream crates include it in auto-generated JSON Schema bundles.

## [0.5.1] - 2026-04-22

### Changed
- Publish `repository` metadata so crates.io links back to the GitHub repo.

## [0.5.0] - 2026-04-22

### Changed
- `ExpandLookup::write_value` now takes `&mut dyn fmt::Write` instead of `&mut impl fmt::Write`, making the trait dyn-compatible so `&dyn ExpandLookup` and `[&dyn ExpandLookup; N]` chains can be used directly. Existing implementors of the trait must update their signatures accordingly.
- `ExpandStr::expand_to_string` and `ExpandStr::write_expanded` now accept `?Sized` lookups, so you can pass `&dyn ExpandLookup` without wrapping it.

## [0.4.3] - 2026-04-21

### Fixed
- Rustdoc build on docs.rs: switched the nightly feature gate from `doc_auto_cfg` to `doc_cfg` so the crate's documentation compiles again.
