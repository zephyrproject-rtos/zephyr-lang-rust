// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

// Pre-build code for zephyr module.

// This module makes the values from the generated .config available as conditional compilation.
// Note that this only applies to the zephyr module, and the user's application will not be able to
// see these definitions.  To make that work, this will need to be moved into a support crate which
// can be invoked by the user's build.rs.

// This builds a program that is run on the compilation host before the code is compiled.  It can
// output configuration settings that affect the compilation.

use std::env;
use std::path::{Path, PathBuf};

use bindgen::Builder;

fn main() -> anyhow::Result<()> {
    // Determine which version of Clang we linked with.
    let version = bindgen::clang_version();
    println!("Clang version: {:?}", version);

    // Pass in the target used to build the native code.
    let target = env::var("TARGET")?;

    // And get the root of the zephyr tree.
    let zephyr_base = env::var("ZEPHYR_BASE")?;

    // Rustc uses some complex target tuples for the riscv targets, whereas clang uses other
    // options.  Fortunately, these variants shouldn't affect the structures generated, so just
    // turn this into a generic target.
    let target = if target.starts_with("riscv32") {
        "riscv32-unknown-none-elf".to_string()
    } else {
        target
    };

    // Likewise, do the same with RISCV-64.
    let target = if target.starts_with("riscv64") {
        "riscv64-unknown-none-elf".to_string()
    } else {
        target
    };

    let target_arg = format!("--target={}", target);

    // println!("includes: {:?}", env::var("INCLUDE_DIRS"));
    // println!("defines: {:?}", env::var("INCLUDE_DEFINES"));

    let out_path = PathBuf::from(env::var("OUT_DIR").expect("missing output directory"));
    let wrapper_path = PathBuf::from(env::var("WRAPPER_FILE").expect("missing wrapper file"));

    // Bindgen everything.
    let bindings = Builder::default()
        .clang_arg("-DRUST_BINDGEN")
        .clang_arg(format!("-I{}/lib/libc/minimal/include", zephyr_base))
        .clang_arg(&target_arg)
        .header(
            Path::new("wrapper.h")
                .canonicalize()
                .unwrap()
                .to_str()
                .unwrap(),
        )
        .use_core();

    let bindings = define_args(bindings, "-I", "INCLUDE_DIRS");
    let bindings = define_args(bindings, "-D", "INCLUDE_DEFINES");

    let bindings = bindings
        .wrap_static_fns(true)
        .wrap_static_fns_path(wrapper_path);

    let bindings = bindings.derive_copy(false);

    let bindings = bindings
        // Deprecated
        .blocklist_function("sys_clock_timeout_end_calc")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));

    let dotconfig = env::var("DOTCONFIG").expect("missing DOTCONFIG path");
    let options = zephyr_build::extract_kconfig_bool_options(&dotconfig)
        .expect("failed to extract kconfig boolean options");

    let bindings = bindings
        // Kernel
        .allowlist_item("E.*")
        .allowlist_item("K_.*")
        .allowlist_item("Z_.*")
        .allowlist_item("ZR_.*")
        .allowlist_function("k_.*")
        .allowlist_function("z_log.*")
        // Bluetooth
        .allowlist_item_if("CONFIG_BT_.*", || options.contains("CONFIG_BT"))
        .allowlist_function_if("bt_.*", || options.contains("CONFIG_BT"))
        // GPIO
        .allowlist_item_if("CONFIG_GPIO_.*", || options.contains("CONFIG_GPIO"))
        .allowlist_function_if("gpio_.*", || options.contains("CONFIG_GPIO"))
        // UART
        .allowlist_item_if("CONFIG_UART_.*", || options.contains("CONFIG_SERIAL"))
        .allowlist_function_if("uart_.*", || options.contains("CONFIG_SERIAL"))
        // Generate
        .generate()
        .expect("Unable to generate bindings");

    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");

    Ok(())
}

trait BuilderExt {
    type B;

    fn allowlist_function_if<P>(self, pattern: &str, pred: P) -> Self::B
    where
        P: FnOnce() -> bool;

    fn allowlist_item_if<P>(self, pattern: &str, pred: P) -> Self::B
    where
        P: FnOnce() -> bool;
}

impl BuilderExt for Builder {
    type B = Builder;

    fn allowlist_function_if<P>(self, pattern: &str, pred: P) -> Self::B
    where
        P: FnOnce() -> bool,
    {
        if pred() {
            return self.allowlist_function(pattern);
        }
        self
    }

    fn allowlist_item_if<P>(self, pattern: &str, pred: P) -> Self::B
    where
        P: FnOnce() -> bool,
    {
        if pred() {
            return self.allowlist_item(pattern);
        }
        self
    }
}

fn define_args(bindings: Builder, prefix: &str, var_name: &str) -> Builder {
    let text = env::var(var_name).expect("missing environment variable");
    let mut bindings = bindings;
    for entry in text.split(" ") {
        if entry.is_empty() {
            continue;
        }
        println!("Entry: {}{}", prefix, entry);
        let arg = format!("{}{}", prefix, entry);
        bindings = bindings.clang_arg(arg);
    }
    bindings
}
