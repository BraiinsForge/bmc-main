# Credential Accounts

Saved accounts let the user enter a service credential once — a Braiins Pool token, an API key, a username and password
pair, a Home Assistant instance — and hand it to any widget that needs one. The widget can tell that a credential is
available and which account it came from. A WASM widget never sees the secret itself: the device attaches it to the
widget's outgoing requests and connections.

## User stories

### Save a credential once and reuse it

> As a user, I want to enter a credential once and use it for several widgets, so that I do not retype the same token
> every time I add a widget.

- The user creates a named account by choosing its kind and filling in its fields.
- Secret fields are masked while being typed and are never shown again afterwards.
- One account can be used by as many widgets as the user likes.
- The account list shows which widgets are currently using each account.

### Give a widget the account it should use

> As a user, I want to choose which of my accounts a widget uses, so that the right widget talks to the right service
> under the right identity.

- A widget that needs a credential offers one picker per credential it asks for, labelled with what that credential is
  for.
- Each picker offers only the accounts of the kind that slot accepts, so an account cannot be bound to the wrong place.
- Choosing "None" leaves the slot empty.
- A widget missing a credential it needs is flagged while editing, but the user can still save and come back to it
  later.

### Keep the secret away from the widget

> As a user, I want my token to stay on the device, so that adding a widget does not mean trusting whoever wrote it with
> my credentials.

- A widget can see that a slot is filled, which kind of account filled it, and the name the user gave that account.
- For WASM widgets — the kind an outside author writes — the secret value itself is never handed to the widget's code,
  in any form. The device's own runtime holds it and puts it into the widget's outgoing requests and messages at the
  moment each one leaves.
- A native widget is part of the device's firmware. It would receive the secret directly, and is trusted the same way
  the rest of the firmware is.

### Send a credential only where it belongs

> As a user, I want a credential that belongs to one service to be useless against any other, so that a misbehaving or
> compromised widget cannot forward my token somewhere else.

- Some kinds of account are tied to the service they belong to — a Braiins Pool token can only be sent to Braiins, and a
  Home Assistant token only to the address saved with it.
- A request that would carry such a credential anywhere else is refused outright rather than sent without it. A message
  on an open connection is refused the same way, and the connection is closed.
- For a kind that is not tied to a service, the user may list the destinations their account allows — one per line, a
  host, a wildcard like `*.example.com`, or an address range. The same refusal then applies to anywhere else.
- Leaving that list empty keeps the credential usable with whatever destination the widget asks for.

### Rotate or revoke a credential and have it take effect at once

> As a user, I want editing or removing an account to take effect immediately, so that rotating a leaked token actually
> stops it being used.

- Saving a new value reaches every widget using that account straight away, with nothing to restart.
- A widget holding an open connection that was authenticated with the old value is disconnected, so the old value stops
  working at once; the widget reconnects with the new one.
- Taking an account away from a widget removes its access immediately.
- Deleting an account still in use first names the widgets holding it; once the user confirms, it is removed from every
  one of them and then deleted.
- A widget whose credential disappears returns to the same state as one that never had it, rather than failing
  obscurely.

### Connect widgets to Home Assistant

> As a user, I want to give widgets access to my Home Assistant instance, so that they can show and control my home.

- The user saves a Home Assistant account with the address they open Home Assistant at and a long-lived access token
  created in Home Assistant.
- An address that is not an `http://` address, or that has a path after the host and port, is refused when saving. A
  trailing slash is dropped.
- The token is only ever sent to that address, so the account takes no list of other destinations.

### Share diagnostics without sharing credentials

> As a user, I want to send a support archive to Braiins without handing over my credentials with it.

- Saved credentials are kept apart from the rest of the device's configuration, in a file only the device's
  administrator can read.
- The support archive never contains that file.
- Credential values never appear in the device's logs, including when logging is turned up for troubleshooting.

## Constraints

- This version offers five kinds of account: a single token, a username and password pair, a Braiins Pool token, a local
  file token, whose value the device reads from a file another service writes (used for the miner's own BOS API), and a
  Home Assistant instance. New kinds require a firmware update.
- Widgets reach Home Assistant's live updates over an unencrypted connection only, so an account takes an `http://`
  address; encrypted `https://` addresses are not supported in this version.
- Miner-attached products (BFM100, BMM100, BMM101) start with a *Local BOS API* account of the local file token kind,
  pointed at the Boser API token file and restricted to `localhost`. It is an ordinary account afterwards: it can be
  renamed, re-pathed or deleted, and a deleted one is not recreated.
- A widget declares which kind of account each of its credentials accepts, and only matching accounts can be bound.
- Credential values are stored unencrypted on the device. They are protected by file permissions and by being kept out
  of support archives and logs, not by encryption; anyone with administrative access to the device can read them.
- Restricting an individual account to a narrower set of destinations than its kind allows is not offered in this
  version.
- Changing any field of an account means entering all of them again, so a new Home Assistant address needs the token
  too.
