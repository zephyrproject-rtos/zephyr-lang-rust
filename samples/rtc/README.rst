.. _rust_rtc_sample:

Rust RTC
########

Overview
********

This sample uses the blocking Rust RTC API to acquire a device from the
``rtc`` devicetree alias, check readiness, set a validated calendar value,
and read the time. It sets the clock to February 29, 2024 at 12:34:00 and
prints calendar time once per second after checking that the clock advances.

``RtcTime`` uses full calendar years and months numbered 1 through 12.
It does not perform timezone conversions. The sample uses zero nanoseconds
because supported precision depends on the RTC driver.

.. warning::

   Running this sample overwrites the RTC time, including on physical hardware.
   Acquiring an RTC handle alone does not reset or set the clock.

Requirements
************

The default supported platform is ``qemu_cortex_m3``. Its board overlay adds
a ``zephyr,rtc-emul`` device, so no physical RTC is required. The emulator
driver is enabled automatically when this node and ``CONFIG_RTC`` are enabled.

Building and Running
********************

From the root of the Rust module, build and run with:

.. code-block:: console

   west build -b qemu_cortex_m3 samples/rtc
   west build -t run

Example output (logging prefixes omitted):

.. code-block:: text

   RTC time: 2024-02-29 12:34:00
   RTC time: 2024-02-29 12:34:01
   RTC sample successful
   RTC time: 2024-02-29 12:34:02

The initial read may already include a clock tick. The sample checks the
date, hour, and minute on the initial read and during a bounded progression
check. It polls for up to five seconds for the seconds field to increase,
then prints ``RTC sample successful``. Acquisition, readiness, set/read,
date readback, or progression failures cause a panic instead of printing
the success marker. Twister uses this marker to validate execution in CI.
After the marker, the sample continues reading and printing indefinitely.

To run the automated check from the module root:

.. code-block:: console

   west twister -T samples/rtc -p qemu_cortex_m3 --inline-logs

Using Physical Hardware
***********************

For a supported physical RTC, enable the device and its driver and provide
an ``rtc`` devicetree alias pointing to it. The device must have a compatible
listed in the RTC rules in ``dt-rust.yaml``; enabling ``CONFIG_RTC`` alone
does not generate a Rust handle for an arbitrary RTC driver. This sample
does not test physical hardware in CI.
