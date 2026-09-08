# Physical Buttons

The Deck's physical buttons let the user act on the device without its screen or the web UI: an IP-report button puts
the device's address on screen, and a reset button restarts or factory-resets the device. Which buttons a product
carries, and which of them the device software answers to, depends on the product.

## User stories

### Put the address back on screen

> As a user, I want a short press of the IP-report button to show the device's address so that I can open the web UI
> without looking the address up, long after the boot screens have gone.

- A short press, released within one second, shows the same address screen and QR code the device shows after a boot,
  for the same ten seconds, and then the scenes return.
- If the display is off, the press wakes it first, and the address screen is what comes up.
- When the device has no address, the press answers with the "no address" screen instead, so a press is never ignored in
  silence.
- The address shown is the current one, including an address that changed while nothing was on screen.
- Pressing again while the screen is up restarts its ten seconds.
- During a boot's connect screen, or the update confirmation that precedes it, the press does nothing: the address is
  about to be shown anyway. During setup or Wi-Fi reconfiguration it does nothing either, since those screens must stay.
- A failure screen the user could tap away closes on the press, and the address takes its place. On a device without a
  touch screen this is the way to clear such a screen before it times out on its own.
- A hold of one second or longer does nothing today. Turning the display off on a hold of three seconds or longer is a
  follow-up (BDK-816).

### Restart or factory-reset the device

> As a user, I want the reset button to restart the device on a short press and factory-reset it on a long hold so that
> I can recover a device I cannot reach any other way.

- A press released within two seconds restarts the device.
- A hold of five seconds or longer factory-resets it: the device wipes its configuration and comes back up in setup,
  showing its setup network. The installed firmware stays.
- A release between two and five seconds does nothing, so a hesitant hold cannot pick either action by accident.
- On BMM100 and BMM101 the reset button belongs to the mining firmware; the device software leaves it alone (BDK-797).

## Constraints

- Buttons arrive as kernel button events, and a button is handled wherever the kernel reports it; no product list gates
  the handling. BMM100 and BMM101 carry an IP-report button today.
- The mining firmware reads the same IP-report button and sends the IP-report network packet on the same short press;
  the one-second bound is its own, so the press that shows the address is the press that sends the packet. Neither
  application relays the button to the other, and the device software never sends the packet itself.
- The address screen the button raises behaves like the boot connect screens in
  [Device Setup & Connect Screens](device-setup-screens.md): a tap dismisses it where the device has touch, and a firing
  alarm, the upgrade screens and the settings tray draw above it.
- Button presses count as activity for [Night Mode](night-mode.md)'s screen auto-off: they wake a dark screen and
  restart its timeout.
