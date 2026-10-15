# zephyr Design

## Purpose

`zephyr` is the main Rust runtime crate for applications that run on top of the Zephyr RTOS. It is
the layer that turns:

- raw C bindings from `zephyr-sys`;
- build-time reflection from `zephyr-build`;
- Zephyr kernel and driver concepts;

into Rust APIs that are safer and more idiomatic, while still staying recognizably close to Zephyr.

The crate is `#![no_std]`. It is designed to support both minimal no-alloc systems and richer
systems with `CONFIG_RUST_ALLOC`.

## Design Goals

From the source, the crate is trying to balance several competing goals:

- preserve access to Zephyr concepts rather than hiding the RTOS behind an unrelated abstraction;
- move as much unsafety as possible into library internals;
- support static, linker-visible kernel objects in a way that matches Zephyr's object model;
- support targets that may lack native atomic instructions;
- allow generated build-specific reflection of Kconfig and devicetree state;
- optionally layer in allocation, timers, work queues, logging, and Embassy integration.

This is not a port of `std`, and it is not a purely "Rust-native RTOS". It is a Rust-facing Zephyr
support crate.

## High-Level Architecture

The crate is built in layers.

### 1. Build-Time Reflection

`build.rs` invokes `zephyr-build` to generate:

- `kconfig.rs`, included as `zephyr::kconfig`;
- `devicetree.rs`, included as `zephyr::devicetree`.

This makes the active Zephyr build configuration visible to Rust code.

### 2. Raw FFI Layer

`zephyr::raw` simply re-exports `zephyr-sys`.

This is the escape hatch for direct access to generated C bindings. It remains unsafe and close to
the underlying ABI.

### 3. Thin Safe Wrappers

`zephyr::sys` provides low-level wrappers over `zephyr-sys` that remove `unsafe` from common use
but intentionally preserve Zephyr semantics as much as possible.

Examples:

- `sys::sync::Semaphore`, `Mutex`, `Condvar`
- `sys::queue::Queue`
- `sys::thread`
- `sys::critical` for the `critical-section` crate integration

### 4. Higher-Level Rust APIs

Above `sys`, the crate builds more idiomatic Rust interfaces:

- `sync::Mutex`, `Condvar`, `SpinMutex`, channels, atomics, `Arc`
- `time::{Duration, Instant, Timeout}`
- typed device wrappers under `device`
- higher-level thread helpers in `thread`
- timers in `timer`
- work queues in `work`
- logging integration in `logging`

These are the intended main user-facing APIs.

## Module Map

The major modules break down as follows:

- `lib.rs`: crate root, feature gating, generated module inclusion, panic handler, raw re-export.
- `error.rs`: Zephyr errno-to-`Result` mapping.
- `time.rs`: Zephyr-oriented time types and timeout conversions.
- `object.rs`: kernel-object and fixed-address infrastructure, plus `kobj_define!`.
- `sys/`: safe but low-level wrappers over Zephyr primitives.
- `sync/`: higher-level synchronization primitives modeled after `std` and `crossbeam`.
- `device/`: typed wrappers around Zephyr devices and devicetree-generated constructors.
- `thread.rs`: higher-level thread-pool style and reusable-thread support.
- `printk.rs`: formatting-to-console support.
- `logging/`: `log` crate backend integration.
- `timer.rs`: higher-level wrapper over `k_timer`.
- `work.rs`: work queues, work items, and signal support.
- `embassy/`: feature-gated Embassy executor/time integration.
- `alloc_impl.rs`: global allocator when `CONFIG_RUST_ALLOC` is enabled.
- `simpletls.rs`: lightweight thread-local mapping helper used by work queues.

## Generated Configuration Reflection

The crate directly includes build-generated modules:

- `zephyr::kconfig`
- `zephyr::devicetree`

This is a central part of the design, not an add-on. Many APIs depend on build-time configuration,
and device access is meant to flow through generated devicetree modules such as label aliases and
instance constructors.

The tradeoff is that part of the crate's visible API is configuration-specific. Docs and code
completion are therefore most useful when generated for a concrete build.

## Relationship to `zephyr-sys`

`zephyr-sys` owns raw ABI fidelity.

`zephyr` uses it in three ways:

- direct re-export as `zephyr::raw`;
- thin wrapping in `zephyr::sys`;
- implementation detail beneath higher-level abstractions.

The intent is clear in the source:

- use `zephyr::raw` when direct bindings are unavoidable;
- prefer `zephyr::sys` for close-to-Zephyr but safe access;
- prefer `sync`, `device`, `timer`, `work`, and related modules for regular Rust code.

## Kernel Object Model

One of the biggest design choices in the crate is how it handles Zephyr kernel objects.

Zephyr expects many objects to be:

- statically allocated;
- visible to the kernel object system and linker sections;
- not moved after initialization;
- in some cases accessible from userspace only if specially declared.

Rust's normal ownership and move semantics do not line up cleanly with that model, so the crate
implements explicit infrastructure in `object.rs`.

### `StaticKernelObject`

`StaticKernelObject<T>` is the abstraction for statically declared Zephyr kernel objects. It tracks
single initialization with an atomic state machine and is paired with the `Wrapped` trait so each
raw Zephyr object can expose a more usable Rust wrapper.

This is how types like:

- `StaticMutex`
- `StaticCondvar`
- `StaticThread`
- `StaticStoppedTimer`

are made usable from Rust.

### `kobj_define!`

The `kobj_define!` macro is the user-facing declaration mechanism for static Zephyr kernel objects.
It emits the linker decorations and storage layout expected by Zephyr, including special handling
for thread stacks and object arrays.

This is a core integration point with Zephyr's build and object registration model.

### `ZephyrObject`

For dynamically owned but fixed-address objects, `ZephyrObject<T>` provides delayed initialization
plus runtime move detection. The object starts zeroed and uninitialized, is initialized on first
use inside a critical section, and records the address at which initialization occurred. If it is
moved afterward, the crate panics.

This is the crate's way of reconciling Rust move semantics with Zephyr objects that become
self-referential or address-sensitive after initialization.

### `Fixed`

`Fixed<T>` abstracts over:

- static kernel-object-backed storage;
- pinned owned storage when allocation is available.

This allows APIs like timers and work queues to support both static and dynamically allocated usage
without requiring separate code paths everywhere.

## Time Model

`time.rs` defines Zephyr-oriented time types using `fugit` rather than trying to mirror
`std::time` exactly.

Important choices:

- `Duration` and `Instant` are parameterized in system ticks, not nanoseconds;
- `Timeout` wraps Zephyr's `k_timeout_t`;
- conversions preserve Zephyr's special timeout semantics such as `K_FOREVER`, `K_NO_WAIT`, and
  absolute timeouts on 64-bit timeout builds;
- the implementation is tied directly to `CONFIG_SYS_CLOCK_TICKS_PER_SEC`.

This makes the time model efficient and Zephyr-native, but also intentionally configuration-aware.

## Error Model

`error.rs` is intentionally small. Zephyr APIs that return negative errno values are mapped to:

- `Error(u32)`
- `Result<T>`

The crate does not currently try to model errnos as a rich enum. That keeps wrappers cheap and
simple, but means error values remain mostly numeric.

## `sys` Layer

`zephyr::sys` is the "safe but still Zephyr-shaped" layer.

It provides:

- constants like `K_FOREVER` and `K_NO_WAIT`;
- direct wrappers over kernel primitives;
- a `critical-section` implementation based on `irq_lock()`/`irq_unlock()`;
- thread and queue wrappers;
- synchronization wrappers around `k_sem`, `k_mutex`, and `k_condvar`.

This layer generally avoids imposing Rust ownership policy beyond what is needed to make the calls
memory-safe.

That separation matters:

- `sys` is where direct Zephyr semantics are preserved;
- `sync` and other higher-level modules are where Rust-specific structure is added.

## Synchronization Design

Synchronization is split into two layers.

### Low-Level Primitives

`sys::sync` exposes wrappers around Zephyr kernel objects. These are useful when the underlying
Zephyr primitive is already a reasonable API on its own, especially `Semaphore`.

### Higher-Level Rust Primitives

`sync` builds Rust-style wrappers on top:

- `sync::Mutex<T>` stores protected data alongside a Zephyr mutex and returns RAII guards;
- `sync::Condvar` mirrors the `std`-style pairing with `MutexGuard`;
- `sync::SpinMutex<T>` provides IRQ-safe protection using `k_spinlock`;
- `sync::channel` implements close-to-`crossbeam-channel` semantics on top of `k_queue`;
- `sync::atomic` re-exports `portable-atomic`;
- `sync::Arc` and related types come from `portable-atomic-util`.

Notable design consequences:

- the crate supports targets without native atomics by leaning on `portable-atomic` plus the
  `critical-section` implementation;
- `SpinMutex` exists specifically because Zephyr mutexes are not IRQ-safe;
- channels deliberately inherit Zephyr queue limitations, including incomplete drop/disconnect
  semantics.

## Threads

Thread support also has two layers.

### `sys::thread`

This layer models the traditional Zephyr approach:

- statically allocated `k_thread` plus stack;
- explicit creation and startup;
- optional closure-based spawn when allocation is available;
- helper macros and types for stack declarations.

### `thread`

This higher-level module adds a reusable-thread model based on thread pools and shared startup data.
It is designed to support the `#[zephyr::thread]` proc macro and provide a friendlier interface for
declaring Rust threads with arguments, start control, joining, and reuse after termination.

Overall, the design keeps Zephyr's thread model visible rather than trying to replace it with a
green-thread abstraction.

## Devices and Devicetree Integration

The device model is tightly coupled to generated devicetree code.

### Generated Entry Points

`zephyr-build` generates devicetree modules that call constructors such as:

- `crate::device::gpio::Gpio::new(...)`
- `crate::device::gpio::GpioPin::new(...)`
- `crate::device::flash::FlashController::new(...)`
- `crate::device::flash::FlashPartition::new(...)`

### `Unique`

The `device::Unique` helper enforces one-time construction of wrapper instances for device-tree
generated objects. This is a pragmatic safeguard around shared static Zephyr devices.

### Current Scope

The current hand-written device wrappers are focused on:

- GPIO controllers and GPIO pins;
- flash controllers and flash partitions.

GPIO also contains optional async waiting support under the `async-drivers` feature.

This design makes device support extensible through devicetree augmentation plus hand-written Rust
wrapper types, rather than requiring every device to be exposed manually at the raw FFI layer.

## Logging and Console Output

The crate has two related but distinct output paths.

### `printk`

`printk!` and `printkln!` format Rust arguments and emit them through Zephyr's console path via
`k_str_out`. The implementation uses a small stack buffer and flushes in chunks.

### `logging`

`logging` integrates the `log` crate with Zephyr in a configuration-sensitive way:

- use `printk` when Zephyr logging is unavailable or minimal;
- use Zephyr's logging subsystem when full logging is enabled and allocation is available;
- otherwise install a no-op logger.

This keeps Rust logging aligned with Zephyr's logging configuration rather than inventing a
separate logging path.

## Allocation Strategy

Allocation is feature- and configuration-gated by `CONFIG_RUST_ALLOC`.

When enabled:

- `alloc_impl.rs` installs a global allocator backed by Zephyr `malloc`/`free`;
- modules such as `timer`, `work`, `simpletls`, channel internals, and closure-based thread spawn
  become available;
- `portable-atomic-util` facilities such as `Arc` are usable in practice.

The crate does not attempt to emulate full `std`; it provides `alloc` support only.

## Timers

`timer.rs` wraps `k_timer` into explicit state-typed Rust objects:

- `StoppedTimer`
- `SimpleTimer`
- `CallbackTimer`

The key design choice is that a running callback timer must be pinned because Zephyr stores callback
state inside the underlying timer object and invokes callbacks asynchronously from IRQ context.

The API reflects this by:

- separating stopped and running states;
- making callback mode explicitly pinned;
- stopping timers in `Drop` where safe to do so.

## Work Queues

`work.rs` wraps Zephyr work queues and related signaling primitives.

The current design is deliberately opinionated:

- only basic work queue support is implemented, despite Zephyr having additional work variants;
- work queues are started from a `WorkQueueBuilder`;
- work queues must never be dropped, because Zephyr provides no safe stop mechanism for their
  backing thread;
- a lightweight TLS map is used to associate current threads with work queues;
- `Signal` wraps `k_poll_signal` for event-style coordination.

This is one of the clearest places where the crate chooses explicit lifecycle restrictions instead
of pretending the underlying Zephyr primitive is easier to own than it really is.

## Embassy Integration

`embassy` is feature-gated and intentionally separate from the base runtime.

Current features:

- `executor-zephyr`: an Embassy executor that parks on a Zephyr semaphore;
- `time-driver`: a Zephyr-backed Embassy time driver.

The integration strategy is not to replace Zephyr scheduling, but to let async Rust executors run
on top of Zephyr threads and timers where that model is beneficial.

## Panic Handling

The crate defines the panic handler in `lib.rs`.

Current behavior:

- optionally print panic information with `printkln!` when `CONFIG_PRINTK` is enabled;
- transfer control to an external `rust_panic_wrap()` function provided elsewhere in the build.

This keeps panic behavior explicitly integrated with the surrounding Zephyr/C runtime.

## Important Constraints and Tradeoffs

### Build-Specific Surface

`kconfig` and `devicetree` are generated per build, so part of the crate's API is inherently
configuration-specific.

### Tight Zephyr Coupling

The crate is intentionally specific to Zephyr's object model, scheduler, devicetree, and build
artifacts. It is not aiming to be a portable embedded HAL.

### Partial Userspace Story

Some infrastructure explicitly does not support `CONFIG_USERSPACE` yet, including the
`critical-section` implementation.

### Lifecycle Gaps

Some Zephyr primitives do not offer the hooks needed for fully Rust-idiomatic lifecycle semantics.
Examples visible in the source:

- channels cannot implement proper disconnect/drop behavior;
- work queues cannot be safely dropped;
- some flash wrapper sharing is noted as not fully safe yet.

### No `std`

The crate provides selected higher-level facilities, but does not try to reproduce the full Rust
standard library API surface.

### Safety Is Layered, Not Absolute

The crate removes a large amount of `unsafe` from application code, but some APIs remain explicitly
unsafe where Zephyr semantics cannot honestly be represented as fully safe Rust.

## Extending the Crate

Based on the current structure, extensions generally fall into one of these buckets:

1. add or expand raw bindings in `zephyr-sys`;
2. expose them thinly in `zephyr::sys`;
3. build a higher-level Rust wrapper in `zephyr`;
4. add devicetree augmentation plus typed device wrappers when the feature is device-oriented;
5. gate optional subsystems behind Cargo features and/or Zephyr Kconfig when they depend on alloc
   or extra ecosystem crates.

The current design favors incremental growth in those layers rather than large monolithic APIs.

## Non-Goals

From the current source, `zephyr` is not trying to:

- be a general-purpose OS abstraction layer;
- hide Zephyr concepts behind unrelated Rust terminology;
- provide full `std` compatibility;
- make every Zephyr primitive look fully safe if its lifecycle semantics do not justify that;
- provide board-independent documentation for generated build-specific modules.

## Summary

`zephyr` is the main Rust support crate for running on Zephyr. Its design centers on a layered
approach:

- generated build reflection from `zephyr-build`;
- raw ABI access through `zephyr-sys`;
- thin safe wrappers in `sys`;
- higher-level Rust APIs for synchronization, devices, timers, work, logging, and threads;
- explicit infrastructure for Zephyr kernel objects and fixed-address semantics.

The result is a crate that tries to be idiomatic where possible, but remains honest about Zephyr's
underlying model and constraints.
