# JSON Schema

`zenops schema` writes a single JSON document to stdout with two schemas inside `schemas.*`:

- `schemas.output_event` — every line of the NDJSON stream emitted by `-o json` (from `apply`, `status`, `pkg`, `doctor`, `init`).
- `schemas.config` — the TOML [`config.toml`](config.md) structure.

The bundle carries a `zenops_version` field and a `$id` that embeds the same
version. The schema shape is part of the zenops crate's public API and follows
the same SemVer promise as the crate version: breaking changes require a major
bump, additive changes a minor bump.

## Editor autocomplete for `config.toml`

Point [taplo](https://taplo.tamasfe.dev/) (used by Even Better TOML in VS Code,
and by the `taplo` LSP in Helix and Neovim) at the config schema. In
`~/.config/taplo/taplo.toml`:

```
[[rule]]
include = ["**/zenops/config.toml", "~/.config/zenops/config.toml"]
[rule.schema]
url = "file:///path/to/zenops-schema.json#/schemas/config"
```

Generate the schema file with `zenops schema > zenops-schema.json`.
