// SPDX-License-Identifier: Apache-2.0

#[path = "../../zephyr-sys/bindgen_config.rs"]
mod bindgen_config;

use std::{env, path::PathBuf, process::Command};

fn main() -> anyhow::Result<()> {
    println!("cargo:rerun-if-env-changed=RUST_BINDGEN_CONFIG");
    println!("cargo:rerun-if-env-changed=TEST_INCLUDE_DIR");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("base.h");
    let builder = bindgen::Builder::default()
        .header(base.to_str().unwrap())
        .clang_arg("-DAPP_DEFINE=7")
        .clang_arg("-DAPP_SCALE=3")
        .clang_arg(format!("-I{}", env::var("TEST_INCLUDE_DIR")?))
        .allowlist_function("k_.*")
        .allowlist_var("ZR_.*")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));
    let bindings = bindgen_config::extend(builder, env::var_os("RUST_BINDGEN_CONFIG").as_deref())?
        .wrap_static_fns(true)
        .wrap_static_fns_path(out.join("wrapper.c"))
        .generate()?;
    bindings.write_to_file(out.join("bindings.rs"))?;
    assert!(Command::new("cc")
        .args(["-DAPP_DEFINE=7", "-DAPP_SCALE=3", "-I"])
        .arg(env::var("TEST_INCLUDE_DIR")?)
        .arg("-c")
        .arg(out.join("wrapper.c"))
        .arg("-o")
        .arg(out.join("wrapper.o"))
        .status()?
        .success());
    assert!(Command::new("ar")
        .arg("crs")
        .arg(out.join("libwrapper.a"))
        .arg(out.join("wrapper.o"))
        .status()?
        .success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=wrapper");
    Ok(())
}
