# BMC Developer Documentation

Engineering-facing documentation for components that warrant their own write-up beyond a user story — protocol
specifications, internal interfaces, spawn-environment contracts, and architectural rationale. Stories live in
[`docs/stories/`](../stories/README.md); this directory is for the implementation-side details a contributor needs to
work on or change a component.

Components large enough to grow multiple documents get their own subdirectory here.

## Documents

### [Widget Runtime Configuration](widget-runtime-configuration.md)

How a widget process receives its geometry, per-instance params, and current system settings over the `deck_widget_v1`
Wayland protocol. Covers the spawn-environment contract (no BMC-specific env vars), identity resolution via
`SO_PEERCRED` on the Wayland socket, and the configure-batch handshake widgets use to fetch their initial state.
