// Copyright (c) 2026 Google LLC
// SPDX-License-Identifier: Apache-2.0

//! Tests of `zephyr::error`: errno constants, name lookup, `to_result*`, `ResultExt`, and that
//! the pre-existing `Error(pub u32)` API and formatting are unchanged.

#![no_std]

extern crate alloc;

use alloc::format;
use core::mem::size_of;

use zephyr::error::{to_result, to_result_void, Error, Result, ResultExt};
use zephyr::printkln;
use zephyr::raw;
use zephyr_panic as _;

#[no_mangle]
extern "C" fn rust_main() {
    test_to_result();
    test_constants_match_sys();
    test_display_unchanged();
    test_result_ext();
    test_layout();
    test_backward_compat();
    printkln!("All tests passed");
}

fn test_to_result() {
    assert_eq!(to_result_void(0), Ok(()));
    assert_eq!(to_result(0), Ok(0));
    assert_eq!(to_result(5), Ok(5));
    assert_eq!(to_result_void(-(raw::EINVAL as i32)), Err(Error::EINVAL));
    assert_eq!(to_result(-(raw::EBUSY as i32)), Err(Error::EBUSY));
    assert_eq!(to_result_void(-999), Err(Error::from_raw(999).unwrap()));

    assert_eq!(Error::from_raw(0), None);
    assert_eq!(Error::from_raw(raw::EINVAL as i32), Some(Error::EINVAL));
    assert_eq!(Error::from_raw(-(raw::EINVAL as i32)), Some(Error::EINVAL));

    assert_eq!(Error::EINVAL.raw(), raw::EINVAL as i32);
    assert_eq!(Error::EINVAL.as_neg(), -(raw::EINVAL as i32));
    printkln!("test_to_result: ok");
}

fn test_constants_match_sys() {
    assert_eq!(Error::EINVAL.raw(), raw::EINVAL as i32);
    assert_eq!(Error::EALREADY.raw(), raw::EALREADY as i32);
    assert_eq!(Error::ENODEV.raw(), raw::ENODEV as i32);
    assert_eq!(Error::EOPNOTSUPP.raw(), raw::EOPNOTSUPP as i32);

    assert_eq!(Error::EINVAL.name(), Some("EINVAL"));
    assert_eq!(Error::ENODEV.name(), Some("ENODEV"));

    // Aliases compare equal and report the canonical name.
    assert_eq!(Error::EWOULDBLOCK, Error::EAGAIN);
    assert_eq!(Error::EWOULDBLOCK.name(), Some("EAGAIN"));
    assert_eq!(Error::EAGAIN.name(), Some("EAGAIN"));

    // ENOTSUP is an alias of EOPNOTSUPP on glibc (native_sim), a distinct value elsewhere.
    assert!(Error::ENOTSUP.raw() > 0);
    #[cfg(not(target_os = "linux"))]
    assert_eq!(Error::ENOTSUP.name(), Some("ENOTSUP"));

    assert_eq!(Error::from_raw(999).unwrap().name(), None);
    printkln!("test_constants_match_sys: ok");
}

fn test_display_unchanged() {
    // The existing output format must not change for current users.
    let expected = format!("zephyr error errno:{}", raw::ENODEV);
    assert_eq!(format!("{}", Error::ENODEV), expected);
    assert_eq!(format!("{:?}", Error::ENODEV), expected);

    let unknown = Error::from_raw(999).unwrap();
    assert_eq!(format!("{}", unknown), "zephyr error errno:999");
    assert_eq!(format!("{:?}", unknown), "zephyr error errno:999");

    assert_eq!(
        format!("{:?}", Err::<(), _>(Error::EALREADY)),
        format!("Err(zephyr error errno:{})", raw::EALREADY)
    );
    printkln!("test_display_unchanged: ok");
}

fn test_result_ext() {
    let already: Result<()> = Err(Error::EALREADY);
    assert_eq!(already.ignore(Error::EALREADY), Ok(()));
    assert_eq!(already.ignore(Error::EINVAL), Err(Error::EALREADY));
    assert_eq!(Ok::<(), Error>(()).ignore(Error::EALREADY), Ok(()));
    assert_eq!(
        Err::<u8, _>(Error::EBUSY).ignore_with(Error::EBUSY, || 7),
        Ok(7)
    );
    assert_eq!(Ok::<u8, Error>(3).ignore_with(Error::EBUSY, || 7), Ok(3));
    printkln!("test_result_ext: ok");
}

fn test_layout() {
    assert_eq!(size_of::<Error>(), 4);
    printkln!("test_layout: ok");
}

fn test_backward_compat() {
    // Code written against the original `Error(pub u32)` keeps working.
    assert_eq!(Error(raw::EINVAL), Error::EINVAL);
    assert_eq!(Error::EINVAL.0, raw::EINVAL);
    assert_eq!(to_result(-5), Err(Error(5)));
    match to_result_void(-(raw::EBUSY as i32)) {
        Err(e) => assert_eq!(e.0, raw::EBUSY),
        Ok(()) => panic!("expected an error"),
    }
    // `Error(0)` is still constructible; it just has no name.
    assert_eq!(Error(0).name(), None);
    printkln!("test_backward_compat: ok");
}
