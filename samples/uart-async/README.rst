Async UART loopback
###################

This sample uses dedicated emulated UARTs, separate from the console, to exercise
both explicit Rust async backends. It needs no physical wiring or DMA hardware.
The event-backed emulator implements the same API as DMA-backed physical drivers.

Each backend takes application-provided static storage, splits into borrowed RX/TX
halves, and concurrently sends and receives a binary message. It also cancels an
initial pending read and explicitly awaits shutdown. The TX queue is smaller than
the message, demonstrating partial progress; the event RX buffer is larger than
the message, demonstrating inactivity-based short-message reception.

Build and run both backends on different emulated devices:

.. code-block:: console

   west build -b qemu_cortex_m3 modules/lang/rust/samples/uart-async
   west build -t run

For interrupt-only or event-only operation, add respectively
``-- -DEXTRA_CONF_FILE=irq.conf`` or ``-- -DEXTRA_CONF_FILE=event.conf`` to the
build command. ``native_sim/native/64`` is also supported; run the generated
``build/zephyr/zephyr.exe`` directly.

The final message is ``Async UART loopback: all tests passed``.

Contract notes
**************

* Individual reads and writes make progress only when returning their byte count.
  Cancelling a pending individual operation does not consume caller data.
* Writes count bytes accepted into static storage, not completed wire transfers.
  Cancelling ``write_all`` can leave an accepted prefix queued or in flight.
* Event-backed ``flush`` awaits driver TX completion; IRQ-backed ``flush`` returns
  ``ENOTSUP`` because portable hardware-idle confirmation is unavailable.
* RX software overflow reports ``EOVERFLOW`` and discards software-buffered input.
* Shutdown stops RX and aborts queued/in-flight TX; it is not a flush or power-off.
  Drop starts that stop without waiting. Static storage is single-use and retained
  for late callbacks, even after failure or dropping the handle.
* Event driver buffers are additional to the software queues: payload storage is
  approximately ``3 * RX + 2 * TX`` bytes, plus metadata and alignment padding.
  Driver RX buffers and TX staging are 32-byte aligned; actual DMA memory/cache
  requirements still depend on the driver and application memory placement.
