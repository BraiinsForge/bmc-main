# Device Language

The language the device speaks, for its display and its web UI.

## User stories

### Pick the language in the web UI

> As a device owner, I want to choose the language my device speaks, so that its display and its web UI are in a
> language I read.

- The general settings list English, as British or American, and every language the BMC application stack ships, each
  named in itself and with its flag where it has one.
- A pick is confirmed in the new language, and the display and the web UI switch to it at once.

### Read the device's own screens in it

> As a device owner, I want to use the device in my native language, so that I can read everything it shows me.

- The first setup, the settings tray, the upgrade progress, the alarm, the offline indicator, a declined restart and the
  banner over a stopped widget are all worded in the device language.
- Relative times the device keeps up to date, such as "5 minutes ago", follow the language's own grammar.
- Chinese ships with the BMC application stack, and its characters draw on every screen.
- A screen that is showing when the language changes redraws in the new one.

### Switch it during the first setup

> As a new owner, I want to switch the language on the device's screen, so that the rest of the setup is in it.

- The first setup's instruction screens show a switch: each language's flag, or its name where it has none.
- A tap sets the same setting the web UI does. A device already set up offers no switch.

### Read the web UI in it

> As a device owner, I want the web UI in my language, so that I can read what it shows me and what it refuses.

- Every page opens in the device language, the Wi-Fi setup pages included.
- A widget's settings form words what it checks itself. What the device refuses, such as a missing required value, stays
  in English.
- A refused login, a wrong current password, and a refused field on the setup, network or date and number forms are
  worded in it by the device.
- A screen reader hears the controls and notifications in it.
- Chinese draws in the system's own Chinese typeface where it has one, otherwise in the device's.

### Switch it on the setup pages

> As a new owner, I want to switch the language on the web setup pages, so that the rest of the setup is in it.

- The header of the setup pages shows the current language's flag and lists every language, each named in itself.
- A pick sets the same setting as the general settings and the device's own switch.
- The switch stays while a device redoes its Wi-Fi, though the device's own screen hides its switch then.

### Read dates and figures as the device writes them

> As a device owner, I want the web UI to write dates, times and figures as my device does, so that the two agree.

- Dates, times and figures follow the device's date, time and number settings, and a change applies at once.
- The alarm pages start the week on the device's first day.

### Keep it

> As a device owner, I want my choice to stay, so that I do not pick it again.

- The language survives reboots and upgrades; a fresh device starts in British English.
- A device upgraded from a BMC application stack without the setting stays in English: American if its date layout was
  M/D/YYYY, British otherwise.
- A device set to a language it no longer ships, as when an upgrade leaves out its pack, speaks English. The web UI's
  language pickers mark the setting as no longer available and the device's own switch shows none selected, until
  another language is picked.

## Constraints

- The languages sit behind the marker file `/etc/bmc/i18n`, which no device has by default. Without it the device and
  its web UI speak English as before, though the web UI still writes dates, times and figures by the device's settings.
- The web UI follows the marker at once, the device's screens after the device's application restarts.
- A widget's own text stays as the widget writes it; only the relative times the device draws for it follow the
  language.
- Dates, times and figures keep their numeric layouts in every language, and the AM/PM marks stay in English, on the
  device and in the web UI.
- Other errors the device reports stay in English.
- Chinese is drawn at the sizes Latin text uses.
- The Chinese typeface covers GB2312, the Simplified Chinese set: a Wi-Fi name in Korean, or with Traditional characters
  or Japanese kanji outside that set, shows those as missing-glyph boxes.
