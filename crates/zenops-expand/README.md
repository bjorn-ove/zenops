# zenops-expand

Internal helper crate for [zenops](https://crates.io/crates/zenops). Provides
`ExpandStr`, a newtype around `String` that carries `${name}` placeholders and
must be passed through `.expand(lookup)` to produce a usable `String`. The type
deliberately does not implement `Display`, `AsRef<str>`, or `Deref<str>`, so the
compiler enforces expansion before the value reaches shell output or a
filesystem path.

Dual-licensed under MIT or Apache-2.0.
