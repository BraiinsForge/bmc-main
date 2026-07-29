# Credential-type artwork attribution

One SVG per built-in credential type, keyed by the type's id. The bytes are baked into the firmware with `include_str!`
and travel to the frontend inside `CredentialType.icon`, so each file ships in the binary rather than being fetched.

## Third-party

`generic-token.svg` and `generic-userpass.svg` come from the Carbon Design System icon set.

- Source: <https://github.com/carbon-design-system/carbon>, `packages/icons/src/svg/32/`
- Files used: `password.svg` → `generic-token.svg`, `user.svg` → `generic-userpass.svg`
- Version: `@carbon/icons` 11.84.0
- Licence: **Apache-2.0** — Copyright IBM Corp.
- Retrieved: 2026-07-29

They are the same two glyphs the frontend previously rendered from `@carbon/react/icons`, moved here so the backend
declares a type's artwork rather than the frontend inferring it from the type id.

Modified: rescaled from the 32px grid to 24, and placed on an opaque tile with an explicit fill. Both changes exist
because these travel as bytes and render in an `<img>`, where there is no `currentColor` to inherit — the Carbon
originals declare no fill and so came out black on a dark background. The tile is what lets one asset read on either.
The Apache-2.0 licence permits modification; this notice is retained regardless.

## Braiins

`braiins-pool.svg` is the Braiins Pool brand mark, previously bundled with the frontend under
`src/components/images/icons/Accounts/`. Not third-party, recorded here so the directory's provenance is complete.
