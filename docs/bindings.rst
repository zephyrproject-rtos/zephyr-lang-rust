.. _rust_bindings:

Bindings to Zephyr for Rust
###########################

Zephyr is written in C, and its primary API is also made available to C.  It is written as a
mixture of types, function prototypes, inline functions, and macros.

Although Rust interfaces fairly easily with C at an ABI level, the compiler does not have any
support for generating the code necessary for these bindings. In order to call C code, the types
must be reproduced in Rust, function prototypes written in Rust, and often wrappers made for inline
functions and macros (or this functionality replicated in Rust).

This is a tedious, and error-prone process, and the end result would quickly get out of date with
the development of Zephyr.

The `bindgen`_ project seeks to address much of this issue by automatically generating bindings to C
code for Rust.  It makes use of ``libclang`` to parse and interpret a given set of headers, and
generate the Rust code needed to call this code.

.. _bindgen: https://github.com/rust-lang/rust-bindgen

Because the Zephyr headers use numerous conditional compilation macros, the bindings needed will be
very specialized to a given board, and even a given configuration.  To do this, the file
:file:`lib/rust/zephyr-sys/build.rs`, which ``cargo`` knows to run at build time, will
generate the bindings for the particular build of Zephyr being used by the current Rust application.

These bindings will be made available in the ``zephyr-sys`` crate, as well as under ``zephyr::raw``.

Using the Bindings
******************

In general, using direct bindings to C functions from Rust is a bit more difficult than calling them
from C.  Although all of these calls are considered "unsafe", and must be placed in an ``unsafe``
block, the Rust language has stricter constraints on what is allowed, even by unsafe code.  The
intent of supporting the Rust Language on Zephyr project is to allow full use of Zephyr, without
needing to resort to unsafe code, it is understandable that this implementation will be incomplete,
and some applications may require resorting to, what is sometimes referred to as "The Dark Arts".
The `Rustinomicon`_ is a good resource for those wishing to delve into this area.

.. _Rustinomicon: https://doc.rust-lang.org/nomicon/

In addition, the code in :file:`zephyr/lib/rust/zephyr` that provides the higher level abstractions
will primarily be implemented using these auto-generated bindings.

Generation
**********

The default headers and symbol patterns live in ``zephyr-sys/wrapper.h`` and
``zephyr-sys/build.rs``. Applications can extend both without modifying the module.
Pass an application-owned YAML file to CMake::

    zephyr_include_directories(include)
    zephyr_compile_definitions(APP_SCALE=3)
    rust_cargo_application(BINDGEN_CONFIG bindings.yaml)

For example, ``bindings.yaml`` can contain::

    headers:
      - include/api.h
      - include/constants.h
    functions:
      - app_.*
    types:
      - app_value
    variables:
      - APP_ANSWER

All four keys are optional lists; ``{}`` is a valid empty configuration. Unknown
keys, wrong value types, empty or invalid regular expressions, and missing headers
are errors. Patterns use bindgen's fully anchored Rust regular expression syntax.
The lists extend the built-in allowlists; they never replace them. Bindgen may
also generate types referenced by allowed functions. Only explicitly requested
patterns are added; there is no catch-all default.

The configuration path is relative to the application's CMake source directory.
Header paths are relative to the configuration file (absolute paths also work),
and headers are processed in their declared order after the standard wrapper.
Use header guards when headers include one another. Invalid C headers produce
Clang diagnostics during binding generation.

CMake passes the absolute configuration path as ``RUST_BINDGEN_CONFIG`` to all
Cargo invocations, including clippy and rustdoc, and records it in the generated
``sample-cargo-config.toml`` for direct Cargo/IDE use. Direct Cargo builds can also
set this variable to an absolute path, alongside the normal Zephyr build
environment. An unset or empty value preserves the default bindings. An
application ``build.rs`` cannot configure a dependency's earlier build script;
configuration must reach Cargo through this explicit CMake/environment boundary.

Use ``zephyr_include_directories`` and ``zephyr_compile_definitions`` before
``rust_cargo_application`` for application headers requiring include directories
or compiler definitions. These settings reach both bindgen through
``zephyr_interface`` and the C compiler building the wrappers. Settings applied
only to the ``app`` target do not reach bindgen. Custom non-inline functions
still need an implementation, for example through ``target_sources(app PRIVATE
src/api.c)``.

Static inline functions use the existing bindgen wrapper mechanism: the generated
C file includes the additional headers, is compiled by Zephyr, and is linked with
the application. Wrap function-like macros in explicitly allowed static inline
functions, or expose constant expressions as typed constants. Arbitrary C macros
are not automatically translated. See :file:`samples/custom-bindings` for a
complete example using functions, types, constants and multiple headers.

Cargo tracks the configuration, additional headers, transitively included headers,
Kconfig file, and relevant environment variables. CMake invokes Cargo on every
build, so editing a header or an allowlist regenerates bindings without a pristine
build. Bindgen's Cargo callbacks track transitive includes. Changing CMake include
paths or definitions requires the normal CMake regeneration.

Validation
**********

Run the host tests with::

    cargo test --manifest-path zephyr-build/Cargo.toml
    python3 tests/bindgen-host/run.py
    python3 tests/bindgen-host/cmake.py

The host tests require Python 3.11 or later, CMake, Ninja, libclang, a host C
compiler, and ``ar``. The generation test
checks default bindings, extensions, wrapper linking, errors, and independent
header/configuration rebuilds using the production configuration helper. It does
not require a Zephyr SDK. The custom-bindings sample runs under the existing
Twister sample CI for supported QEMU targets; for example::

    west build -b qemu_cortex_m3 samples/custom-bindings
    west build -t run
