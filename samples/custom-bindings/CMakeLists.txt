# SPDX-License-Identifier: Apache-2.0
cmake_minimum_required(VERSION 3.20.0)
find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(custom_rust_bindings)

# These settings reach both bindgen and the generated C wrappers. App-private target settings
# alone do not propagate to the zephyr_interface used by bindgen.
zephyr_include_directories(include)
zephyr_compile_definitions(APP_SCALE=3)
target_sources(app PRIVATE src/api.c)
rust_cargo_application(BINDGEN_CONFIG bindings.yaml)
