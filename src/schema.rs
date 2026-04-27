//! `zenops schema` — dump JSON Schema for every structured surface.
//!
//! Emits a single bundle to stdout containing the schema for the NDJSON event
//! stream (`-o json` output of apply/status/pkg/doctor/init) and the schema
//! for the TOML config input. The zenops crate version is embedded in the
//! bundle; the schema shape is treated as part of the crate's public API and
//! versioned under the same SemVer promise.

use std::io::Write;

use schemars::schema_for;
use serde_json::json;

use crate::{config::StoredConfig, error::Error, output::Event};

/// Crate version embedded in every emitted schema bundle. Read at compile
/// time so `cargo install`'d binaries report the version they were built
/// from.
pub const ZENOPS_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Serialise the schema bundle (output events + config input) as
/// pretty-printed JSON to `stdout`. One trailing newline.
pub fn run(stdout: &mut dyn Write) -> Result<(), Error> {
    let bundle = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": format!("https://github.com/bjorn-ove/zenops/schema/v{ZENOPS_VERSION}"),
        "title": "zenops schema bundle",
        "zenops_version": ZENOPS_VERSION,
        "schemas": {
            "output_event": schema_for!(Event),
            "config": schema_for!(StoredConfig),
        }
    });

    let mut rendered = serde_json::to_vec_pretty(&bundle).map_err(Error::SchemaEmit)?;
    rendered.push(b'\n');
    stdout.write_all(&rendered).map_err(Error::SchemaWrite)?;
    Ok(())
}
