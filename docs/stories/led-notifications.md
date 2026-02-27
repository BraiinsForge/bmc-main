# LED Notifications

The device has a strip of 10 addressable APA102 LEDs driven over SPI.  LEDs
provide ambient, glanceable feedback about device state, network connectivity,
alarms, price movements, and system operations without requiring the user to
look at the screen.

## User stories

### Device boot

> As a user, I want to see the LEDs indicating that the device is alive and
> initializing.  Once the device progresses down the boot process, it
> will start animating.

- A solid violet light is on from the moment the device powers on (set by
  U-Boot).
- A violet KnightRider animation (1 s cycle) plays once the application
  starts.
- The animation stops and LEDs turn off once the device is fully ready.

### Wi-Fi connectivity

> As a user, I want the LEDs to tell me what is happening with my Wi-Fi
> connection so I do not have to navigate to the settings screen.

- **Connecting:** violet KnightRider animation (1 s cycle) while the device
  attempts to join the configured network.
- **Connected:** a brief green flash (2 s) confirms the connection succeeded,
  then LEDs turn off.
- **Scanning:** violet KnightRider animation while a user-initiated Wi-Fi
  scan is in progress; stops when the scan completes.
- **No network / Error:** LEDs turn off immediately.

### Firmware upgrade

> As a user, I want to see progress and outcome of a firmware upgrade at a
> glance.

- **In progress:** orange KnightRider animation (1 s cycle) runs for the
  entire duration of the download or upgrade.
- **Success:** a brief green flash (2 s) signals completion, then LEDs revert
  to the previous state.
- **Error:** a brief red flash (2 s) signals failure, then LEDs revert to the
  previous state.

### Clock alarm

> As a user, I want the LEDs to visually reinforce my alarm so I notice it
> even from across the room.

- An orange breathing animation (4 s cycle) pulses continuously while the
  alarm is active.
- The animation stops when the alarm is dismissed or snoozed.

### Price alerts

> As a user, I want the LEDs to indicate whether the price has gone up or down
> over the last 24 hours.

- **Price up:** green breathing animation (4 s cycle).
- **Price down:** red breathing animation (4 s cycle).
- The animation stops when the price trend event ends.

### Scene preview

> As a user, I want the LEDs to light up when I am previewing a display scene
> so I can evaluate the full visual effect.

- A solid white light turns on for the duration of the scene preview.
- LEDs turn off when the preview ends.

### Enable / disable

> As a user, I want a master switch so I can turn LED notifications off
> entirely when I find them distracting.

- Toggling **Enable LED Notifications** in *Settings > Sound & Light* turns
  all LED activity on or off.
- A separate **Enable LED Notifications in Night Mode** toggle controls
  whether LEDs remain active during scheduled night hours.
- When disabled, no effects render and the driver consumes zero CPU.

## Priority

Multiple events can be active simultaneously.  The LED driver resolves
conflicts with a fixed priority order (highest first):

1. Device lifecycle (boot animation)
2. Clock alarm
3. Wi-Fi state
4. System upgrade
5. Scene preview
6. Wi-Fi scan
7. Price alerts

A temporary effect (success/error flash) overlays the current persistent
effect and automatically expires back to it.  Only one temporary effect
can be active at a time; a newer one replaces any in-progress temporary
effect.

## Hardware

| Property       | Value                    |
|----------------|--------------------------|
| LED type       | APA102 (addressable RGB) |
| Count          | 10                       |
| Interface      | SPI, 4 MHz               |
| Frame rate     | 120 Hz                   |
| Max brightness | 31 (5-bit APA102 limit)  |
