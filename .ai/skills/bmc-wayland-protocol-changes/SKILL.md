---
name: bmc-wayland-protocol-changes
description: Use when changing one of this repo's Wayland protocols (`deck-widget.xml` and the `deck-*-v1/protocol/*.xml` crates) — adding an event, a request, an argument or an enum entry, bumping an interface version, or adding a protocol. Covers why a new message only ever goes at the end of its interface, how `since` and the interface versions move together (the factory's version included), the version gates on both the compositor and the client side, why a protocol that has not shipped stays at version 1, and the same append-only rule in the widget system snapshot. Triggers on "add an event to the protocol", "new request", "bump the protocol version", "since=", "deck_widget_surface", "create_global", "registry.bind", "SystemFieldKind", or any edit to a protocol XML.
---

# Changing a Wayland protocol

The compositor (`bmc-openwrt`) and its clients (`bmc-widget`, which every widget host and native widget links, and the
overlays through `bmc-system-overlay`) are built from the same XML, but they do not always ship together: a widget
package built against an older protocol keeps running on a newer compositor. Every change has to stay readable by a peer
built from the XML as it was before it.

## Opcodes are positions

Within an interface, requests and events are each numbered by their order in the XML. Nothing in the file names those
numbers, and nothing in the code spells them out (`wayland_scanner` generates both sides), so a reordering compiles and
passes every test built from the same XML — and breaks every peer built from the old one, which decodes each later
message as the one before it.

- A new event or request goes **after the last one of its kind** in its interface, never between existing ones.
- Never move, rename or remove a message or an argument of an interface that has shipped. A change that cannot be an
  append is a new interface with a new name (`_v3`), as `deck_widget_manager_v3` is.
- Enum entries carry their own `value=`, so their order does not matter, but a shipped value is never reused or changed.
  A new entry gets its own `since` and is gated like a new event.

## Versions move together

- The new message gets `since="N"`, and the interface's `version` becomes `N`.
- An object a factory hands out takes the factory's bound version, so the factory interface's `version` goes up with it:
  `deck_widget_surface_v2` objects come from `deck_widget_manager_v3`'s `get_widget_surface`, so a message added to the
  surface raises both interfaces to the same `N`.
- The compositor advertises the new version in its `create_global::<…>(N, ())` under `bmc-openwrt/src/compositor/`.
- A client asks for what it understands with `registry.bind::<…>(name, version.min(N), qh, ())`; raise the cap when the
  client learns the new messages.

## Gate on the bound version

A peer bound at a lower version never receives what it does not know.

- The compositor sends a new event only to resources with `resource.version() >= SINCE`, as the settings and device-info
  broadcasts filter their resources.
- A client sends a new request only when the object it holds is bound at `version() >= SINCE`.
- Name the threshold once, next to the code that gates on it, and say which `since` it mirrors — as `PRODUCT_NAME_SINCE`
  and `DISPLAY_INFO_SINCE` do in `bmc-openwrt/src/compositor/platform.rs`.

## A protocol that has not shipped

A protocol introduced in the MR at hand stays at `version="1"` with no `since` attributes, however many commits reshape
it before it merges — nothing outside the MR has bound an earlier shape, so there is nothing to stay compatible with.
List the destructor first, as most of the others do. Versioning starts with the first change after it ships.

## The same rule in the widget system snapshot

The snapshot the widget host delivers to wasm widgets (`bmc-wasm-runtime/src/system.rs`) is a list of kind-tagged
entries with no length. The SDK's decoder (`find` in `bmc-wasm-runtime/sdk/src/system.rs`) stops at the first
`SystemFieldKind` it does not know and reads every field after it as absent. A new field therefore takes a new
`SystemFieldKind` variant, numbered after every existing one, and is encoded after every existing entry.

## Checklist

1. The new message is the last of its kind in its interface, with `since="N"`; a new enum entry has a fresh `value=`.
2. The interface's `version` is `N`, and so is its factory's.
3. `create_global` advertises `N`; every client's `bind` caps at `N` once it understands the new messages.
4. Both sides gate on a named `*_SINCE` constant.
5. A protocol new in this MR is at version 1 with no `since`.
