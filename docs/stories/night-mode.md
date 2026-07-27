# Night Mode

Night mode changes the device behavior during configured quiet hours. It lowers the display brightness, uses a separate
sound volume, optionally turns the screen off after inactivity, and controls whether LED notifications remain visible.

## User stories

### Schedule quiet hours

> As a user, I want night mode to activate automatically during my configured quiet hours so the device is less
> distracting overnight.

- Enable or disable night mode from *Settings > Display*.
- Configure a local start and end time for night mode.
- The schedule supports intervals that cross midnight, such as 22:30 to 06:30.
- Changing the device timezone recalculates whether night mode is currently active.
- Night mode settings persist and survive device reboots.

### Dim the display

> As a user, I want a separate brightness level for night mode so the display remains readable without lighting up the
> room.

- Day brightness and night-mode brightness are configured separately.
- When night mode becomes active, the backlight switches to the configured night-mode brightness.
- When night mode ends, the backlight returns to the normal configured brightness.
- Brightness changes apply immediately when the active mode or configured value changes.

### Reduce sound volume

> As a user, I want a separate sound volume for night mode so alarms and notifications are less disruptive overnight.

- Normal sound volume and night-mode sound volume are configured separately.
- When night mode becomes active, the audio output switches to the configured night-mode volume.
- When night mode ends, the audio output returns to the normal configured volume.
- Volume changes apply immediately when the active mode or configured value changes.

### Hold the display still

> As a user, I want the display to stop moving between scenes during night mode so nothing changes while I sleep.

- Automatic scene cycling stops for as long as night mode is active.
- When night mode becomes active, the display returns to the first visible scene.
- Swiping between scenes by hand still works while the screen is on.
- When night mode ends, automatic scene cycling resumes, unless the user has turned it off in *Settings*.

### Turn the screen off after inactivity

> As a user, I want the screen to turn off during night mode after a period of inactivity so the room stays dark.

- Screen auto-off is only active while night mode is active.
- The user can choose **Never** or a fixed inactivity timeout.
- A configured timeout of **Never** keeps the screen on during night mode.
- Any non-zero timeout turns the screen off after the configured inactivity period.
- When night mode ends, the screen is turned back on if auto-off had turned it off.

### Wake the screen

> As a user, I want to wake the screen with normal device interaction so I do not need a special gesture at night.

- Touch activity wakes the screen when screen auto-off has turned it off.
- The touch that wakes the screen only wakes it — it does not press, swipe, or otherwise interact with the scene
  underneath.
- Physical button activity wakes the screen when screen auto-off has turned it off.
- Waking the screen restores the correct current brightness for the active mode.
- After waking from screen auto-off, the display shows the first visible scene.
- Activity restarts the inactivity timeout while night mode remains active.

### Control LED notifications overnight

> As a user, I want to decide whether LED notifications remain active during night mode so I can avoid distracting
> lights while still receiving important ambient signals if I want them.

- A global **Enable LED Notifications** setting controls LED notifications outside night mode.
- A separate **Enable LED Notifications in Night Mode** setting controls LED notifications while night mode is active.
- When night mode becomes active, the LED controller switches to the night-mode LED setting.
- When night mode ends, the LED controller switches back to the normal LED setting.
- LED notification effects and priorities are defined by the [LED Notifications](led-notifications.md) story.

## Constraints

- Night mode uses the existing backlight driver for screen brightness, power, and wake behavior.
- Night mode uses the existing LED control path; it does not introduce a parallel LED driver.
- Night mode state will be broadcasted to running widgets through the Wayland widget settings protocol, when BDK-344
  lands.
- Bootloader environment sync mirrors the current day/night schedule, screen brightness, and LED settings so early boot
  behavior matches the configured mode.
- Screen auto-off blanks the display hardware; it does not replace the active scene. The scene reset that pairs with it
  happens while the panel is already dark, so it is never visible.
