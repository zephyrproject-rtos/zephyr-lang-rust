// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

// Pre-build code for zephyr module.

// This module makes the values from the generated .config available as conditional compilation.
// Note that this only applies to the zephyr module, and the user's application will not be able to
// see these definitions.  To make that work, this will need to be moved into a support crate which
// can be invoked by the user's build.rs.

// This builds a program that is run on the compilation host before the code is compiled.  It can
// output configuration settings that affect the compilation.

use std::io::{BufRead, BufReader, Write};
use std::env;
use std::fs::File;
use std::path::Path;
use std::process::{Command, Stdio};

use proc_macro2::TokenStream;
use regex::Regex;

use devicetree::{Augment, DeviceTree};

mod devicetree;

/// Export boolean Kconfig entries.  This must happen in any crate that wishes to access the
/// configuration settings.
pub fn export_bool_kconfig() {
    let dotconfig = env::var("DOTCONFIG").expect("DOTCONFIG must be set by wrapper");

    // Ensure the build script is rerun when the dotconfig changes.
    println!("cargo:rerun-if-env-changed=DOTCONFIG");
    println!("cargo-rerun-if-changed={}", dotconfig);

    let config_y = Regex::new(r"^(CONFIG_.*)=y$").unwrap();

    let file = File::open(&dotconfig).expect("Unable to open dotconfig");
    for line in BufReader::new(file).lines() {
        let line =  line.expect("reading line from dotconfig");
        if let Some(caps) = config_y.captures(&line) {
            println!("cargo:rustc-cfg={}", &caps[1]);
        }
    }
}

/// Capture bool, numeric and string kconfig values in a 'kconfig' module.
/// This is a little simplistic, and will make the entries numeric if they look like numbers.
/// Ideally, this would be built on the types of the values, but that will require more
/// introspection.
pub fn build_kconfig_mod() {
    let dotconfig = env::var("DOTCONFIG").expect("DOTCONFIG must be set by wrapper");
    let outdir = env::var("OUT_DIR").expect("OUT_DIR must be set");

    // The assumption is that hex values are unsigned, and decimal are signed.
    let config_hex = Regex::new(r"^(CONFIG_.*)=(0x[0-9a-fA-F]+)$").unwrap();
    let config_int = Regex::new(r"^(CONFIG_.*)=(-?[1-9][0-9]*)$").unwrap();
    // It is unclear what quoting might be used in the .config.
    let config_str = Regex::new(r#"^(CONFIG_.*)=(".*")$"#).unwrap();
    let gen_path = Path::new(&outdir).join("kconfig.rs");

    let mut f = File::create(&gen_path).unwrap();

    let file = File::open(&dotconfig).expect("Unable to open dotconfig");
    for line in BufReader::new(file).lines() {
        let line = line.expect("reading line from dotconfig");
        if let Some(caps) = config_hex.captures(&line) {
            writeln!(&mut f, "#[allow(dead_code)]").unwrap();
            writeln!(&mut f, "pub const {}: usize = {};",
                &caps[1], &caps[2]).unwrap();
        } else if let Some(caps) = config_int.captures(&line) {
            writeln!(&mut f, "#[allow(dead_code)]").unwrap();
            writeln!(&mut f, "pub const {}: isize = {};",
                &caps[1], &caps[2]).unwrap();
        } else if let Some(caps) = config_str.captures(&line) {
            writeln!(&mut f, "#[allow(dead_code)]").unwrap();
            writeln!(&mut f, "pub const {}: &'static str = {};",
                &caps[1], &caps[2]).unwrap();
        }
    }
}

/// Parse the finalized DTS file, generating the Rust devicetree file.
pub fn build_dts() {
    let zephyr_dts = env::var("ZEPHYR_DTS").expect("ZEPHYR_DTS must be set");
    let outdir = env::var("OUT_DIR").expect("OUT_DIR must be set");
    let gen_include = env::var("BINARY_DIR_INCLUDE_GENERATED")
        .expect("BINARY_DIR_INCLUDE_GENERATED");
    let builddir = env::var("BUILD_DIR").expect("BUILD_DIR");

    let augments = env::var("DT_AUGMENTS").expect("DT_AUGMENTS must be set");
    let augments: Vec<String> = augments.split_whitespace().map(String::from).collect();

    // Make sure that cargo knows to run if this changes, or any file mentioned changes.
    println!("cargo:rerun-if-env-changed=DT_AUGMENTS");
    for name in &augments {
        println!("cargo:rerun-if-changed={}", name);
    }

    let mut augs = Vec::new();
    for aug in &augments {
        // println!("Load augment: {:?}", aug);
        let mut aug = devicetree::load_augments(aug).expect("Loading augment file");
        augs.append(&mut aug);
    }
    // For now, just print it out.
    // println!("augments: {:#?}", augs);
    let augs: Vec<_> = augs
        .into_iter()
        .map(|aug| Box::new(aug) as Box<dyn Augment>)
        .collect();

    let generated = format!("{}/devicetree_generated.h", gen_include);
    let dt = DeviceTree::new(&zephyr_dts, generated);

    let out_path = Path::new(&outdir).join("devicetree.rs");
    let mut out = File::create(&out_path).expect("Unable to create devicetree.rs");

    let tokens = dt.to_tokens(&augs);
    if has_rustfmt() {
        write_formatted(out, tokens);
    } else {
        writeln!(out, "{}", tokens).unwrap();
    };

    // Output all of the node names in the discovered tree.
    let all_nodes_path = Path::new(&builddir)
        .join("rust")
        .join("all-dt-nodes.txt");
    let mut out = File::create(&all_nodes_path).expect("Unable to create all-dt-nodex.txt");
    dt.output_node_paths(&mut out).expect("Unable to write to all-dt-nodes.txt");
}

/// Generate cfg directives for each of the nodes in the generated device tree.
///
/// This assumes that build_dts was already run by the `zephyr` crate, which should happen if this
/// is called from a user application.
pub fn dt_cfgs() {
    let builddir = env::var("BUILD_DIR").expect("BUILD_DIR");
    let path = Path::new(&builddir)
        .join("rust")
        .join("all-dt-nodes.txt");
    for line in BufReader::new(File::open(&path).expect("Unable to open all-dt-nodes")).lines() {
        let line = line.expect("Error reading line from all-dt-nodes");
        println!("cargo:rustc-cfg=dt=\"{}\"", line);
    }
}

/// Determine if `rustfmt` is in the path, and can be excecuted. Returns false on any kind of error.
pub fn has_rustfmt() -> bool {
    match Command::new("rustfmt")
        .arg("--version")
        .status()
    {
        Ok(st) if st.success() => true,
        _ => false,
    }
}

/// Attempt to write the contents to a file, using rustfmt. If there is an error running rustfmt,
/// print a warning, and then just directly write the file.
fn write_formatted(file: File, tokens: TokenStream) {
    let mut rustfmt = Command::new("rustfmt")
        .args(["--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(file)
        .stderr(Stdio::inherit())
        .spawn()
        .expect("Failed to run rustfmt");
    // TODO: Handle the above failing.

    let mut stdin = rustfmt.stdin.as_ref().expect("Stdin should have been opened by spawn");
    writeln!(stdin, "{}", tokens).expect("Writing to rustfmt");

    match rustfmt.wait() {
        Ok(st) if st.success() => (),
        _ => panic!("Failure running rustfmt"),
    }
}
