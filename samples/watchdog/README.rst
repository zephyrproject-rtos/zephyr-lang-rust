.. SPDX-License-Identifier: Apache-2.0

Rust watchdog
#############

The main thread starts a 1000 ms watchdog and moves its owned ``WatchdogChannel``
to a worker thread, which feeds it five times at 250 ms intervals. The main thread
joins the worker, prints ``Waiting for reset...``, and waits without feeding.
The watchdog resets the target, and the sample starts again.

No shared controller or application mutex is needed. The wrapper serializes
device operations internally; lock contention returns ``EAGAIN`` without retrying.
Both supported drivers expose one channel per device, so this sample demonstrates
cross-thread ownership, not multiple hardware channels or deliberate contention.
The latter is covered by ``tests/drivers/watchdog``.

Building and running
********************

From a Zephyr west workspace:

.. code-block:: console

   west build -b mps2/an385 modules/lang/rust/samples/watchdog
   west build -t run

Stop QEMU after observing ``Watchdog sample application from Rust`` again after
``Waiting for reset...``. The console test checks this ordered sequence, so
feeding alone is not sufficient to pass. The MPS2 overlay removes unused LED and
button nodes whose one-cell GPIO specifiers the Rust wrappers do not support.

For the NXP FRDM-MCXA156 board:

.. code-block:: console

   west build -b frdm_mcxa156/mcxa156 -d build/watchdog-mcxa156 \
       modules/lang/rust/samples/watchdog

With a J-Link probe:

.. code-block:: console

   west flash -d build/watchdog-mcxa156 -r jlink

This board already enables WWDT0 and provides the alias; no overlay is needed.
Use the serial console at 115200 baud to observe the same feed, reset, and restart
sequence. Cross-building alone does not verify hardware feeding timing or reset
behavior.
