<p align="center">
  <img src="simple-animation.gif" />
</p>

# The simplest oscilloscope imaginable.
just-a-scope is a battery powered, ultra light, and tiny oscilloscope.

Often times when working on embedded projects, once GPIO has just been configured and you take the leap from programming into the real world, you cannot even get a blinky program running. Even if you are lucky enough to succeed in your LED-blinking, whatever you are moving onto next won't work. It is simply way more complicated than a single LED. 

At that point you wish you could "just see the electricity", but you cannot. You wish you had an oscilloscope, but all of the commercially available ones are extremely overkill and way out of budget. This is the problem just-a-scope tries to solve.

just-a-scope is really just an esp32 with some protection, scaling electronics and probes connected to the Analog-to-Digital converter. 9 times out of 10, that's all we'll ever need.

The plot is drawn in a web browser on a device connected over WIFI. At that point you already have the entire plot on your computer, and you can download a .csv file with all measurements made.

## Hardware
The microcontroller is an esp32s3, bought already soldered onto a SEEED Studio XIAO. This is a cheap and very small board which includes a USB-C connector as well as charging electronics for a lithium battery.

When buying one of these boards, you are likely to receive an antenna in the same package. This antenna is known to be unusable due to very poor signal, so a third party antenna must be used.