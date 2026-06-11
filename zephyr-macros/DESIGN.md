# zephyr-macros Design

## Purpose

`zephyr-macros` is a small proc-macro crate that currently exists to provide one user-facing
attribute:

- `#[zephyr::thread(...)]`

Its role is not to implement runtime functionality itself. Instead, it generates boilerplate around
the runtime thread support provided by the main `zephyr` crate.

In other words:

- `zephyr-macros` rewrites syntax;
- `zephyr::thread` provides the actual runtime types and behavior.

## Source Layout

- `src/lib.rs`: proc-macro entry point and user-facing documentation for `#[thread]`.
- `src/task.rs`: full implementation of the attribute expansion and validation logic.

Despite the filename `task.rs`, the current macro is about Zephyr threads, not a general task
system.

## Public Surface

The crate exposes a single attribute macro:

- `thread`

From a user perspective, it is re-exported by the `zephyr` crate as `#[zephyr::thread]`.

The macro converts a Rust function declaration into:

- a hidden renamed inner function containing the original body;
- a public wrapper function with the original name;
- static thread-pool and stack storage;
- a C ABI startup shim that Zephyr can invoke as a thread entry point.

## Overall Expansion Strategy

The design of `#[zephyr::thread]` is to let users write something that looks like a normal Rust
function:

```rust
#[zephyr::thread(stack_size = 1024)]
fn worker(arg: u32) {
    // ...
}
```

and expand it into something callable that returns `zephyr::thread::ReadyThread`.

That returned `ReadyThread` can then be started explicitly by the caller. This matches the runtime
design in `zephyr::thread`, where threads are created in a prepared state and then started.

## Attribute Arguments

The macro currently supports:

- `stack_size`
- `pool_size`

Both are parsed as Rust expressions via `darling`.

Current behavior from the source:

- `pool_size` defaults to `1`;
- `stack_size` currently defaults to `2048`, even though the public docs say it should be required.

That default is a pragmatic implementation choice rather than a final interface guarantee.

## Generated Runtime Structure

For each annotated function, the macro generates several internal items.

### Hidden Inner Function

The original function is renamed to a hidden symbol like:

- `__mythread_thread`

This preserves the user's function body with minimal transformation.

### Argument Carrier Struct

The function arguments are repackaged into a generated internal struct:

- `_ZephyrInternalArgs`

This allows the macro to move all arguments into Zephyr's thread startup data as one owned value.

### Static Thread Pool

The macro generates a static array of:

- `zephyr::thread::ThreadData<_ZephyrInternalArgs>`

The array size is `pool_size`. This means the attribute is not generating a single-use thread by
default; it is generating access to a reusable pool of thread slots.

### Static Stack Pool

The macro also generates a matching static array of:

- `zephyr::thread::ThreadStack<_ZEPHYR_INTERNAL_STACK_SIZE>`

again sized by `pool_size`.

### Startup Shim

An `extern "C"` startup function is generated. Zephyr invokes this as the actual thread entry
point. It:

1. receives the raw startup pointer passed through Zephyr;
2. reinterprets it as `InitData<_ZephyrInternalArgs>`;
3. extracts the stored argument struct;
4. calls the hidden inner Rust function with those arguments.

This is the key bridge from Zephyr's C-style thread entry ABI into the original Rust function
signature.

### Public Wrapper Function

The user's original function name is retained for a generated wrapper that:

- builds `_ZephyrInternalArgs` from the user arguments;
- calls `zephyr::thread::ThreadData::acquire(...)`;
- returns `zephyr::thread::ReadyThread`.

So the macro is effectively a constructor generator for thread instances.

## Validation Rules

The macro performs static validation before generating code.

The current checks reject:

- `async fn`
- generic functions
- `where` clauses
- explicit ABI qualifiers
- variadic functions
- receiver arguments such as `self`
- non-`()` / non-`!` return types
- non-`'static` references in arguments
- `impl Trait` in argument position
- destructuring patterns in parameters

These constraints follow from the runtime model:

- thread functions need a concrete, monomorphic entry point;
- arguments must outlive transfer into the spawned Zephyr thread;
- Zephyr startup expects owned data, not arbitrary pattern destructuring;
- the generated startup shim assumes a plain identifier-based argument unpacking model.

## Error Handling Strategy

One notable design choice is that macro failures do not simply stop expansion immediately after the
first problem.

Instead, the implementation tries to:

- accumulate validation errors into a token stream;
- still emit something structurally close to the expected output;
- fall back to a placeholder `todo!()`-based wrapper return value when needed.

This is explicitly done to improve IDE behavior. The source comments make clear that the goal is to
preserve completion and editing support even when the input is currently invalid.

That is a practical proc-macro design choice rather than a runtime concern.

## Relationship to `zephyr::thread`

The macro is tightly coupled to the runtime API in `zephyr::thread`.

It depends directly on the existence and behavior of:

- `zephyr::thread::ThreadData`
- `zephyr::thread::ThreadStack`
- `zephyr::thread::InitData`
- `zephyr::thread::ReadyThread`
- `zephyr::thread::stack_len`

This means `zephyr-macros` is not a general reusable thread macro framework. It is specifically a
syntax layer over the runtime thread model implemented in this repository.

## Naming and Current Rough Edges

A few implementation details in the current source are worth calling out:

- `task.rs` still carries comments referring to `task` rather than `thread`;
- the generated stack linker section currently uses a placeholder string `.noinit.TODO_STACK`;
- the crate metadata contains `descriptions` instead of the usual `description`;
- the user-facing docs say `stack_size` must be specified, but the implementation still provides a
  default.

These do not change the design intent, but they show the crate is still evolving.

## Extending the Crate

With the current structure, likely extensions would be:

1. add more validation and better diagnostics to `#[thread]`;
2. tighten the interface so documented requirements like mandatory `stack_size` match the
   implementation;
3. generate richer configuration options such as thread priority or options flags;
4. add additional proc macros only when they have a clear runtime home in the `zephyr` crate.

The current crate is intentionally minimal. It does not appear to be aiming for a broad catalog of
macros without matching runtime support.

## Non-Goals

From the current source, `zephyr-macros` is not trying to:

- implement runtime scheduling or thread management itself;
- provide a general-purpose task system independent of `zephyr`;
- support arbitrary Rust function signatures;
- hide the Zephyr runtime model behind magical syntax.

## Summary

`zephyr-macros` is a thin proc-macro layer over the `zephyr::thread` runtime. Its single current
job is to turn a restricted Rust function into the static storage, startup shim, and wrapper
function needed to spawn Zephyr-managed threads with typed Rust arguments, while enforcing the
signature constraints that make that transformation sound.
