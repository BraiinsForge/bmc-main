# Software Upgrades

Braiins Deck updates its firmware and applications through one over-the-air upgrade flow. Users do not have to manage
these layers separately: the device checks what is available, chooses a compatible combination, and communicates the
interruption the upgrade requires.

## User stories

### Upgrade all device software from one place

> As a user, I want to check for and install the latest available software from one place so I do not have to manage
> firmware and application updates separately.

- The system upgrade page checks for both firmware and application updates.
- When an update is available, the user sees what will change, the download size, and the available release notes before
  starting it.
- Firmware and application updates that belong together are presented and installed as one system upgrade.
- Download and installation progress remains visible until the upgrade applies or the device begins restarting.

### Keep a sporadically connected device current

> As a user, I want automatic updates to keep looking for the latest software whenever my device is in use, so it still
> stays current when it is normally switched off or disconnected overnight.

- Initial device setup enables automatic updates; the user can turn them off or back on from the system upgrade page.
- An enabled device checks every two hours rather than relying on a single nightly maintenance window.
- Each device staggers its checks, so many devices do not contact the update service at the same instant.
- A newly configured device checks once as soon as setup finishes instead of waiting for its first scheduled check.
- Temporary update-service or network failures are retried. If retries do not succeed, the device checks again at a
  later scheduled opportunity.

### Receive compatible firmware and application updates together

> As a user, I want firmware and application updates to stay compatible so an upgrade leaves the whole device on a
> working set of software.

- An application-only update can be installed without waiting for a new firmware release.
- When firmware and applications both need updating, the device installs the application versions intended for the new
  firmware as part of the same operation.
- An update never replaces an installed application with an older version merely because the firmware changes.

### Understand the interruption before upgrading

> As a user, I want to know whether an upgrade will briefly restart the application or reboot the whole device so I can
> start it at a convenient time.

- The upgrade page communicates the expected interruption before a manual upgrade starts.
- An application-only update can restart the application without rebooting the device.
- A firmware update reboots the device and activates its accompanying application updates as the new firmware starts.
- The interface explains that the device is restarting while the connection is unavailable.

## Constraints

- Checks and downloads require the device to be powered on and able to reach its configured update services. Automatic
  updates do not make an offline device upgrade immediately when connectivity returns; the next retry or scheduled check
  provides another opportunity.
- Scheduled automatic checks do not run during the first 30 minutes after a normal startup. This avoids competing with
  startup work; the one-time check after initial setup is exempt.
- Automatic updates install available upgrades without asking for confirmation. Turning automatic updates off leaves
  manual checking and installation available.
- A firmware upgrade requires a full device reboot. The exact interruption for an application-only update is reported by
  the upgrade offer rather than inferred solely from the update type.
