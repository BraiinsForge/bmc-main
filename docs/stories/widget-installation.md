# Widget Installation

The backend is prepared for installing widget packages that are available but not yet installed. It can discover
installable widgets, check and plan an installation, and run it through the same upgrade flow used for application and
firmware upgrades.

The frontend integration is not implemented yet. A future frontend change will expose this capability in the "Add a
widget" picker and provide the user experience described below.

## Planned frontend user stories

### Discover installable widgets in the picker

> As a user, I want available-but-not-installed widgets to show up in the same "Add a widget" picker as my installed
> widgets so I can find and add new widgets in one place.

- Not-yet-installed widget packages appear in the picker alongside installed widgets, each with its name, icon, and
  category.
- Not-installed entries are visually marked as such, so the user can tell at a glance what still needs installing.
- The picker metadata and the static preview image come from the package catalog, so a widget can be shown before it is
  installed.

### Keep the picker usable without internet

> As a user, I want the "Add a widget" picker to keep working when the device is offline so I can still manage the
> widgets I already have.

- Installed widgets are always shown in the picker, with or without an internet connection.
- When the device cannot reach the package catalog, installable (not-yet-installed) widgets are simply left out — the
  picker does not error or block.
- Once connectivity returns, the installable widgets appear in the picker again.

### Preview a widget before installing it

> As a user, I want to see what a widget looks like and what installing it involves before I commit, so there are no
> surprises.

- Tapping a not-installed widget opens a preview panel — not a dialog — with a static preview image of the widget (it
  cannot run yet).
- The panel offers a single primary action derived from the install check: **Install** when nothing else is needed, or
  **Upgrade & install** when an upgrade must come along.
- When an upgrade is required, the panel names what is included — "requires an application upgrade" or "requires a
  firmware upgrade" — based on the plan the device returned.
- A disruption warning is shown independently of the upgrade type: a brief screen restart or a full device reboot,
  driven solely by the device's reported disruption.

### Confirm what will happen

> As a user, I want a clear confirmation step that spells out the consequences before anything is downloaded or applied.

- Confirming opens a dialog detailing the download size, the changed packages when an application upgrade is included,
  and the firmware version when a firmware upgrade is included.
- The same disruption warning (screen restarts briefly / device reboots) is repeated on the confirmation step.
- The user can abort at the preview panel or the confirmation dialog at any time, and nothing on the device changes.

### Install seamlessly, even across a restart or reboot

> As a user, I want the widget to just appear after I confirm, even if the device has to restart or reboot to get there.

- After accepting, download and apply progress is shown live.
- If the install involves a brief application restart or a full firmware reboot, the connection drops with an
  explanatory overlay and then returns on its own.
- After reconnecting, the pending add resumes: the widget ends up installed, added to the scene, and previewed live.
- If the session was closed or reloaded during the restart, the user simply returns to a normal view and the widget
  shows as installed in the picker.

## Constraints

- Uninstalling widget packages and any generic (non-widget) package management UI are out of scope.
- Installing together with a firmware upgrade happens in a single run — nothing is persisted across the reboot. The
  configured feed selects the package index for the exact firmware version being installed.
- The disruption warning is a distinct axis from the upgrade type. Today an application upgrade implies a brief screen
  restart and a firmware upgrade implies a reboot, but that correspondence is not guaranteed and is never assumed.
- The interactive picker experience — the preview panel, the confirmation dialog, and the reconnect-and-resume handling
  — is delivered by a follow-up (BDK-570) and is not yet implemented. This story establishes the device-side capability
  the picker builds on: discovering installable widgets with their catalog metadata and preview, checking what an
  install entails, and running the install over the shared upgrade stream.
