# Async gpio

This demo makes use of the GPIO `wait_for_high()` and `wait_for_low()` async operations.

Unfortunately, not all GPIO controllers support level triggered interrupts.  Notably, most of the
stm32 line does not support level triggered interrupts.
