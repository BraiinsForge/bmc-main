# Changelog

This changelog covers the display application, widgets, and web UI delivered as packages. Braiins OS firmware has its
own release notes in `bos-main`. Features vary by device.

## 26.10

### Highlights

- You can now upload third-party widgets through the web in given format.
- BMM101 now runs the same widgets as Braiins Deck, with layouts adapted to its display.
- New Fleet Management widget that automatically discovers BOS and AxeOS miners on the local network.
- Widget animations are now available on the devices, in some cases approaching 30 frames per second on the Deck.
- Widgets can declare lists in their settings.

### Braiins Mini Miner 101 support

- BMM101 now runs the display application and widgets from this repository. Its screen cycles through a clock, a Bitcoin
  price ticker, three Miner Info views, and Bitcoin Mining Data by default.
- BMM101's web now has dedicated "Display" menu item with the same page for configuring display as the Deck has.
- Custom widgets may be added, the BMM101 is now extensible!
- All widgets from the Deck are available on BMM101.

### Widgets and display

- On the Deck, you can upload a widget package file (`.nar`) from the Display Widgets page. The device checks the file
  and reports whether the widget was installed. Only plain WebAssembly widgets are allowed to be installed this way.
  Other packages, such as system components, still have to be installed through ssh.
- Widget settings can now include lists that you add to, remove from, and reorder. Invalid entries are marked in the
  form. Explanations for on/off settings are now shown too.
- Text that does not fit can be clipped or shortened with an ellipsis.
- Hashrate and power values scale to readable units.
- Widgets can now use animations.
- ISS Position's globe is smoother, and several rendering fixes reduce flicker during scene changes and overlays.
- The analog clock widget animates a transition on its hand.

#### Fleet management widget

A new widget is available for viewing statistics of your local miner fleet.

- Automatically discovers BOS and AxeOS miners on the local network. BOS miners need a saved account for their data;
  AxeOS miners need no credentials.
- Shows total hashrate, power, efficiency, temperature, and miner health for the fleet.
- Groups miners by model and lets you open a model or individual miner for more detail.
- Shows recent hashrate trends for the fleet and individual miners.

### Fixes to everyday use

- On the Deck, you can set alarms for the same time on different days and remove an alarm while it is snoozed. The
  snooze menu no longer gets cut off by the alarm form.
- In auto-off the screen stays on with finger held on it.
- The night mode state does not stay invalid after clock synchronizes after boot.
- Financial tickers show large prices without unnecessary decimal places.
- When Deck changes its Wi-Fi network from the web UI, it shows connection progress on its screen.
- Checking for an upgrade in another browser tab no longer cancels the upgrade you were about to start. If the initial
  application download during a firmware upgrade is interrupted, it can continue from the bytes already downloaded.
- The Deck Upgrades page shows the version of the installed application packages, separately from the firmware version.
- The random facts widget uses an in-house source that no longer contains any inappropriate facts.
- Formula 1 widget will show points of drivers in "Driver Standings" view instead of 0.

The [26.10 technical release notes](docs/releases/26.10.md) describe the implementation and maintenance work behind
these changes.
