# zenops-safe-relative-path

Internal helper crate for [zenops](https://crates.io/crates/zenops). Provides a
relative-path type (`SafeRelativePath`, `SafeRelativePathBuf`) that statically
prevents `..` traversal, plus a compile-time `srpath!()` macro for path
literals. See the main project for usage context.

Dual-licensed under MIT or Apache-2.0.
