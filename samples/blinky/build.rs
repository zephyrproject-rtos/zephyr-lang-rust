fn main() {
    // This call will make make config entries available in the code for every device tree node, to
    // allow conditional compilation based on whether it is present in the device tree.
    // For example, it will be possible to have:
    // ```rust
    // #[cfg(dt = "aliases::led0")]
    // ```
    zephyr_build::dt_cfgs();
}
