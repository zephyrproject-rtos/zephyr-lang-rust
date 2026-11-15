#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Check CMake's Cargo environment and generated IDE configuration without the SDK."""
from pathlib import Path
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[2]
with tempfile.TemporaryDirectory(prefix="zephyr-cmake-") as tmp:
    tmp = Path(tmp)
    source = tmp / "application with spaces"
    source.mkdir()
    (source / "bindings.yaml").write_text("{}\n")
    (source / "dummy.c").write_text("/* empty */\n")
    template = '''cmake_minimum_required(VERSION 3.20)
project(bindgen_cmake_test C)
set(CONFIG_CPU_CORTEX_M TRUE)
set(CONFIG_CPU_CORTEX_M3 TRUE)
add_library(app STATIC dummy.c)
add_library(linker INTERFACE)
add_library(zephyr_interface INTERFACE)
foreach(name offsets_h syscall_list_h_target driver_validation_h_target kobj_types_h_target)
  add_custom_target(${name})
endforeach()
include("@ROOT@/CMakeLists.txt")
rust_cargo_application(@ARGS@)
'''.replace("@ROOT@", str(ROOT))
    for name, args in [("default", ""), ("custom", "BINDGEN_CONFIG bindings.yaml")]:
        (source / "CMakeLists.txt").write_text(template.replace("@ARGS@", args))
        build = tmp / name
        subprocess.run(["cmake", "-G", "Ninja", "-S", str(source), "-B", str(build)],
                       check=True, capture_output=True, text=True)
        config = tomllib.loads((build / "rust/sample-cargo-config.toml").read_text())
        expected = str(source / "bindings.yaml") if name == "custom" else ""
        assert config["env"]["RUST_BINDGEN_CONFIG"] == expected
        ninja = (build / "build.ninja").read_text()
        assert ninja.count("RUST_BINDGEN_CONFIG=") == 3  # build, clippy, rustdoc
        assert str(build / "rust/wrapper.c") in ninja
    (source / "CMakeLists.txt").write_text(template.replace("@ARGS@", "BINDGEN_CONFIG missing.yaml"))
    result = subprocess.run(["cmake", "-S", str(source), "-B", str(tmp / "invalid")],
                            capture_output=True, text=True)
    assert result.returncode and "Bindgen configuration does not exist" in result.stderr
print("CMake Cargo environment and IDE configuration tests passed")
