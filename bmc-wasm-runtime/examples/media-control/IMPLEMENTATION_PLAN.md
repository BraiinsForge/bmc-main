# Plan: Media Control Widget — Kodi Integration

## Context

The media-control widget discovers Cast, UPnP and Kodi devices via mDNS and provides playback control. Kodi communicates
via HTTP JSON-RPC (`_xbmc-jsonrpc-h._tcp`).

## Completed

### json! proc macro

Added a `json!` compile-time macro to `sdk-macros` that parses JSON templates with `#(expr)` / `#s(expr)` interpolations
and emits `fmt!(...)` calls. Migrated all `fmt!("{{...` calls in `kodi.rs` and `cast.rs`.

### Basic Auth (hardcoded)

Kodi's HTTP JSON-RPC requires authentication when a password is set. Currently hardcoded as `kodi:kodi` via HTTP Basic
Auth header built at connect time from `KODI_PASSWORD` constant in `kodi.rs`.

## TODO

### Parameterise Kodi credentials

The hardcoded `kodi:kodi` Basic Auth must be replaced with user-configurable credentials:

- Add username/password fields to widget settings UI
- Pass credentials to `kodi::connect()` (store in `KodiState`)
- Build the `Authorization` header dynamically from the configured values
- Consider: allow empty password (no auth) for Kodi instances with auth disabled
