# Settings Tray

A persistent overlay, reachable from any scene, that gives quick access to core system settings — brightness, sound
volume, night mode, device restart, and WiFi reconfiguration — without leaving the current scene or opening the web UI.

## User stories

### Reveal and dismiss the tray from anywhere

> As a user, I want to swipe down from the top edge to reveal a settings tray from any scene so that core controls are
> always one gesture away.

- Swiping down from the top edge reveals the tray on top of whatever scene is showing.
- A predominantly vertical upward swipe dismisses it; a horizontal drag across the controls does not.
- The tray dismisses itself after 15 seconds without interaction.
- The tray retracts on its own if a full-screen screen takes over while it is open — for example when an alarm starts
  ringing — so it never sits on top of that screen.
- While the tray is sliding in or out it does not react to touch, so a late tap cannot trigger a control mid-animation.

### Adjust brightness and volume

> As a user, I want to adjust screen brightness and sound volume from the tray so that changes take effect immediately
> where I am.

- Brightness and volume each step up or down through a pair of buttons; the value applies immediately and persists after
  the tray is dismissed.
- The values reflect the current system state when the tray opens, including changes made elsewhere (for example from
  the web UI) while the tray is open.
- While the user is stepping a value, delayed feedback of earlier values never bounces it back.

### Toggle night mode

> As a user, I want to see and toggle night mode from the tray so that I can override the schedule on the spot.

- The tray shows whether night mode is currently on or off, and — when a schedule is configured — the time until the
  current state lasts.
- Tapping the toggle switches night mode immediately.

### Restart the device deliberately

> As a user, I want to restart the device from the tray, with a confirmation gesture, so that a stray tap can never
> reboot it.

- Restart requires press-and-hold; a filled circle fades in behind the button and shrinks into it while holding, and
  releasing early cancels.
- Completing the hold restarts the device; the tray shows the progress ("Keep holding…", "Restarting…").
- While a firmware upgrade is being applied the device declines the restart and the tray shows why, so an upgrade can
  never be corrupted by a manual reboot.

### See the device's address

> As a user, I want the tray to show the device's IP address so that I can reach its web UI from my phone without
> hunting for it.

- The tray shows the device's current IP address on every device.
- When no address is available, the tray shows a placeholder rather than a stale or invented one.
- Where the display has the room, the tray also shows the hostname and a QR code that opens the web UI when scanned.

### Reconfigure WiFi from the device

> As a user, I want to restart WiFi setup from the tray so that I can re-home the device onto a new network without the
> web UI.

- A hold-to-confirm "Reconfigure WiFi" button starts the WiFi setup access point; the tray then shows the setup network
  to join from a phone.
- The tray shows the connection status: the connected network and signal strength.

### See only the controls the device supports

> As a user, I want the tray to show only controls my device actually has so that nothing on screen is a dead end.

- Devices without sound hardware show no volume controls.
- Devices whose WiFi is not managed directly by the system show no WiFi button.
- The available controls are decided by the device itself; the tray never guesses.

## Constraints

- The tray renders above all scenes and widgets; only one instance exists.
- Restart and WiFi actions always require the hold gesture — there is no single-tap variant.
- Bluetooth and extended settings are out of scope; the tray covers core settings only.
