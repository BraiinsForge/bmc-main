# Credential Accounts

Saved accounts let the user enter a service credential once — a Braiins Pool token, an API key, a username and password
pair — and hand it to any widget that needs one. The widget can tell that a credential is available and which account it
came from. A WASM widget never sees the secret itself: the device attaches it to the widget's outgoing requests.

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
  in any form. The device's own runtime holds it and puts it into the widget's outgoing requests at the moment each one
  leaves.
- A native widget is part of the device's firmware. It would receive the secret directly, and is trusted the same way
  the rest of the firmware is.

### Send a credential only where it belongs

> As a user, I want a credential that belongs to one service to be useless against any other, so that a misbehaving or
> compromised widget cannot forward my token somewhere else.

- Some kinds of account are tied to the service they belong to — a Braiins Pool token can only be sent to Braiins.
- A request that would carry such a credential anywhere else is refused outright rather than sent without it.
- For a kind that is not tied to a service, the user may list the destinations their account allows — one per line, a
  host, a wildcard like `*.example.com`, or an address range. The same refusal then applies to anywhere else.
- Leaving that list empty keeps the credential usable with whatever destination the widget asks for.

### Rotate or revoke a credential and have it take effect at once

> As a user, I want editing or removing an account to take effect immediately, so that rotating a leaked token actually
> stops it being used.

- Saving a new value reaches every widget using that account straight away, with nothing to restart.
- Taking an account away from a widget removes its access immediately.
- An account still in use cannot be deleted; the device names the widgets holding it so the user can free it first.
- A widget whose credential disappears returns to the same state as one that never had it, rather than failing
  obscurely.

### Share diagnostics without sharing credentials

> As a user, I want to send a support archive to Braiins without handing over my credentials with it.

- Saved credentials are kept apart from the rest of the device's configuration, in a file only the device's
  administrator can read.
- The support archive never contains that file.
- Credential values never appear in the device's logs, including when logging is turned up for troubleshooting.

## Constraints

- This version offers three kinds of account: a single token, a username and password pair, and a Braiins Pool token.
  New kinds require a firmware update.
- A widget declares which kind of account each of its credentials accepts, and only matching accounts can be bound.
- Credential values are stored unencrypted on the device. They are protected by file permissions and by being kept out
  of support archives and logs, not by encryption; anyone with administrative access to the device can read them.
- Restricting an individual account to a narrower set of destinations than its kind allows is not offered in this
  version.
