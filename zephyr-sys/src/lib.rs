// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! Zephyr application support for Rust
//!
//! This crates provides the core functionality for applications written in Rust that run on top of
//! Zephyr.

#![no_std]
// Allow rust naming convention violations.
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
// Zephyr makes use of zero-sized structs, which Rustc considers invalid.  Suppress this warning.
// Note, however, that this suppresses any warnings in the bindings about improper C types.
#![allow(improper_ctypes)]
#![allow(rustdoc::broken_intra_doc_links)]
#![allow(rustdoc::bare_urls)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

// We have directed bindgen to not generate copy for any times.  It unfortunately doesn't have an
// easy mechanism to enable just for a few types.

// Fortunately, it isn't difficult to mostly auto-derive copy/clone.
macro_rules! derive_clone {
    ($($t:ty),+ $(,)?) => {
        $(
            impl Clone for $t {
                fn clone(&self) -> $t {
                    *self
                }
            }
        )+
    };
}

macro_rules! derive_copy {
    ($($t:ty),+ $(,)?) => {
        $(
            impl Copy for $t {}
        )+
    }
}

derive_copy!(z_spinlock_key);
derive_clone!(z_spinlock_key);
