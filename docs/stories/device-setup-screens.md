# Device Setup & Connect Screens

Full-screen messages the Deck shows on its own display when it needs to be set up or has just booted: which Wi-Fi
network to join to configure it, how far the setup has got, and the address the web UI is reachable at. A brand-new or
factory-reset device can be set up from a phone without knowing anything about it in advance.

## User stories

### Set up a new device without knowing its address

> As a new owner, I want the Deck to tell me on its screen how to reach it so that I can configure it from my phone
> without hunting for an address.

- A device with no configuration shows the name of the Wi-Fi network it is broadcasting, plus a QR code that opens the
  setup wizard once the phone has joined that network.
- The QR code and the printed address always name the setup network's own address, so scanning it right after joining
  works.
- The screen stays up as long as the device is waiting, and cannot be tapped away while the setup network is live.

### Follow the Wi-Fi join from the device

> As a user configuring the Deck, I want to see whether it managed to join my network so that I know whether to keep
> waiting or fix the password.

- While the device joins, the screen names the network it is joining.
- A successful join is confirmed on screen, then the device shows the address to finish the setup at, with a QR code for
  it.
- A failed join says so, and the device returns to showing its setup network so the credentials can be entered again.
- Once setup is complete the device says so ("ready") and hands the display over to the ordinary scenes.

### Move the device to a different Wi-Fi network

> As a user, I want to re-run Wi-Fi setup from the device so that I can move the Deck to another network without the web
> UI.

- Starting Wi-Fi setup again from the settings tray shows the same setup screens as a first boot.
- The screens step aside after eight minutes without progress, so a device that is otherwise working goes back to its
  scenes instead of sitting on a setup screen.
- Starting the setup again from the tray brings the screens straight back.
- On success the device returns to its scenes directly, since it was already configured before.

### Understand a setup that cannot continue

> As a user, I want a stuck setup to say what happens next so that I know whether to wait or act.

- When the device resolves the problem itself, the screen says it is restarting and the device restarts.
- When it cannot, the screen says the device needs to be restarted, and it waits rather than pretending to recover.
- A device that was already set up gets its clock back: that screen closes on a tap, or on its own after a minute, since
  there is something to go back to and the settings tray still shows the setup network is up.
- A device still being set up keeps the screen, because there is nothing behind it: it never hides a live setup network
  or an unfinished wizard.
- A device that has Wi-Fi credentials but never obtains an address is reset back into setup, so it lands on a screen
  with a way forward instead of a blank one.

### Learn the device's address after a boot

> As a user, I want the Deck to show its address when it starts so that I can open the web UI without looking it up.

- After the scenes-capable boot, the device shows that it is connecting, then its address and a QR code that opens the
  web UI.
- The address shown is the one the device is reachable at on the network it is configured for, never the setup network's
  own address.
- The screen holds briefly and then hands over to the scenes on its own.
- If the address is lost for a moment while the screen is up, the screen keeps showing the last address rather than
  jumping back a step.

### Confirm a finished update

> As a user, I want the device to confirm an update after it restarts so that I know the upgrade actually finished.

- The first screen after a firmware upgrade's restart confirms the update finished.
- It leads into the ordinary boot sequence, so the confirmation and the address screen are one sequence rather than two
  unrelated screens.

### Tap away the screens that are safe to dismiss

> As a user, I want to dismiss the boot screens so that a screen I have already read does not keep the scenes waiting.

- The connect screens shown on an ordinary boot carry a close glyph in the corner, in the same place as the settings
  tray's, and a tap anywhere on the screen closes them.
- Tapping the update confirmation moves on to the connect screens instead of closing, because the sequence is not
  finished yet.
- The setup screens carry no close glyph and ignore taps, so a tap cannot hide a live setup network. The one exception
  is a failure on an already-configured device, which offers the same close glyph as the boot screens.
- Once dismissed, the boot screens stay dismissed for that boot.

## Constraints

- The boot connect screens are shown once per boot. They are not replayed if the display software restarts later in the
  session.
- The setup screens, by contrast, reflect what the device is waiting for right now, so they come back as long as the
  condition holds.
- The QR codes encode a plain web address, so any camera app opens them; no companion app is involved.
- The screens are guidance only. The setup wizard itself runs in the phone's browser.
- A firing alarm, the upgrade screens and the settings tray all draw above these screens. Their timing keeps running
  underneath, so a screen covered for its whole window is missed rather than postponed.
