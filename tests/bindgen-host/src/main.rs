// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code, non_camel_case_types)]
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
fn main() {
    // SAFETY: k_existing takes no arguments and returns a constant.
    assert_eq!(unsafe { k_existing() }, 42);
    assert_eq!(ZR_EXISTING, 42);
}
