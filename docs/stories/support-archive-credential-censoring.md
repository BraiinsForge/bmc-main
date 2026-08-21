# Support Archive Credential Censoring

The support archive bundles diagnostic files from the device so that a user can share them with Braiins support. Some of
those files contain credentials that the user should never have to hand over — a leaked Braiins Pool API key could even
allow withdrawing Bitcoin.

## User stories

### Safe sharing

> As a user, I want to be sure that I can share the support archive with Braiins without having to entrust them with my
> API keys or passwords.

- The archive builder automatically censors known credential fields before writing any file into the archive.
- The user does not need to take any manual action; censoring is transparent.

### Minimal redaction

> As a user, I want the archived files to stay as close to the originals as possible so that Braiins support can still
> diagnose issues.

- Only the credential value itself is replaced with `<CENSORED>`.
- Surrounding structure (JSON keys, UCI option names, whitespace, quoting) is preserved exactly.

### Reliability over secrecy

> As a support engineer, I want the archive to always be created, even if a filter encounters unexpected input.

- If a filter panics or the file is not valid UTF-8, the original content is included unchanged and a warning is logged.
- Archive creation is never blocked by a filter failure.

## What is protected

Two files are **censored** — the credential value is replaced, the rest is preserved:

| File                                               | Credential            | Format                    |
| -------------------------------------------------- | --------------------- | ------------------------- |
| `/etc/bmc/` config family + `/etc/bmc_config.json` | `api_key` JSON values | Regex on `"api_key": "…"` |
| `/etc/config/wireless`                             | `option key` value    | OpenWrt UCI line scanner  |

One file is **excluded** entirely rather than censored:

| File                    | Reason                                                                  |
| ----------------------- | ----------------------------------------------------------------------- |
| `/etc/bmc/secrets.json` | plaintext account credentials, no diagnostic value — dropped altogether |

## How it works

Every file the archive collects is checked against a set of `SupportFilter`s before it is written. A filter does one of
two things:

- **Exclude** — `excludes(path)` keeps the file out of the archive entirely.
- **Censor** — `matches(path)` marks the file for censoring; it is buffered and every matching filter's `apply()` runs
  in order (each sees the previous one's output) before the result is archived.

A file claimed by more than one censor is filtered by all of them, so adding an overlapping censor can never silently
leak what an earlier one misses. If a filter panics or the file is not valid UTF-8, the original content is archived
unchanged and a warning is logged — archive creation is never blocked by a filter failure.

The `SupportFilter` trait lives in `bmc-support`, which stays platform-agnostic. The concrete filters below live in
`bmc-support-openwrt`, shared by every binary running on the OpenWrt board, and each binary (e.g. `bmc-openwrt`)
registers them in its own `SupportConfig`.

### BMC config censor (`BmcConfigCensor`)

Matches the whole BMC config family — everything under `/etc/bmc/` (the current config and its timestamped backups) plus
the legacy `/etc/bmc_config.json`. Scans for the JSON pattern `"api_key"<colon>"<value>"`, tolerating optional
whitespace around the colon and escaped quotes inside the value, and replaces each matched value with `<CENSORED>`.

### UCI wireless censor (`UciWirelessCensor`)

Matches `/etc/config/wireless` and scans line by line for `option key '<value>'` entries, replacing the value with
`<CENSORED>` while preserving the surrounding single quotes.

### Secrets exclusion (`SecretsExclusion`)

Keeps the plaintext secret store `/etc/bmc/secrets.json` (and its in-progress `secrets.*` temp file) out of the archive
entirely — a censor would have to track every credential field as the catalog grows, and one miss would leak, so the
file is dropped rather than filtered.
