# Home Assistant Tile Widget

The Home Assistant Tile shows one entity from the user's Home Assistant and lets them operate it from the Deck, the way
the Tile card does on a Home Assistant dashboard. The current version covers the entities that are pressed: buttons,
scenes and scripts; those that are switched on and off: switches, input booleans, automations, sirens and remotes; and
lights.

## User stories

### Connect the tile to Home Assistant

> As a Home Assistant user, I want to point a Deck widget at one of my entities, so that I can reach it without opening
> a dashboard.

- The user saves a Home Assistant account once, with the instance's address and a long-lived access token, and binds it
  to the tile. See [Credential Accounts](../credential-accounts.md).
- The tile asks for the entity ID, such as `scene.evening` or `light.kitchen`, and refuses to save without one. Its
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
  One that could not be sent reads _Not connected_, _Too many taps_ while too many calls are still on their way, or
  _Could not send_ on a fault of the tile's own. Home Assistant's own message, often longer than a tile's line, goes to
  the device log.

### Run a script

> As a user, I want to start a script from the tile, see whether it is still running and stop it, so that I neither
> start it twice by mistake nor wonder whether it ran.

- A running script reads _Running_ with its icon in amber, whether it was started from the Deck or from anywhere else.
- A tap while it runs stops it, as Home Assistant's own toggle does, whatever the script's mode. The line reads
  _Stopping…_ until Home Assistant answers.

### Switch something on or off

> As a user, I want to see whether a switch is on and flip it from the tile, so that I can turn the kettle on from where
> I stand.

- The tile looks like a button's: a large icon over the name, with the position in the line: _On_, _Off_, or _Unknown_
  when Home Assistant does not know it.
- While on, the icon sits on its tinted disc, amber, or red for a sounding siren. While off, the disc is gone and the
  icon is grey, so the position shows without telling colours apart. The icon follows the position too: a switch and an
  input boolean wear a toggle, filled while on and outlined while off, or a plug for an outlet; the rest wear Home
  Assistant's own icons. Only a siren keeps its bullhorn either way.
- A tap on the tile flips it, as on a button. A tile in no known position looks off, with a _?_ on a grey disc at the
  corner of its icon, and a tap turns it on.
- The tile flips at once. Whatever Home Assistant reports next shows instead, and when it reports nothing within 2 s of
  answering, the tile goes back to the position it last reported, so a device that ignores the call does not stay shown
  flipped.
- A second tap before Home Assistant reports flips it back.
- A flip that fails says so on the line, as a press does, even when a later tap or a report has replaced it. Taps that
  Home Assistant leaves unanswered pile up: past a handful, another reads _Too many taps_, and a flip already on its way
  stays shown.
- An automation is on while enabled: the tile enables and disables it, and never runs it.

### Switch and dim a light

> As a user, I want to switch a light on and off and set its brightness and colour from the tile, so that I need not
> reach for my phone.

- The card is the light's switch: a tap turns it on or off, under the same tap and hold settings as a press. The bulb is
  struck through while the light is off.
- A flip shows at once, as Home Assistant's own toggle does. Home Assistant's next report replaces it, and when none
  comes within 2 s of the answer, the tile shows the position Home Assistant reported last.
- The line reads _On_ or _Off_, or the brightness, such as _60%_, on a tile with no brightness bar. A light whose
  position Home Assistant does not know reads _Unknown_, with a _?_ on the corner of its icon, and a tap turns it on.
- Where the tile has room, rows of controls sit beside or under the name, in this order: brightness − and + around a
  bar, the colour temperature, the hue, and brightness presets of 25, 50, 75 and 100 %. Each shows only for a light that
  takes it, and a tile with less room drops the last: 1x1 shows none, 2x1 two, 2x2 three and fullscreen all four.
- − and + step the brightness to the next ten percent. − from 10 % or below turns the light off, and + turns a light
  that is off back on at the brightness it had. A tap on the bar sets the brightness under the finger, never below 1 %,
  so the bar never turns the light off.
- The colour temperature and hue bars are painted in their colours, with a cursor at the light's value: on the colour
  temperature while the light shows a white, on the hue while it shows a colour. − and + move a tenth of the light's
  range or 30° of hue, and a tap sets the value under the finger. A hue goes out fully saturated.
- A run of − and + taps goes to Home Assistant once the taps stop, drawn faint until Home Assistant confirms it; a tap
  on a bar or a preset goes at once. A colour temperature replaces a hue still waiting to go, and the other way round,
  and turning the light off drops whatever still waits.
- A change that fails reads as a failed press does, and the tile returns to what Home Assistant reports.
- The icon wears the light's colour or colour temperature, made readable as Home Assistant's own badge makes it: a pale
  colour more vivid, a white one grey. A light that reports neither is amber, as are the brightness bar and presets,
  where white would vanish under white text.

### Guard an action with a hold

> As a user, I want an action that runs only when I hold the tile, so that a stray touch never opens the gate.

- The _Tap action_ setting chooses whether a tap fires the action (_Toggle_) or does nothing (_Nothing_). _Toggle_ is
  Home Assistant's name for it, and does what Home Assistant's own toggle does: it presses a button, activates a scene,
  starts or stops a script, and switches a switch or a light.
- The _Hold to toggle_ setting fires the action once the tile is held for 1, 2, 3 or 5 s. A ring fills around the icon
  while the finger is down, and the action fires once per hold, even if the finger stays down.
- The two settings are independent, as in Home Assistant: with both set, a tap fires the action as well. To guard an
  action, set a hold and set _Tap action_ to _Nothing_.
- Where only a hold fires the action, the tile says so after the state, such as _5 minutes ago · Hold 2 s_, and letting
  go early reads _Released too early_. On a tile one row high, a state too long for both is cut short, never the hint.
- Where neither a tap nor a hold is set, the tile reads _No action set_ and touches do nothing. A switch's or light's
  tile instead keeps showing its position, as a status that touches on the card do not change; a light's rows still act.

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
  cannot act for now drops its colour and dims its icon; a switch's or light's tile set to neither tap nor hold, a
  status that takes no touch on the card anyway, keeps its icon undimmed.

### Fit any tile size

> As a user, I want the tile to look right in every slot I put it in, so that a scene of tiles reads as one.

- On 1x1 and 2x1 tiles the icon is 80 px; on 2x2 and fullscreen tiles it is 128 px, with larger type. A tile without
  controls puts _Hold N s_ on a row of its own there.
- Controls sit beside the name on a 2x1 or fullscreen tile and under it on a 2x2 tile, which keeps the large icon only
  while the controls fit under it.
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
- an `input_boolean` helper, an automation, and the `demo` integration's switches (one of them an outlet), siren and
  remote;
- three template switches: one fixed to off whose `turn_on` and `turn_off` do nothing, for a device that ignores the
  call; one whose `turn_on` and `turn_off` call a script that does not exist, so Home Assistant refuses them; and one
  that reports an `input_boolean` it turns on or off after a 3 s delay, for a slow device;
- the `demo` integration's lights, among them one that takes both a colour temperature and a colour; and three template
  lights: one that only switches; one that dims, whose `turn_on`, `turn_off` and `set_level` call a script that does not
  exist, so Home Assistant refuses them; and one that dims, reporting an `input_boolean` and an `input_number` it sets
  after a 3 s delay, for a slow device;
- a non-administrator user with a long-lived access token.

In the Deck web app, save two Home Assistant accounts: one with that token and one with a wrong token.

## Verification

- **Configuration:** adding the tile opens its settings with the entity ID required; closing them unsaved leaves _No
  entity set_ on the Deck; a new entity ID switches the tile without a restart.
- **Each type:** a tap on the `input_button`, the demo button and a scene fires it once, as Home Assistant's logbook
  shows, and the line tells when it was last used. A press made in Home Assistant shows on the Deck.
- **Scripts:** the script, started from the tile and from Home Assistant, reads _Running_, and a tap while it runs stops
  it.
- **Switches:** a tap and a hold flip each switch type, the icon and its disc at once, and the siren's disc is red; a
  flip made in Home Assistant shows on the Deck; two quick taps on the slow switch end in the position the second asked
  for; the switch that ignores the call springs back 2 s after a tap, and the one Home Assistant refuses springs back
  reading _Home Assistant refused_. A tap on the automation disables it, and its traces show no run. A switch set to
  `unknown` in the developer tools reads _Unknown_ with a grey _?_, and a tap turns it on. With Home Assistant paused,
  taps pile up until one reads _Too many taps_. With neither a tap nor a hold set, the tile shows its position undimmed
  and touches flip nothing.
- **Touch:** a second tap while the press is on its way fires nothing; a swipe that starts on the tile changes the
  scene, fires nothing and leaves no tint; each tap and hold combination behaves as described above. With a 1 s hold and
  _Tap action_ set to _Nothing_, a hold let go early and then held through at once reads _Pressing…_ in place of
  _Released too early_. A setting saved while a finger stays on the tile after its hold fired fires nothing more.
- **Lights:** the card switches each light on and off, and a change made in Home Assistant shows on the Deck. −, +, the
  bar and the presets set the brightness, and a run of taps sends one call, as Home Assistant's logbook shows. The
  colour temperature and hue bars set the light, and the icon follows its colour. The light that only switches shows no
  controls. The gallery's light capture shows the _Unknown_ state.
- **Slow and refused lights:** on the slow light, + shows the new brightness faint until Home Assistant reports it; on
  the one Home Assistant refuses, a preset springs back reading _Home Assistant refused_.
- **Sizes:** the tile at every size, with a long name, and a colour light with the rows each size fits.
- **Problems:** each message above, by unbinding the account, entering a malformed ID, an unsupported type and a missing
  entity, binding the wrong-token account, stopping Home Assistant before and after the tile starts, and disabling an
  entity's integration. _Home Assistant too old_, _Communication error_ and _Use an http:// address_ need an instance
  older than 2024.3, a reply larger than the device takes, or an `https://` account the web app refuses to save; the
  gallery's screens capture shows them.

## Constraints

- BMC100 only, at every widget size: 1x1, 2x1, 2x2 and fullscreen.
- Buttons, input buttons, scenes, scripts, switches, input booleans, automations, sirens, remotes and lights only; other
  entity types read _Type not supported yet_ until the tile supports them.
- A switch whose state Home Assistant only assumes (`assumed_state`), such as a radio outlet, is flipped like any other:
  the tile cannot send _on_ to one it shows on.
- A light's colour is set by its hue alone, at full saturation; its saturation, effects and favourite colours cannot be
  set from the tile yet.
- Home Assistant 2024.3 or newer, reached over plain `http://`; encrypted connections are not supported yet.
- Each type is drawn with the tile's icon for that type rather than the entity's own.
- An entity Home Assistant reports as unavailable keeps its type's icon under the orange _!_, the same for every type,
  though Home Assistant gives an unavailable automation an icon of its own.
- The tile's words are English.
