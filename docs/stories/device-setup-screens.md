# Device Setup & Connect Screens

Full-screen messages the device shows on its own display when it needs to be set up or has just booted: which Wi-Fi
network to join to configure it, or that the network cable already does, how far the setup has got, and the address the
web UI is reachable at. A brand-new or factory-reset device can be set up from a phone without knowing anything about it
in advance. The device is the Deck, or the BMC application on a Mini Miner, whose screen is smaller and which has an
Ethernet port.

## User stories

### Set up a new device without knowing its address

> As a new owner, I want the device to tell me on its screen how to reach it so that I can configure it from my phone
> without hunting for an address.

- A device with no configuration shows the name of the Wi-Fi network it is broadcasting, plus a QR code that opens the
  setup wizard once the phone has joined that network.
- The QR code and the printed address always name the setup network's own address, so scanning it right after joining
  works.
- The screen stays up as long as the device is waiting, and cannot be tapped away while the setup network is live.

### Set up a device over its network cable

> As a new owner of a device with an Ethernet port, I want to plug it into my network and be told where to open the
> wizard, so that I never have to join a setup network from my phone.

- A device whose cable already holds an address shows that address and a QR code for it instead of a setup network:
  there is nothing to join.
- A device that has both offers the cable beside its setup network, and plugging the cable in while that screen is up
  swaps to the address once the port has one. The setup network is not advertised once it has gone.
- Pulling the cable brings the setup network back as soon as it is up again, so the screen never offers a network nobody
  can join or an address nobody can reach.
- A cable without an address shows no address. A device with no Wi-Fi at all asks for the cable instead of offering a
  setup network.
- Skipping Wi-Fi in the wizard on a device reachable over its cable shows that the device is being set up until the
  address to finish at appears.

### Follow the Wi-Fi join from the device

> As a user configuring the device, I want to see whether it managed to join my network so that I know whether to keep
> waiting or fix the password.

- While the device joins, the screen names the network it is joining.
- A successful join is confirmed on screen, then the device shows the address to finish the setup at, with a QR code for
  it.
- A failed join says so, and the device returns to showing its setup network so the credentials can be entered again.
  The network it gave up on is not named again afterwards.
- Once setup is complete the device says so ("ready") and hands the display over to the ordinary scenes.

### Move the device to a different Wi-Fi network

> As a user, I want to re-run Wi-Fi setup from the device so that I can move it to another network without the web UI.

- Starting Wi-Fi setup again from the settings tray shows the same setup screens as a first boot, and they stay up the
  same way: the device keeps showing its setup network until the new credentials arrive.
- On success the device returns to its scenes directly, since it was already configured before.

### Understand a setup that cannot continue

> As a user, I want a stuck setup to say what happens next so that I know whether to wait or act.

- When the device resolves the problem itself, the screen says it is restarting and the device restarts.
- When it cannot, the screen says the device needs to be restarted, and it waits rather than pretending to recover.
- A device that was already set up gets its clock back: that screen closes on a tap, or on its own after a minute, since
  there is something to go back to and the settings tray still shows the setup network is up.
- A device still being set up keeps the screen: it never hides a live setup network or an unfinished wizard.
- A device that has Wi-Fi credentials but never obtains an address is reset back into setup, so it lands on a screen
  with a way forward instead of a blank one.
- A device that runs on its cable with no Wi-Fi configured is not reset: a reset would not bring a cable, so it waits
  for one and says so.

### Learn the device's address after a boot

> As a user, I want the device to show its address when it starts so that I can open the web UI without looking it up.

- After the scenes-capable boot, the device shows that it is connecting, then its address and a QR code that opens the
  web UI.
- The address shown is the one the device is reachable at on the network it is configured for, never the setup network's
  own address. A device running on its cable is told about the network and the cable, never about Wi-Fi.
- The screen holds briefly and then hands over to the scenes on its own.
- If the address is lost for a moment while the screen is up, the screen keeps showing the last address rather than
  jumping back a step. The one exception is a setup address the device is reached at over its cable: a cable pulled for
  more than a moment takes the address off the screen, and it returns when the cable does. An address the device got
  over Wi-Fi stays up, whether or not it also has a cable to lose.
- On products with an IP-report button, a short press brings this screen back at any later time; see
  [Physical Buttons](physical-buttons.md).

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
