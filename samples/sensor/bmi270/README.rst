.. zephyr:code-sample:: rust-bmi270
   :name: BMI270 6-axis IMU sensor (Rust)
   :relevant-api: sensor_interface

   Configure and read accelerometer and gyroscope data from a BMI270 sensor in Rust.

Overview
********

This is a Rust port of ``samples/sensor/bmi270``. It configures the BMI270 accelerometer and
gyroscope to measure at 100 Hz and writes the result to the console.

The sensor is accessed via the device tree using ``DEVICE_DT_GET_ONE(bosch_bmi270)`` macro,
which points to a BMI270 sensor on the I2C bus at address 0x68. The sensor API functions
are exposed through zephyr-sys bindings with proper inline wrappers for device tree macros.

Requirements
************

* BMI270 sensor connected to I2C at address 0x68

Hardware Setup
**************

Connect the BMI270 sensor to the cy8ckit_062s2_ai board:

* VDD: 1.8V - 3.6V power supply
* VDDIO: 1.8V - 3.6V I/O voltage
* GND: Ground
* SDA: I2C SDA (P0.3)
* SCL: I2C SCL (P0.2)
* SDO: Connect to GND for address 0x68 (or VDD for 0x69)

Note: External pull-up resistors may be required on SDA/SCL lines (typically 2.2kΩ to 10kΩ).

Building and Running
********************

.. code-block:: console

   west build -t rustdoc -b cy8ckit_062s2_ai -p always samples/sensor/bmi270
   west flash

Sample Output
*************

.. code-block:: console

   *** Booting Zephyr OS build v4.4.0-5388-g6a16925e945f ***
   INFO:rustapp: Starting BMI270 sample for cy8ckit_062s2_ai
   INFO:rustapp: Device found at 0x10015248
   INFO:rustapp: Device 0x10015248 is ready
   INFO:rustapp: Accelerometer configured
   INFO:rustapp: Gyroscope configured, starting sampling loop
   INFO:rustapp: AX: 0.000000; AY: 0.000000; AZ: 0.000000; GX: 0.000000; GY: 0.000000; GZ: 0.000000;
   INFO:rustapp: AX: -2.622927; AY: -4.237267; AZ: 9.759662; GX: 0.000000; GY: 0.000000; GZ: 0.000000;
   INFO:rustapp: AX: -2.373923; AY: -4.436591; AZ: 9.384958; GX: 0.105198; GY: 0.043943; GZ: 0.112122;
   INFO:rustapp: AX: -2.309876; AY: -4.258816; AZ: 8.757658; GX: 0.117448; GY: 0.032757; GZ: 0.120112;
   ...

The output shows accelerometer readings (AX, AY, AZ) in G and gyroscope readings
(GX, GY, GZ) in degrees per second (dps). Initial readings may be zero while the sensor
calibrates; actual values will appear as the sensor stabilizes or is moved.
