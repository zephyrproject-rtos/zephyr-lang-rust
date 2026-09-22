.. _rust_pressure_polling_sample:

Pressure Polling Sample (Rust)
==============================

Overview
========

This is a Rust version of the pressure polling sensor sample. It demonstrates how to:

- Get a sensor device from the device tree using device aliases
- Perform synchronous sensor readings
- Poll a sensor in a loop
- Convert and display sensor values (temperature, pressure, and altitude)

The sample polls a DPS368 pressure sensor at 1 Hz and displays:
- Temperature in Celsius
- Pressure in kilopascals
- Altitude in meters (if supported by the sensor)

Requirements
============

- Zephyr with Rust support
- DPS368 pressure sensor must be enabled in the device tree for the board

Building and Running
====================

Build the sample:

.. code-block:: console

   $west build -t rustdoc -b cy8ckit_062s2_ai -p always samples/sensor/pressure_polling

Run the sample:

.. code-block:: console

   west flash

Sample Output
=============

The sample will output readings like::

   temp 25.32 Cel, pressure 101.325 kPa, altitude 20.5 m
   temp 25.33 Cel, pressure 101.324 kPa, altitude 20.3 m
   temp 25.32 Cel, pressure 101.326 kPa, altitude 20.6 m
