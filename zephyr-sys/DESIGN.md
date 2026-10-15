# zephyr-sys Design

## Purpose

`zephyr-sys` is the raw FFI layer for the Rust support in this repository. Its job is to expose a
selected subset of Zephyr's C API to Rust with minimal policy and minimal hand-written code. The
crate is intentionally narrow:

- `zephyr-sys` generates `unsafe` bindings to Zephyr headers.
- `zephyr` builds safer and more idiomatic Rust wrappers on top of those bindings.
- CMake drives the build environment so the bindings match the active Zephyr board and Kconfig
  configuration.

The crate is `#![no_std]` and mostly consists of generated code included from `OUT_DIR`.

## Source Layout

- `build.rs`: runs `bindgen`, translates Cargo/CMake environment into clang arguments, and writes
  generated Rust bindings plus C wrappers for static inline functions.
- `wrapper.h`: seed header for bindgen. This chooses the Zephyr headers that define the reachable
  API surface and adds a few helper constants/functions that bindgen would not otherwise expose.
- `src/lib.rs`: includes generated bindings and applies a few post-generation trait impls needed by
  the rest of the workspace.
- `Cargo.toml`: keeps the crate small; the only runtime dependency is the generated code itself, and
  the only explicit dependencies are build-time ones.

## Build-Time Architecture

The binding pipeline is controlled outside the crate by the top-level module `CMakeLists.txt`.

1. CMake determines the Rust target triple from the active Zephyr architecture and ABI settings.
2. CMake collects Zephyr include directories and compile definitions from `zephyr_interface`.
3. CMake invokes Cargo with environment variables such as `ZEPHYR_BASE`, `INCLUDE_DIRS`,
   `INCLUDE_DEFINES`, `WRAPPER_FILE`, `DOTCONFIG`, and `ZEPHYR_DTS`.
4. Cargo runs `zephyr-sys/build.rs`.
5. `build.rs` runs `bindgen` over `wrapper.h`, emits Rust bindings to `OUT_DIR/bindings.rs`, and
   emits a generated C source file at the `WRAPPER_FILE` path for static inline wrappers.
6. CMake compiles that generated wrapper C file into the final application along with the Rust
   static library.

This means the crate does not attempt to discover a Zephyr installation itself. It relies on the
Zephyr CMake flow to provide a fully configured compilation environment.

## Environment Contract

`build.rs` currently assumes these variables are present:

- `TARGET`: Rust compilation target. The script normalizes some RISC-V triples to generic clang
  triples because libclang and Rust use different spellings.
- `ZEPHYR_BASE`: used to add the minimal libc include path.
- `INCLUDE_DIRS`: passed through as `-I...` arguments.
- `INCLUDE_DEFINES`: passed through as `-D...` arguments.
- `WRAPPER_FILE`: output path for bindgen-generated static-function wrappers.
- `OUT_DIR`: Cargo output directory for `bindings.rs`.

The crate fails fast with `unwrap()` or `?` when these values are missing. That fits the current
design: `zephyr-sys` is not intended to be built in isolation from the surrounding Zephyr build.

## API Selection Strategy

The crate does not expose "all of Zephyr". The generated surface is bounded in two ways.

First, `wrapper.h` constrains the parse root by including only a chosen set of headers:

- kernel APIs
- thread stack definitions
- GPIO
- logging
- Bluetooth
- flash
- IRQ support

Second, `build.rs` further constrains what bindgen emits with allowlists. The current allowlists are
pattern-based and focus on kernel, device, GPIO, flash, logging, Bluetooth, and selected constants
and generated device symbols.

This two-stage selection is important:

- adding a new header in `wrapper.h` expands the type universe bindgen can see;
- adding a new allowlist pattern in `build.rs` expands the symbols that are actually emitted.

Both usually need to change together when broadening coverage.

## Why `wrapper.h` Exists

`wrapper.h` is more than a convenience include file. It adapts Zephyr's headers into a shape that
bindgen can consume reliably.

### Configuration Alignment

It includes `zephyr/autoconf.h` first so parsing happens under the active Kconfig configuration.
Without this, many Zephyr declarations would not match the actual compiled system.

### Macro and ABI Fixups

It undefines `KERNEL` because building with `KERNEL` defined interferes with syscall-related
declarations in a way that is not useful for Rust binding generation.

It also defines `__SOFTFP__` for some Cortex-M soft-float configurations because CMSIS headers
expect that gcc-style define.

### Exporting Values Bindgen Misses

Bindgen only emits simple constants reliably. `wrapper.h` therefore materializes several macro-based
values as named `const` objects with a `ZR_` prefix, including stack layout constants, selected poll
constants, and several GPIO flags.

### Access to Macro-Only APIs

Some Zephyr functionality is provided as macros rather than callable functions. `wrapper.h` adds
small static inline adapters such as `zr_irq_lock()` and `zr_irq_unlock()` so the operations can be
bound as normal extern functions.

## Static Inline Wrapper Generation

Many Zephyr APIs are exposed as `static inline` functions in headers. Rust cannot link against those
directly because they do not exist as standalone symbols in a library.

`build.rs` uses `bindgen`'s `wrap_static_fns(true)` support to generate a companion C file at the
path provided by `WRAPPER_FILE`. That file contains exported wrapper functions such as
`k_uptime_get__extern(...)` which call the original inline definitions. CMake then compiles that C
file into the final build, making the functions linkable from Rust.

This is a core design choice: the crate depends on generated C shims, not only generated Rust code.

## Rust Crate Surface

`src/lib.rs` is deliberately thin.

- It applies crate-level lint suppressions that are normal for bindgen output.
- It includes `OUT_DIR/bindings.rs`.
- It reintroduces `Copy` and `Clone` for a small number of types where the workspace expects value
  semantics (`z_spinlock_key`, `k_timeout_t`).

The explicit `derive_copy!`/`derive_clone!` macros are a local correction after
`.derive_copy(false)`. The current approach is conservative: bindgen is told not to mark everything
as `Copy`, and the crate opts specific types back in by hand.

## Relationship to `zephyr`

The higher-level `zephyr` crate re-exports `zephyr-sys` as `zephyr::raw` and uses it to implement
safe wrappers in modules such as `zephyr::sys`, device abstractions, synchronization, threading,
timers, and logging.

That separation of responsibilities is clear in the current codebase:

- `zephyr-sys` owns fidelity to the C ABI.
- `zephyr` owns safety, ergonomics, and Rust-side policy.

## Current Constraints and Tradeoffs

### Configuration-Specific Bindings

The bindings are generated per build, not shipped as a fixed checked-in API. This is necessary for
Zephyr because header contents vary with board, architecture, and Kconfig, but it also means:

- docs and IDE support need the same build environment;
- the exact Rust API may vary across applications;
- reproducibility depends on the surrounding Zephyr configuration.

### Tight Coupling to CMake

The crate assumes the top-level Rust module CMake logic will provide all inputs. That keeps the
crate simple, but makes standalone Cargo usage impractical today.

### Pattern-Based Coverage

Allowlist patterns are easy to maintain, but they are approximate. A broad pattern can expose more
surface than intended, while a narrow pattern can silently omit required symbols until a consumer
hits a missing binding.

### Manual Constant Adapters

The `ZR_` constants are pragmatic and explicit, but they create a second namespace that higher-level
code must know about. This is acceptable for now, but it is a maintenance point whenever new macro
constants are needed.

### Selective Trait Repair

The manual `Copy`/`Clone` restoration in `src/lib.rs` is safe only as long as those C types remain
plain value types. This is a reasonable tradeoff, but additions should be reviewed carefully rather
than expanded mechanically.

## Extending the Crate

When adding more raw Zephyr APIs, the expected workflow is:

1. Add the necessary header include to `wrapper.h` if bindgen cannot already reach the declarations.
2. Add or adjust allowlist patterns in `build.rs`.
3. If the target API is a macro or a complex constant, add an adapter in `wrapper.h`.
4. If the target API is a `static inline` function, rely on the generated wrapper C output.
5. Add safe or thin wrappers in `zephyr`, not in `zephyr-sys`, unless the change is strictly about
   raw binding fidelity.

Longer term, the intended direction is to make the raw binding surface extensible from an
application that depends on `zephyr`, rather than requiring every new binding need to be added in
this crate ahead of time. That would let applications request additional headers, symbols, or
adapter glue as part of their own build.

That problem is currently unsolved. It is challenging because the binding set is generated inside
the Zephyr/CMake-driven build environment, while the `zephyr` crate is consumed from downstream
Cargo applications that do not currently have a supported mechanism to extend `zephyr-sys`
generation in a coordinated, reproducible way.

## Non-Goals

From the current source, `zephyr-sys` is not trying to:

- present an idiomatic Rust API;
- guarantee a stable, board-independent surface;
- replace the higher-level `zephyr` crate;
- build independently of the Zephyr CMake environment;
- hand-model the entire Zephyr API.

## Summary

`zephyr-sys` is a generated, configuration-specific FFI crate. Its design is centered on using
Zephyr's existing configured C build as the source of truth, then applying a small amount of
hand-written adaptation in `wrapper.h` and `src/lib.rs` to bridge the gaps that bindgen alone does
not cover.
