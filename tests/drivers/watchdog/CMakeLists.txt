# SPDX-License-Identifier: Apache-2.0

cmake_minimum_required(VERSION 3.20.0)

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(watchdog_rust_test)

set(DT_AUGMENTS "${CMAKE_CURRENT_SOURCE_DIR}/dt-rust.yaml")
target_sources_ifdef(CONFIG_WATCHDOG app PRIVATE src/driver.c)
rust_cargo_application()
