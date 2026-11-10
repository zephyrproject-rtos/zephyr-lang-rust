Blocking UART loopback
#####################

This sample uses a dedicated ``zephyr,uart-emul`` device with internal loopback.
The console remains separate, so console output does not enter the UART data
stream. No physical UART wiring is required.

The sample reads and writes individual bytes with ``poll_in`` and ``poll_out``,
then uses ``embedded-io``'s ``write_all`` and ``read_exact`` helpers to exchange
text and binary data. Interrupt-driven and async UART support are disabled.
The payload is kept below the emulator's FIFO capacity.

Build and run on Cortex-M3 QEMU:

.. code-block:: console

   west build -b qemu_cortex_m3 modules/lang/rust/samples/uart
   west build -t run

Alternatively, build for ``native_sim/native/64`` and run
``build/zephyr/zephyr.exe``.

The final console message is ``UART loopback: all tests passed``.

The polling wrapper preserves the existing UART configuration. Its blocking
traits transfer one byte per call, and a read without available input sleeps
between polls. Polling has no portable hardware TX-drain query, so ``flush``
returns ``ENOTSUP`` rather than claiming that transmission has completed.
