# Clock Alarm

Users set up alarms from the Deck's web app; when one goes off, the Deck takes over the whole screen to show it is
ringing and offers the two actions that matter in the moment — stop it, or snooze it. The screen is the visual half of a
firing alarm; the LED and sound feedback are covered in [LED Notifications](led-notifications.md).

## User stories

### See a ringing alarm

> As a user, I want the Deck to clearly show when an alarm is going off so I notice it and know which alarm it is.

- When an alarm fires, a full-screen alarm appears on top of whatever was on the display.
- It shows the alarm's scheduled time and its label, falling back to "Alarm" when the alarm has no label.
- It stays up until the alarm is stopped or snoozed.

### Stop or snooze from the screen

> As a user, I want to stop or snooze a ringing alarm directly from the screen so I can act on it with one tap.

- The screen shows a **Stop Alarm** button that ends the alarm.
- When the alarm can still be snoozed, it also shows a **Snooze** button that silences it for the alarm's snooze
  interval and lets it ring again afterwards.
- After stopping or snoozing, the alarm screen goes away and the previous display returns.

### Only snooze while snoozing is allowed

> As a user, I want the Snooze option to disappear once I'm out of snoozes so I'm not misled into thinking I have
> another one.

- The Snooze button is shown only while the alarm may still be snoozed.
- An alarm with no snooze configured shows Stop only.
- An alarm that has reached its snooze limit shows Stop only; snoozing is no longer offered, and the alarm cannot be
  snoozed past its limit.

### The alarm owns the screen while ringing

> As a user, I want the ringing alarm to be front and center so nothing else sits on top of it.

- While the alarm is ringing it covers the active scene and takes the screen's touch input.
- If the quick-settings tray is open when the alarm fires, it retracts so it does not sit on top of the alarm.

### Never stuck ringing

> As a user, I want to be able to silence an alarm even if the on-screen controls are not available, so a ringing alarm
> never traps the device.

- If the alarm controls cannot be shown for any reason, a touch anywhere on the screen still stops the alarm.
- If nothing can display the alarm at all, the device stops it on its own after a short moment rather than ringing
  indefinitely.

### Create and manage alarms from the web app

> As a user, I want to add, edit, enable, and delete alarms from the Deck's web interface so I control when it rings.

- The web app lists every configured alarm with its time, label, repeat days, sound, and snooze setting, each with an
  on/off toggle.
- I can add a new alarm, edit an existing one, toggle one on or off without deleting it, or delete it.

### Choose when an alarm rings

> As a user, I want to set each alarm's time and repeat days so it fits my routine.

- Each alarm has a time, shown in my configured 12- or 24-hour format.
- I can repeat it on any set of weekdays, or leave it as a one-off.

### Give an alarm a label and a sound

> As a user, I want to name an alarm and pick its sound so I recognize it when it rings.

- An alarm can have an optional label, shown both in the list and on the firing screen.
- I pick the sound from the available alarm sounds and can preview it from the form before saving.

### Configure snooze per alarm

> As a user, I want to decide whether and how an alarm can be snoozed so it matches how heavily I sleep.

- Snooze can be enabled or disabled per alarm.
- When enabled, I set the snooze duration and how many times it may be snoozed — the same limit the firing screen
  enforces when it stops offering Snooze.

## Constraints

- Whether Snooze is offered, and how many times, follows the individual alarm's snooze configuration.
- The matching sound and LED feedback while an alarm is active are described in
  [LED Notifications](led-notifications.md).
