#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Exercise production config handling and Cargo invalidation, without a Zephyr SDK.

Run: python3 tests/bindgen-host/run.py
Requires Cargo, libclang, cc and ar. All generated inputs/builds live in a temporary directory.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

SOURCE = Path(__file__).resolve().parent
ROOT = SOURCE.parents[1]

with tempfile.TemporaryDirectory(prefix="zephyr-bindgen-") as tmp:
    tmp = Path(tmp)
    fixture = tmp / "fixture"
    shutil.copytree(SOURCE, fixture, ignore=shutil.ignore_patterns("target", "Cargo.lock"))
    manifest = fixture / "Cargo.toml"
    manifest.write_text(manifest.read_text().replace("../../zephyr-build", str(ROOT / "zephyr-build")))
    build = fixture / "build.rs"
    build.write_text(build.read_text().replace("../../zephyr-sys/bindgen_config.rs", str(ROOT / "zephyr-sys/bindgen_config.rs")))
    inputs = tmp / "application with spaces"
    inputs.mkdir()
    config = inputs / "bindings.yaml"
    header = inputs / "api.h"
    nested = inputs / "nested.h"
    nested.write_text("#define APP_VALUE 12\n")
    header.write_text('#pragma once\n#include <nested.h>\ntypedef struct { int value; } app_type;\n'
                      'static inline int app_answer(void) { return APP_VALUE + APP_DEFINE; }\n'
                      'int unlisted_api(void);\n')
    second = inputs / "second.h"
    second.write_text("#pragma once\n#define SECOND_VALUE 91\n")
    env = dict(os.environ, TEST_INCLUDE_DIR=str(inputs))
    env.pop("RUST_BINDGEN_CONFIG", None)

    def run(expect=None):
        result = subprocess.run(["cargo", "run", "--manifest-path", str(manifest)], env=env,
                                text=True, capture_output=True)
        if expect:
            assert result.returncode, result.stdout
            assert expect in result.stderr, result.stderr
            return
        assert result.returncode == 0, result.stderr
        generated = list((fixture / "target" / "debug" / "build").glob("bindgen-host-test-*/out/bindings.rs"))
        assert len(generated) == 1, generated
        text = generated[0].read_text()
        assert "k_existing" in text and "ZR_EXISTING" in text
        assert "hidden_function" not in text and "unlisted_api" not in text
        return text

    assert "app_answer" not in run()
    env["RUST_BINDGEN_CONFIG"] = ""
    assert "app_answer" not in run()
    env["RUST_BINDGEN_CONFIG"] = str(config)
    config.write_text("headers: [api.h]\nfunctions: [app_answer]\ntypes: [app_type]\nvariables: [APP_VALUE]\n")
    text = run()
    assert all(symbol in text for symbol in ["app_answer", "app_type", "APP_VALUE"])
    # Call both old and application inline wrappers: verifies compilation and linking.
    main = fixture / "src" / "main.rs"
    main.write_text(main.read_text().replace("fn main() {", "fn main() {\n    assert_eq!(unsafe { app_answer() }, APP_VALUE as i32 + 7);"))
    run()
    # Change a transitively included header without changing the manifest or Rust sources.
    nested.write_text("#define APP_VALUE 23\n")
    assert "APP_VALUE: u32 = 23" in run()
    # Independently change the allowlist (and add a second header).
    config.write_text(config.read_text() + "# new bindings\n")
    config.write_text(config.read_text().replace("[api.h]", "[api.h, second.h]")
                      .replace("[APP_VALUE]", "[APP_VALUE, SECOND_VALUE]"))
    assert "SECOND_VALUE" in run()
    header.write_text(header.read_text() + "\n#define DIRECT_VALUE 33\n")
    config.write_text(config.read_text().replace("SECOND_VALUE]", "SECOND_VALUE, DIRECT_VALUE]"))
    assert "DIRECT_VALUE: u32 = 33" in run()
    header.write_text(header.read_text().replace("33", "34"))
    assert "DIRECT_VALUE: u32 = 34" in run()
    for value, error in [("unknown: []", "unknown field"), ("functions: ['[']", "invalid pattern"),
                         ("headers: [missing.h]", "header is missing")]:
        config.write_text(value)
        run(error)
    header.write_text("#error deliberate invalid application header\n")
    config.write_text("headers: [api.h]\n")
    run("deliberate invalid application header")
    env["RUST_BINDGEN_CONFIG"] = "relative.yaml"
    run("must be an absolute path")
    # Compile the real sample headers, including the explicit macro wrapper and typed constant.
    sample = ROOT / "samples" / "custom-bindings"
    env["RUST_BINDGEN_CONFIG"] = str(sample / "bindings.yaml")
    env["TEST_INCLUDE_DIR"] = str(sample / "include")
    main.write_text((SOURCE / "src/main.rs").read_text().replace(
        "fn main() {", "fn main() {\n    assert_eq!(unsafe { app_scaled(14) }, 42);\n    assert_eq!(APP_ANSWER, 42);"))
    text = run()
    assert all(symbol in text for symbol in ["app_add", "app_scaled", "app_value", "APP_ANSWER"])
    # Type-check the real zephyr-sys build script and library on the host. Substitute only
    # its Zephyr wrapper for a small C fixture; a full Zephyr build is covered by Twister.
    sys_fixture = tmp / "zephyr-sys"
    shutil.copytree(ROOT / "zephyr-sys", sys_fixture,
                    ignore=shutil.ignore_patterns("target", "Cargo.lock"))
    sys_manifest = sys_fixture / "Cargo.toml"
    sys_manifest.write_text(sys_manifest.read_text().replace("../zephyr-build", str(ROOT / "zephyr-build")))
    (sys_fixture / "wrapper.h").write_text((SOURCE / "base.h").read_text() +
        "typedef struct { int key; } z_spinlock_key;\n"
        "typedef struct { int ticks; } k_timeout_t;\n"
        "static inline z_spinlock_key k_test(k_timeout_t timeout) {\n"
        "    z_spinlock_key key = { timeout.ticks }; return key; }\n")
    dotconfig = tmp / "dotconfig"
    dotconfig.write_text("")
    sys_env = dict(env, ZEPHYR_BASE=str(tmp), DOTCONFIG=str(dotconfig),
                   INCLUDE_DIRS=str(sample / "include"), INCLUDE_DEFINES="APP_SCALE=3",
                   WRAPPER_FILE=str(tmp / "sys-wrapper.c"), CARGO_TARGET_DIR=str(fixture / "target"))
    result = subprocess.run(["cargo", "check", "--manifest-path", str(sys_manifest)],
                            env=sys_env, text=True, capture_output=True)
    assert result.returncode == 0, result.stderr
    print("Bindgen host generation, wrapper linking, diagnostics and rebuild tests passed")
