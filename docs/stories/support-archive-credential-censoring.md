# Support Archive Credential Censoring

The support archive bundles diagnostic files from the device so that a user can
share them with Braiins support.  Some of those files contain credentials that
the user should never have to hand over — a leaked Braiins Pool API key could
even allow withdrawing Bitcoin.

## User stories

### Safe sharing

> As a user, I want to be sure that I can share the support archive with
> Braiins without having to entrust them with my API keys or passwords.

- The archive builder automatically censors known credential fields before
  writing any file into the archive.
- The user does not need to take any manual action; censoring is transparent.

### Minimal redaction

> As a user, I want the archived files to stay as close to the originals as
> possible so that Braiins support can still diagnose issues.

- Only the credential value itself is replaced with `<CENSORED>`.
- Surrounding structure (JSON keys, UCI option names, whitespace, quoting) is
  preserved exactly.

### Reliability over secrecy

> As a support engineer, I want the archive to always be created, even if a
> filter encounters unexpected input.

- If a filter panics or the file is not valid UTF-8, the original content is
  included unchanged and a warning is logged.
- Archive creation is never blocked by a filter failure.

## Censored files

| File                      | Credential            | Format                        |
|---------------------------|-----------------------|-------------------------------|
| `/etc/bmc_config.json`    | `api_key` JSON values | Regex match on `"api_key": "…"` |
| `/etc/config/wireless`    | `option key` value    | OpenWrt UCI line scanner      |

## How it works

1. Every file read by `add_fs_file` is passed through `filters::apply()`.
2. `apply()` looks up the file path in a static `CREDENTIAL_FILTERS` registry.
3. If a matching filter exists, it runs the filter function on the raw bytes.
4. The filter returns censored content (or the original content on failure).
5. The archive writer stores whatever `apply()` returns.

### BMC config filter (`censor_bmc_config`)

Scans for the JSON pattern `"api_key"<colon>"<value>"`, tolerating optional
whitespace around the colon and escaped quotes inside the value.  Replaces
each matched value with `<CENSORED>`.

### UCI wireless filter (`censor_uci_wireless`)

Scans line by line for `option key '<value>'` entries.  Replaces the value
content with `<CENSORED>`, preserving the surrounding single quotes.
