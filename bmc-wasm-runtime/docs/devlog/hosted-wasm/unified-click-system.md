# BDK-340: Unified Click System & Absolute Positioning

Status: Completed.

## Context

The widget runtime had three parallel interaction mechanisms:

1. **Button clicks** — `Vec<bool>` indexed by sequential `u32` IDs, counted during tree layout
2. **Canvas touch clicks** — `HashMap<String, TouchHit>` keyed by string
3. **Canvas touch drags** — `HashMap<String, TouchHit>` keyed by string

The index-based button system was fragile: adding or removing a button anywhere in the tree shifted all subsequent
indices. Modals made it worse — the host had to count buttons inside modal bodies to offset indices correctly. Widgets
had to track index positions manually (`result.clicks[5]`), making click handling opaque and error-prone.

Meanwhile, canvases (`touchable()`) already had a clean string-keyed system that returned position data. The two systems
duplicated host-side hit testing logic and forced widgets to use different patterns for buttons vs. interactive
canvases.

## Design

Unify everything into the canvas-style `HashMap<String, TouchHit>` system:

- Every `button!` gets a required string ID as its first argument
- Buttons and canvases share the same `clicks` and `drags` maps in `TreeRenderResult`
- Host-side hit testing uses the string ID as the interaction key for both
- Modal IDs changed from `u16` to `String` for hierarchical scoping
- Modal close buttons get scoped IDs: `{modal_id}::close`
- Modal body scroll regions get scoped IDs: `{modal_id}::body`

### Wire format change

Button serialization gains an ID prefix:

```
Before: [type:u8][style:u8][size:u8][icon_id:u16][label_len:u16][label_bytes...][has_skin:u8][skin...]
After:  [type:u8][id_len:u16][id_bytes...][style:u8][size:u8][icon_id:u16][label_len:u16][label_bytes...][has_skin:u8][skin...]
```

Modal serialization also changed — `modal_id` from fixed `u16` to length-prefixed string:

```
Before: [type:u8][modal_id:u16][is_open:u8]...
After:  [type:u8][id_len:u16][id_bytes...][is_open:u8]...
```

### SDK API change

```
// Before
button!("Click me", style: Primary)
result.clicks[0]  // fragile index
modal(1, is_open, "Title", 600.0, body)
result.clicks.contains_key("__modal_close_1")

// After
button!("my_btn", "Click me", style: Primary)
result.clicks.contains_key("my_btn")  // self-documenting
modal("settings", is_open, "Settings", 600.0, body)
result.clicks.contains_key("settings::close")  // scoped
```

### TreeRenderResult

```rust
// Before
pub struct TreeRenderResult {
    pub clicks: Vec<bool>,
    pub touch: HashMap<String, host::TouchHit>,
    pub drag: HashMap<String, host::TouchHit>,
}

// After
pub struct TreeRenderResult {
    pub clicks: HashMap<String, host::TouchHit>,
    pub drags: HashMap<String, host::TouchHit>,
}
```

## Absolute positioning

`PropsData` gained four `f32` inset fields (`inset_top`, `inset_right`, `inset_bottom`, `inset_left`) that map to
`taffy::Position::Absolute` with corresponding insets. Any node with at least one non-NAN inset becomes absolutely
positioned within its parent. This was implemented in the protocol and host tree builder as a prerequisite for the
calendar FAB.

## Calendar FAB

The calendar theme toggle was broken — it used a negative-margin + height:0 row hack to overlay a button. With absolute
positioning and the unified click system, it became a `touchable()` canvas positioned with
`inset_bottom: 12.0, inset_right: 12.0`:

```rust
fn theme_toggle_fab(theme: &Theme) -> Node {
    let size = 36.0;
    let r = size / 2.0;
    touchable(
        "theme_toggle",
        props!(width: size, height: size, inset_bottom: 12.0, inset_right: 12.0),
        [
            Draw::circle(r, r + 2.0, r + 1.0, 0x00_00_00_30),  // drop shadow
            Draw::circle(r, r + 1.0, r + 0.5, 0x00_00_00_20),
            Draw::circle(r, r, r, GRAY_70),                      // button face
            Draw::icon_builtin(4.0, 4.0, 28.0, 28.0, icon, theme.text_primary),
        ],
    )
}
```

## What was removed

~100 lines of button counting and index machinery on the host side:

- `count_tree_buttons()`, `count_node_buttons()`, `count_modal_body_buttons()`
- `format_btn_key()` — mapped `u32` button index to interaction key string
- `button_id: &mut u32` parameter threaded through `build_taffy_node`
- `result.clicks.push(false)` pre-allocation during layout
- `button_index_start` offset tracking in `ModalInfo`
- SDK-side `count_buttons()`, `host_get_button_count()`, `host_get_click()`

## Files changed

| File                      | Change                                                                                                                                   |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `sdk/src/tree.rs`         | `Node::Button` gets `id: String`, `TreeRenderResult` unified, `collect_interaction_keys` replaces `collect_touch_keys` + `count_buttons` |
| `sdk/src/host.rs`         | Remove `host_get_button_count`, `host_get_click`                                                                                         |
| `sdk/src/lib.rs`          | `button!` macro: required ID as first arg                                                                                                |
| `src/tree.rs`             | `ButtonContext.id: String`, `TreeResult` unified, remove counting/indexing functions                                                     |
| `src/host_api.rs`         | `HostState`: `tree_clicks` + `tree_drags` replace three separate maps                                                                    |
| `src/runtime_wasmi.rs`    | Remove old host exports, update state transfer                                                                                           |
| `protocol/src/text.rs`    | `PropsData` inset fields (done earlier)                                                                                                  |
| `examples/calendar/`      | FAB as touchable canvas with absolute positioning                                                                                        |
| `examples/hello-widget/`  | String IDs on all buttons                                                                                                                |
| `examples/media-control/` | String IDs, `touch`→`clicks`, `drag`→`drags`                                                                                             |
| `examples/stress-test/`   | String IDs                                                                                                                               |
