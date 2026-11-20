# Home Assistant Tile Widget

The Home Assistant Tile shows one entity from the user's Home Assistant and lets them operate it from the Deck, the way
the Tile card does on a Home Assistant dashboard. The current version covers the entities that are pressed (rather than
switched): buttons, scenes and scripts.

## User stories

### Connect the tile to Home Assistant

> As a Home Assistant user, I want to point a Deck widget at one of my entities, so that I can reach it without opening
> a dashboard.

- The user saves a Home Assistant account once, with the instance's address and a long-lived access token, and binds it
  to the tile. See [Credential Accounts](../credential-accounts.md).
- The tile asks for the entity ID, such as `button.garage_gate` or `scene.evening`, and refuses to save without one. Its
  settings say which types it supports and where Home Assistant shows an entity's ID.
- The tile shows the entity's name from Home Assistant, unless the user sets a name of their own.
- A change made in Home Assistant, such as a press from its dashboard, shows on the Deck within about a second, without
  the user doing anything.

### Press a button or activate a scene

> As a user, I want to tap the tile to press a button or activate a scene, so that I can trigger it from where I stand.

- The whole tile is the button: a large icon over the name and a line that tells when it was last used, as Home
  Assistant's own tile does, such as _5 minutes ago_, or _Never pressed_. The time is worded in the device language and
  moves on by itself.
- A finger on the tile tints it and sinks the icon, like a pushed button. Lifting the finger fires the action; sliding
  it off, or swiping to another scene, fires nothing.
- While the press is on its way, the line reads _Pressing…_ or _Activating…_, and another tap fires nothing.
- A press that fails says so on the line for a few seconds: _Home Assistant refused_, _No answer_ or _Connection lost_.
  One that could not be sent reads _Not connected_, or _Could not send_ on a fault of the tile's own. Home Assistant's
  own message, often longer than a tile's line, goes to the device log.

### Run a script

> As a user, I want to start a script from the tile, see whether it is still running and stop it, so that I neither
> start it twice by mistake nor wonder whether it ran.

- A running script reads _Running_ with its icon in amber, whether it was started from the Deck or from anywhere else.
- A tap while it runs stops it, as Home Assistant's own toggle does, whatever the script's mode. The line reads
  _Stopping…_ until Home Assistant answers.

### Guard an action with a hold

> As a user, I want an action that runs only when I hold the tile, so that a stray touch never opens the gate.

- The _Tap action_ setting chooses whether a tap fires the action (_Toggle_) or does nothing (_Nothing_). _Toggle_ is
  Home Assistant's name for it, and does what Home Assistant's own toggle does: it presses a button, activates a scene,
  and starts or stops a script.
- The _Hold to toggle_ setting fires the action once the tile is held for 1, 2, 3 or 5 s. A ring fills around the icon
  while the finger is down, and the action fires once per hold, even if the finger stays down.
- The two settings are independent, as in Home Assistant: with both set, a tap fires the action as well. To guard an
  action, set a hold and set _Tap action_ to _Nothing_.
- Where only a hold fires the action, the tile says so after the state, such as _5 minutes ago · Hold 2 s_, and letting
  go early reads _Released too early_. On a tile one row high, a state too long for both is cut short, never the hint.
- Where neither a tap nor a hold is set, the tile reads _No action set_ and touches do nothing.

### Know when the tile cannot act

> As a user, I want the tile to tell me why it does not respond, so that I can fix it or wait.

- While the tile loads, it shows the outline of what is coming: the entity type's icon in grey and _Loading…_.
- When Home Assistant drops out, the tile keeps the last state without its colour, reads _Reconnecting…_ and takes no
  taps. It reconnects on its own.
- An entity Home Assistant reports as unavailable reads _Unavailable_, with an orange _!_ on the corner of its icon as
  in Home Assistant, and takes no taps.
- When the tile cannot show the entity at all, a warning icon replaces the entity's, with what to fix: _No entity set_,
  _Invalid entity ID_, _Type not supported yet_, _No account set_, _Token refused_, _Use an http:// address_, _Home
  Assistant too old_, _Can't connect, retrying…_, _Communication error_ or _Entity not found_.
- Orange text needs the user's attention: something to fix, or a touch that did not do what it should. A tile that
  cannot act for now drops its colour and dims its icon.

### Fit any tile size

> As a user, I want the tile to look right in every slot I put it in, so that a scene of tiles reads as one.

- On 1x1 and 2x1 tiles the icon is 80 px; on 2x2 and fullscreen tiles it is 128 px, with larger type, and _Hold N s_
  sits on a row of its own.
- A name or line too long for its row ends in an ellipsis, so neighbouring tiles keep one type size and their icons stay
  level.

## Parameters

| Setting        | Values                                   | Default |
| -------------- | ---------------------------------------- | ------- |
| Entity ID      | An entity ID, required                   | none    |
| Name           | Free text, shown instead of the entity's | none    |
| Tap action     | _Toggle_ or _Nothing_                    | Toggle  |
| Hold to toggle | _Off_, _1 s_, _2 s_, _3 s_ or _5 s_      | Off     |

## Credentials

| Slot | Label          | Used for                                                                     |
| ---- | -------------- | ---------------------------------------------------------------------------- |
| `ha` | Home Assistant | The instance's address and the long-lived access token the tile logs in with |

Create the token under a Home Assistant user without administrator rights: the token can do everything its user may.

## Setup

To verify the tile, a test Home Assistant instance needs:

- an `input_button` helper, and a `button` from the `demo` integration;
- two scenes, one of them never activated;
- a script with a `delay: 10` step;
- a non-administrator user with a long-lived access token.

In the Deck web app, save two Home Assistant accounts: one with that token and one with a wrong token.

## Verification

- **Configuration:** adding the tile opens its settings with the entity ID required; closing them unsaved leaves _No
  entity set_ on the Deck; a new entity ID switches the tile without a restart.
- **Each type:** a tap on the `input_button`, the demo button and a scene fires it once, as Home Assistant's logbook
  shows, and the line tells when it was last used. A press made in Home Assistant shows on the Deck.
- **Scripts:** the script, started from the tile and from Home Assistant, reads _Running_, and a tap while it runs stops
  it.
- **Touch:** a second tap while the press is on its way fires nothing; a swipe that starts on the tile changes the
  scene, fires nothing and leaves no tint; each tap and hold combination behaves as described above. With a 1 s hold and
  _Tap action_ set to _Nothing_, a hold let go early and then held through at once reads _Pressing…_ in place of
  _Released too early_. A setting saved while a finger stays on the tile after its hold fired fires nothing more.
- **Sizes:** the tile at every size, with a long name.
- **Problems:** each message above, by unbinding the account, entering a malformed ID, an unsupported type and a missing
  entity, binding the wrong-token account, stopping Home Assistant before and after the tile starts, and disabling an
  entity's integration. _Home Assistant too old_, _Communication error_ and _Use an http:// address_ need an instance
  older than 2024.3, a reply larger than the device takes, or an `https://` account the web app refuses to save; the
  gallery's screens capture shows them.

## Constraints

- BMC100 only, at every widget size: 1x1, 2x1, 2x2 and fullscreen.
- Buttons, input buttons, scenes and scripts only; other entity types read _Type not supported yet_ until the tile
  supports them.
- Home Assistant 2024.3 or newer, reached over plain `http://`; encrypted connections are not supported yet.
- Each type is drawn with its default icon rather than the entity's own.
- The tile's words are English.
