// SPDX-License-Identifier: Apache-2.0

use std::ffi::OsStr;
use std::path::Path;

/// Extend the builder before adding the built-in allowlists. An empty environment value is how
/// CMake represents the backwards-compatible, unconfigured case.
pub fn extend(
    mut builder: bindgen::Builder,
    path: Option<&OsStr>,
) -> anyhow::Result<bindgen::Builder> {
    let Some(path) = path.filter(|path| !path.is_empty()) else {
        return Ok(builder);
    };
    let path = Path::new(path);
    println!("cargo:rerun-if-changed={}", path.display());
    let config = zephyr_build::bindgen::BindgenConfig::load(path)?;
    for header in config.headers {
        println!("cargo:rerun-if-changed={}", header.display());
        builder = builder.header(header.to_str().expect("validated UTF-8 header"));
    }
    for pattern in config.functions {
        builder = builder.allowlist_function(pattern);
    }
    for pattern in config.types {
        builder = builder.allowlist_type(pattern);
    }
    for pattern in config.variables {
        builder = builder.allowlist_var(pattern);
    }
    Ok(builder)
}
