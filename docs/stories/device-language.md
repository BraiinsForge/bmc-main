# Device Language

The language the device speaks, for its display and its web UI.

## User stories

### Pick the language in the web UI

> As a device owner, I want to choose the language my device speaks, so that its display and its web UI are in a
> language I read.

- The general settings list English, as British or American, and every language the BMC application stack ships, each
  named in itself and with its flag where it has one.
- A pick is confirmed in the new language, and the display and the web UI switch to it at once.

### Switch it during the first setup

> As a new owner, I want to switch the language on the device's screen, so that the rest of the setup is in it.

- The first setup's instruction screens show a switch: each language's flag, or its name where it has none.
- A tap sets the same setting the web UI does. A device already set up offers no switch.

### Keep it

> As a device owner, I want my choice to stay, so that I do not pick it again.

- The language survives reboots and upgrades; a fresh device starts in British English.
- A device upgraded from a BMC application stack without the setting stays in English: American if its date layout was
  M/D/YYYY, British otherwise.

## Constraints

- The languages sit behind the marker file `/etc/bmc/i18n`, which no device has by default; without it the device
  behaves as the BMC application stack did before them.
- The web UI follows the marker at once, the device's screens after the device's application restarts.
