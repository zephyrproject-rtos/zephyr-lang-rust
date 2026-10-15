# zephyr-build Design

## Purpose

`zephyr-build` is the host-side build support crate for the Rust-on-Zephyr stack. It is not a
runtime crate. Its responsibility is to read build artifacts produced by Zephyr's CMake flow and
turn them into Rust-visible configuration and reflection data.

Today it serves two main roles:

- expose Zephyr Kconfig state to Rust build scripts and generated Rust modules;
- parse Zephyr's finalized devicetree outputs and generate Rust code plus `cfg(...)` markers from
  them.

This crate is consumed from `build.rs`, primarily by the `zephyr` crate itself, and in smaller ways
by downstream application crates.

## Source Layout

- `src/lib.rs`: public entry points used from build scripts.
- `src/devicetree.rs`: internal model for the parsed devicetree plus shared traversal logic.
- `src/devicetree/parse.rs`: pest-based parser for Zephyr's generated `zephyr.dts`.
- `src/devicetree/dts.pest`: grammar for the supported DTS subset.
- `src/devicetree/ordmap.rs`: parser for `devicetree_generated.h` to recover Zephyr node ordinals.
- `src/devicetree/output.rs`: Rust code generation and `cfg` emission for the parsed tree.
- `src/devicetree/augment.rs`: YAML-driven augmentation system that adds higher-level generated
  APIs to matching nodes.

## Position in the Build

The surrounding design is driven by Zephyr's CMake build.

1. CMake configures Zephyr and produces `.config`, `zephyr.dts`, and
   `include/generated/zephyr/devicetree_generated.h`.
2. CMake invokes Cargo with environment variables such as `DOTCONFIG`, `ZEPHYR_DTS`,
   `BINARY_DIR_INCLUDE_GENERATED`, and `DT_AUGMENTS`.
3. `zephyr/build.rs` calls:
   - `zephyr_build::export_bool_kconfig()`
   - `zephyr_build::build_kconfig_mod()`
   - `zephyr_build::build_dts()`
4. The `zephyr` crate then includes generated `kconfig.rs` and `devicetree.rs` from `OUT_DIR`.
5. Downstream application build scripts can also call lighter-weight helpers such as
   `export_bool_kconfig()` or `dt_cfgs()` to make the same Zephyr configuration visible in their own
   crates.

`zephyr-build` therefore acts as the bridge from Zephyr-generated files into Rust compile-time
state.

## Public API Model

The public surface is intentionally small and build-script oriented.

- `export_bool_kconfig()`: emits `cargo:rustc-cfg=CONFIG_...` lines for boolean Kconfig entries.
- `build_kconfig_mod()`: writes `OUT_DIR/kconfig.rs` containing Rust constants for numeric and
  string-like Kconfig values.
- `build_dts()`: parses Zephyr devicetree outputs and writes `OUT_DIR/devicetree.rs`.
- `dt_cfgs()`: emits `cargo:rustc-cfg=dt="..."` markers for devicetree node paths and selected
  phandle-style property paths.
- `has_rustfmt()`: helper used internally to decide whether generated code should be formatted.

This keeps the crate focused on generation rather than on providing a reusable runtime abstraction.

## Kconfig Pipeline

Kconfig support is file-based and intentionally simple.

### Boolean Export

`export_bool_kconfig()` reads the Zephyr `.config` file named by `DOTCONFIG` and matches lines of
the form `CONFIG_FOO=y`. For every such line it emits a Cargo directive:

- `cargo:rustc-cfg=CONFIG_FOO`

This lets both `zephyr` and user crates write conditional compilation against Zephyr Kconfig
booleans.

### Generated Value Module

`build_kconfig_mod()` also reads `DOTCONFIG`, but instead of emitting `cfg`s it generates a Rust
module file.

It recognizes:

- hex values, emitted as `usize`;
- decimal values, emitted as `isize`;
- quoted strings, emitted as `&str`.

The resulting file is included by `zephyr::kconfig`.

### Tradeoff

This Kconfig extraction is pragmatic rather than type-perfect. It infers Rust types from textual
shape instead of Kconfig schema information. That keeps the implementation simple and independent of
Zephyr internals, but it means the generated types are only an approximation of the original Kconfig
types.

## Devicetree Pipeline

The devicetree side is more ambitious. It combines two Zephyr outputs because neither one is
sufficient on its own.

### Inputs

- `zephyr.dts`: the finalized, merged devicetree in DTS syntax.
- `devicetree_generated.h`: generated C macros that include node ordinal information not available
  in the DTS text.

### Why Both Inputs Are Needed

The DTS file is easier to parse and preserves the hierarchical structure, properties, strings,
bytes, and phandle references in a readable form.

The generated header is used to recover the Zephyr "ORD" number for each node. Those ordinals are
important because Zephyr device instances are indexed by them, and downstream Rust code needs to be
able to connect a devicetree node to symbols such as `__device_dts_ord_N`.

### Internal Representation

`DeviceTree::new()` builds an in-memory tree of:

- `Node`: name, full path, route, ordinal, labels, properties, children, and parent link;
- `Property`: property name plus one or more parsed values;
- `Value`: words, bytes, strings, or phandles;
- `Word`: either numbers or phandle references.

Phandles are initially unresolved by label name and then fixed up in a second pass once all labels
have been collected.

## Devicetree Parsing Strategy

The DTS parser is deliberately narrow. It does not aim to implement arbitrary devicetree syntax.
Instead, it parses the subset expected from Zephyr's generated `zephyr.dts`.

Key characteristics of the parser:

- it uses a pest grammar instead of ad hoc string parsing;
- it supports labels, nested nodes, properties, strings, number lists, byte arrays, and phandles;
- it assumes the Zephyr-generated file shape rather than the full generality of DTS source files.

This is a conscious design choice. The crate is not a general-purpose DTS parser; it is a parser for
Zephyr's post-processed output.

## Ordinal Recovery

`ordmap.rs` scans `devicetree_generated.h` for pairs of macros:

- `DT_..._PATH`
- `DT_..._ORD`

It correlates them by macro stem and builds a `BTreeMap<String, usize>` from devicetree path to
ordinal.

That map is then consulted during DTS parsing so each parsed node can be assigned the Zephyr ordinal
that higher-level generated APIs depend on.

## Generated Rust Devicetree Model

`build_dts()` eventually emits `devicetree.rs`, which `zephyr::devicetree` includes.

The generated code mirrors the devicetree hierarchy as nested Rust modules:

- each node becomes a submodule;
- each node module exposes `ORD`;
- simple numeric properties become `pub const` values when possible;
- simple phandle properties become modules that re-export the referenced target node;
- more complex properties fall back to debug-style constants or raw-property output from
  augmentations.

The output is meant to make devicetree structure navigable from Rust source rather than to exactly
mirror every C macro in Zephyr's C headers.

## Augmentation System

The augmentation system is the most specialized part of the crate.

### Motivation

A plain structural reflection of the devicetree is not enough for ergonomic use. Some nodes need
higher-level generated APIs that know how to construct Rust device wrapper types from devicetree
metadata.

Rather than hard-coding all such logic directly into the parser, `zephyr-build` uses a YAML-driven
augmentation layer.

### Model

An augmentation file contains a list of `Augmentation` entries, each with:

- `rules`: predicates deciding which nodes match;
- `actions`: code generators run for matching nodes.

Rules currently include:

- property presence;
- compatible string checks at the current node or ancestor levels;
- boolean combinations;
- root matching.

Actions currently include:

- `Instance`: generate `get_instance()`-style constructors for Rust device wrapper types;
- `Labels`: generate a synthetic `labels` module mapping every devicetree label to its node module;
- `RawProperties`: emit static raw property data for runtime access.

### Data Sources for `Instance`

`Instance` generation can derive the underlying raw Zephyr device from:

- the current node itself;
- an ancestor node;
- a phandle property on the current node.

This allows the generated code to cover patterns such as:

- controller nodes that directly correspond to a Zephyr device;
- child nodes whose device comes from a parent controller;
- specifier nodes such as GPIO consumers that refer to a controller by phandle plus cell
  arguments.

### Current Configuration

The top-level `dt-rust.yaml` file in this repository uses this mechanism to add support for:

- GPIO controllers;
- GPIO-backed LED nodes;
- flash controllers;
- flash partitions;
- raw property reflection;
- global label aliases.

This keeps the generic parsing engine separate from board- and device-family-specific policy.

## `cfg(dt = "...")` Support

`dt_cfgs()` emits Cargo `rustc-cfg` directives for devicetree node paths so downstream crates can
conditionally compile against the presence of nodes.

Examples of the intended shape include paths such as:

- `dt = "aliases::led0"`
- `dt = "soc::gpio_40014000"`

It also emits label-based paths under `dt = "labels::..."`.

This is intentionally lighter-weight than generating the full `devicetree.rs` module for every
consumer crate. It gives applications a way to key conditional compilation off the same devicetree
without needing to parse it themselves.

## Formatting Strategy

When generating Rust source, `build_dts()` prefers to run `rustfmt` if it is available on the host.
If `rustfmt` is absent, it writes the token stream directly.

This is a convenience feature rather than a correctness requirement. The generated code should still
be usable without `rustfmt`, but formatted output is easier to inspect and document.

## Relationship to Other Crates

- `zephyr-sys` is about raw C ABI bindings.
- `zephyr-build` is about compile-time reflection from Zephyr build artifacts.
- `zephyr` consumes both:
  - `zephyr-sys` for runtime FFI;
  - `zephyr-build` for generated `kconfig` and `devicetree` modules.

Downstream application crates may also depend on `zephyr-build` directly in `build-dependencies`
when they need their own Kconfig- or devicetree-driven conditional compilation.

## Current Constraints and Tradeoffs

### Host-Build Assumptions

The crate assumes it is running inside a Zephyr/CMake-orchestrated build with the expected
environment variables already set. It is not designed to discover those artifacts independently.

### Narrow DTS Support

The pest grammar intentionally supports only the subset of DTS syntax seen in Zephyr's generated
output. That reduces complexity, but it means the parser is not suitable as a general DTS frontend.

### Approximate Kconfig Typing

Kconfig value typing is inferred from text, not from Kconfig metadata. This is good enough for
current use, but it is not semantically exact.

### Generated API Is Configuration-Specific

Both generated modules depend on the active board and configuration. The resulting Rust API is
therefore not globally stable across all Zephyr builds.

### Augmentation Policy Lives Outside Core Parsing

The augmentation system keeps policy out of the parser, which is good for modularity, but it also
means higher-level device support depends on external YAML configuration and convention-driven code
generation.

### Tight Coupling to `zephyr`

Some generated augmentation code assumes specific runtime types and constructors exist in the
`zephyr` crate, such as `crate::device::gpio::Gpio` and `crate::device::flash::FlashPartition`.
That makes the output practical for this repository, but not yet a general-purpose devicetree code
generation framework for arbitrary downstream crates.

## Extending the Crate

Typical extension points are:

1. expand Kconfig extraction rules in `src/lib.rs`;
2. broaden the DTS grammar if Zephyr's generated DTS requires new constructs;
3. extend the internal devicetree model if new property forms need richer handling;
4. add new `Rule`, `Action`, or `ArgInfo` variants for augmentation-driven code generation;
5. add new entries to `dt-rust.yaml` when supporting additional Zephyr device classes in the
   generated Rust API.

The current design expects most device-specific growth to happen through augmentation rules and YAML
configuration rather than by teaching the core parser about every device family.

## Non-Goals

From the current source, `zephyr-build` is not trying to:

- replace Zephyr's own CMake, Kconfig, or devicetree tooling;
- parse arbitrary DTS inputs from arbitrary ecosystems;
- provide a stable runtime API;
- generate fully generic device wrapper code independent of the `zephyr` crate;
- infer complete semantic meaning for all devicetree bindings without extra augmentation data.

## Summary

`zephyr-build` is the build-time reflection layer for Rust on Zephyr. It reads Zephyr-generated
configuration artifacts, turns Kconfig into Rust `cfg`s and constants, turns devicetree outputs into
a Rust module tree plus `cfg(dt = "...")` markers, and uses a YAML-driven augmentation system to add
the device-specific generated APIs that the higher-level `zephyr` crate depends on.
