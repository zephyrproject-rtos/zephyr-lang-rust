// SPDX-License-Identifier: Apache-2.0

//! Application-owned extensions to the `zephyr-sys` bindings.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use regex::Regex;
use serde::Deserialize;

/// Optional YAML configuration. Paths are relative to the configuration file, and lists retain
/// their declared order. Patterns use bindgen's fully anchored regular expression syntax.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindgenConfig {
    #[serde(default)]
    pub headers: Vec<PathBuf>,
    #[serde(default)]
    pub functions: Vec<String>,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub variables: Vec<String>,
}

impl BindgenConfig {
    /// Read and validate a configuration at an absolute path, without depending on the working
    /// directory. Header paths are returned as absolute paths.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.is_absolute() {
            bail!(
                "RUST_BINDGEN_CONFIG must be an absolute path: {}",
                path.display()
            );
        }
        let read = || -> anyhow::Result<Self> {
            let mut config: Self = serde_yaml_ng::from_str(&std::fs::read_to_string(path)?)?;
            for (category, patterns) in [
                ("functions", &config.functions),
                ("types", &config.types),
                ("variables", &config.variables),
            ] {
                for pattern in patterns {
                    if pattern.is_empty() {
                        bail!("{category}: empty patterns are not allowed");
                    }
                    Regex::new(&format!("^({pattern})$"))
                        .with_context(|| format!("{category}: invalid pattern {pattern:?}"))?;
                }
            }
            for header in &mut config.headers {
                let resolved = path.parent().unwrap().join(&*header);
                if !resolved.is_file() {
                    bail!("header is missing or is not a file: {}", resolved.display());
                }
                *header = resolved
                    .canonicalize()
                    .with_context(|| format!("cannot resolve header {}", resolved.display()))?;
                if header.to_str().is_none() {
                    bail!("header path must be UTF-8: {}", header.display());
                }
            }
            Ok(config)
        };
        read().with_context(|| format!("invalid bindgen configuration {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn load(text: &str, header: bool) -> anyhow::Result<BindgenConfig> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "zephyr-bindgen-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        if header {
            std::fs::write(dir.join("api.h"), "").unwrap();
        }
        let path = dir.join("bindings.yaml");
        std::fs::write(&path, text).unwrap();
        let result = BindgenConfig::load(&path);
        std::fs::remove_dir_all(dir).unwrap();
        result
    }

    #[test]
    fn defaults_are_empty() {
        let config = load("{}", false).unwrap();
        assert!(config.headers.is_empty());
        assert!(config.functions.is_empty());
        assert!(config.types.is_empty());
        assert!(config.variables.is_empty());
    }

    #[test]
    fn resolves_headers_and_preserves_order() {
        let config = load(
            "headers: [api.h, api.h]\nfunctions: ['foo_.*', bar]\ntypes: [Foo]\nvariables: [VALUE]",
            true,
        )
        .unwrap();
        assert!(config.headers.iter().all(|p| p.is_absolute()));
        assert_eq!(config.headers[0], config.headers[1]);
        assert_eq!(config.functions, ["foo_.*", "bar"]);
        assert_eq!(config.types, ["Foo"]);
        assert_eq!(config.variables, ["VALUE"]);
    }

    #[test]
    fn rejects_invalid_inputs_with_context() {
        for (input, message) in [
            ("unknown: []", "unknown field"),
            ("headers: api.h", "sequence"),
            ("functions: ['[']", "functions: invalid pattern"),
            ("types: ['']", "types: empty patterns"),
            ("variables: ['(']", "variables: invalid pattern"),
            ("headers: [api.h]", "header is missing"),
            ("headers: [", "invalid bindgen configuration"),
            ("functions: []\nfunctions: []", "duplicate field"),
        ] {
            let error = format!("{:#}", load(input, false).unwrap_err());
            assert!(error.contains(message), "{input}: {error}");
            assert!(error.contains("bindings.yaml"), "{error}");
        }
        assert!(BindgenConfig::load(Path::new("relative.yaml"))
            .unwrap_err()
            .to_string()
            .contains("absolute path"));
    }
}
