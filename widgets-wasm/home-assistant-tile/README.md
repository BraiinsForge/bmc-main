# home-assistant-tile

The Home Assistant Tile widget: one entity as a card, with the controls its size has room for, after the frontend's tile
card. It talks to Home Assistant through the [`home-assistant`](../lib/home-assistant/README.md) library.

## Layout

| Module        | What it is                                                                                                  |
| ------------- | ----------------------------------------------------------------------------------------------------------- |
| `fixtures`    | Sizes, settings and contexts to stage a tile at, shared by the gallery scenes and the tests.                |
| `interaction` | What lasts between frames: requests Home Assistant has yet to confirm, the card's tap and hold, the notice. |
| `present`     | One module per group of domains, each a tile built from the entity; `Group` maps the library's `Domain`s.   |
| `view`        | The card and the blocks, drawn from plain values, with no Home Assistant types.                             |
| `lib.rs` glue | Follows the entity, sends requests, settles them on answers and reports. It names no domain.                |

## Words

- **Domain** is Home Assistant's: `light`, `scene`. The library's `Domain` enum lists the ones it knows.
- **Group** is the Tile's: domains that share a tile, such as `momentary` for button, input_button, scene and script.
- **Card action** is what a tap or a hold on the card fires, under the tap and hold settings.
- **Slot** is where a value the user asked for is shown until Home Assistant confirms it, such as a brightness.

## A group's tile

Each group module splits its tile into up to three types:

- `Kind`, the domains it covers, and what never changes for one: its services, its copy, its icon and colour. A group of
  one domain has none;
- `State`, what Home Assistant reports of an entity now, with `State::read(kind, &entity)`;
- `Tile`, its `Presenter` for one frame, built with `Tile::new(kind, &state, &context)`. It keeps what `view` and
  `touched` need, the kind among them when a block calls a service.

A `Tile` gets a `Context`: what is pending, the gesture, the name, any notice, the `Reach` and the room. It builds its
blocks with `enabled: context.can_act()`, its card action with `present::card_action`, and its card with
`present::card`. The card says "Reconnecting…" or "Unavailable" in place of the state and drops its colour, so a tile
handles neither itself. `present::showing` decides whether a frame shows the tile at all: until the entity's first state
arrives, the glue draws the group's skeleton with `present::loading`, and any problem with `present::problem`, both on
the same `Card::centred` layout. A time on the line, such as a button's last press, is a `Line::Since`, which the host
words in the device language and keeps current.

```rust
pub trait Presenter {
    fn view(&self, touches: &Touches) -> Node;
    fn touched(&self, touches: &Touches) -> Option<Request>;
    fn card_action(&self) -> CardAction;
}
```

A request comes from `Request::flip`, `number`, `choice` or `no_value`, which also say when it ends: on the call's
answer, or on Home Assistant's next report for an on or off.

## Adding a domain

1. Library: `domain/<name>.rs` with its `ATTRIBUTES`, readers and service builders, named after the service. Test the
   readers on an entity built with `Entity::tracked(state, attributes, ATTRIBUTES)`.
2. Library: a `Domain` variant, with its arms in `Domain::of` and `Domain::attributes`.
3. Tile: a `Kind` in the group it joins, or a new `present/<group>.rs` with `State` and `Tile`, a `Kind` once it covers
   two domains, its slots as `Slot` consts and its icons from `assets/mdi/`.
4. Tile: its arm in `Group::of`, and for a new group a `Group` variant and its arms in `Group::icon` and `Group::tile`.
   The compiler points at each.
5. Gallery: `gallery/home_assistant_tile_<group>.scene.rs` on the shared `fixtures`, its
   `home-assistant-tile-<group>.capture.toml`, and one more scene in `EXPECTED_SCENE_COUNT` in
   `scripts/check_gallery_lints.py`.
6. `just validate`, then `just wasm::scenes home-assistant-tile` for the shots.

The tests and the gallery build entities and call IDs with the library's `testing` feature.
