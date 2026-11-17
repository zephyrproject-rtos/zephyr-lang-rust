.. SPDX-License-Identifier: Apache-2.0

Rust watchdog
#############

This sample starts a 1000 ms watchdog and feeds it every 250 ms. After printing
``Watchdog fed 5 times``, it continues feeding rather than resetting the target.

Building and running
********************

From a Zephyr west workspace:

.. code-block:: console

   west build -b mps2/an385 modules/lang/rust/samples/watchdog
   west build -t run

Stop QEMU after observing the success message. The MPS2 overlay removes unused
LED and button nodes whose one-cell GPIO specifiers the Rust wrappers do not
support.

For the NXP FRDM-MCXA156 board:

.. code-block:: console

   west build -b frdm_mcxa156/mcxa156 -d build/watchdog-mcxa156 \
       modules/lang/rust/samples/watchdog
   west flash -d build/watchdog-mcxa156

This board already enables WWDT0 and provides the alias; no overlay is needed.
Cross-building does not verify hardware feeding timing or reset behavior.
