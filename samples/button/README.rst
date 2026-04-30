.. zephyr:code-sample:: button
   :name: Button
   :relevant-api: gpio_interface

   Blink an LED continuously, and keep it on while a button is pressed.

Overview
********

The Button sample blinks an LED on a 1-second ticker, keeps the LED steadily on
while the button is pressed, and resumes blinking when the button is released.
If no LED is available, the sample will print messages to the console to show
the state transitions when holding and releasing the button.

The source code shows how to:

#. Get a gpio pin from a devicetree alias
#. Configure one GPIO pin as an input with a callback on interrupts
#. Configure one GPIO pin as an output whose behavior changes based on the input
   pin state

.. _button-sample-requirements:

Requirements
************

Your board must:

#. Have a button configured using the ``sw0`` devicetree alias.

Optionally, you may:

#. Have an LED configured using the ``led0`` devicetree alias.

Building and Running
********************

Build and flash Button as follows, changing ``reel_board`` for your board:

.. zephyr-app-commands::
   :zephyr-app: samples/button
   :host-os: unix,windows
   :board: reel_board
   :goals: build flash
   :compact:

After flashing, try to press the button. If your board defines ``led0``, the LED
will light up when the button is pressed.

Build errors
************

Adding board support
********************

To add support for your board, add something like this to your devicetree:

.. code-block:: DTS

   / {
   	aliases {
   		sw0 = &mybutton0;
   		led0 = &myled0;
   	};

   	leds {
   		compatible = "gpio-leds";
   		myled0: led_0 {
   			gpios = <&gpio0 13 GPIO_ACTIVE_LOW>;
   		};
   	};

   	keys {
   		compatible = "gpio-keys";
   		mybutton0: button_0 {
   			gpios = <&gpio0 0 (GPIO_ACTIVE_LOW | GPIO_PULL_UP)>;
   			label = "Button 0";
   			linux,code = <KEY_ENTER>;
   		};
   	};
   };

Tips:

- See :dtcompatible:`gpio-leds` for more information on defining GPIO-based LEDs
  in devicetree.

- See :dtcompatible:`gpio-keys` for more information on defining GPIO-based KEYs
  in devicetree.

- If the LED is built in to your board hardware, the alias should be defined in
  your :ref:`BOARD.dts file <devicetree-in-out-files>`. Otherwise, you can
  define one in a :ref:`devicetree overlay <set-devicetree-overlays>`.
