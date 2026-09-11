# Physical Buttons

The Deck's physical buttons let the user act on the device without its screen or the web UI: an IP-report button puts
the device's address on screen or turns the display off, and a reset button restarts or factory-resets the device. Which
buttons a product carries, and which of them the BMC application answers to, depends on the product.

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

### Turn the display off

> As a user, I want a long hold of the IP-report button to turn the display off so that I can darken the device without
> the web UI or a touch screen, and without waiting for night mode.

- The display turns off the moment the hold reaches three seconds, while the button is still down, and the address is
  not shown. Letting go afterwards changes nothing. It stays off until the next touch or press of a button the BMC
  application answers to, or until an alarm rings; the alarm screen never sits on a dark panel.
- A hold started on a dark display lights it on the press, as any press does, and darkens it again at the three-second
  mark.
- Outside night mode the display comes back on whichever scene cycling has reached meanwhile, since cycling carries on
  behind the dark panel. During night mode, where cycling is suspended, it comes back on the first scene, as it does
  after night mode's own auto-off.
- While an alarm is ringing the hold does nothing at all, so the screen the user needs to silence the alarm cannot be
  turned off from under them. The hold is refused outright rather than deferred: it does not blank the display once the
  alarm stops.
- A screen turned off this way stays off across the end of night mode, unlike one night mode's own auto-off turned off.
  Night mode's auto-off is otherwise unaffected, and resumes its timeout once the screen is woken.
- A release between one and three seconds does nothing, so a hesitant hold cannot pick either action by accident.

### Restart or factory-reset the device

> As a user, I want the reset button to restart the device on a short press and factory-reset it on a long hold so that
> I can recover a device I cannot reach any other way.

- A press released within two seconds restarts the device.
- A hold of five seconds or longer factory-resets it: the device wipes its configuration and comes back up in setup,
  showing its setup network. The installed firmware stays.
- A release between two and five seconds does nothing, so a hesitant hold cannot pick either action by accident.
- Where Boser owns the board's buttons, the reset button is its alone: the BMC application ignores the press, so the two
  never act on the same reset. Today Boser owns the buttons on BMM100, BMM101 and BFM100; BMC100 does not run Boser and
  answers the button itself.

## Constraints

- Buttons arrive as kernel button events. The IP-report button is handled wherever the kernel reports it; the reset
  button only where Boser does not own the buttons. BMM100 and BMM101 carry an IP-report button today.
- Boser reads the same IP-report button and sends the IP-report network packet on the same short press; the one-second
  bound is its own, so the press that shows the address is the press that sends the packet. Neither application relays
  the button to the other, and the BMC application never sends the packet itself.
- The address screen the button raises behaves like the boot connect screens in
  [Device Setup & Connect Screens](device-setup-screens.md): a tap dismisses it where the device has touch, and a firing
  alarm, the upgrade screens and the settings tray draw above it.
- Button presses the BMC application answers to count as activity for [Night Mode](night-mode.md)'s screen auto-off:
  they wake a dark screen and restart its timeout. A reset press on a board Boser owns does neither.
