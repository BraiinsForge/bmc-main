# bmc-tui

Staged-procedure harness for operating a Braiins Deck from a dev host: initialise its nix store, build and deploy nix
packages, and flash firmware — each as a typed, dry-run-able procedure with progress, confirmation gates, and uniform
error reporting. The package also carries the shared `rich` presentation layer the `bmc-virt` VM harness builds on.

Run it via `nix run .#deck -- <init|deploy|sysupgrade> --help`.

## Architecture

A procedure is a flat sequence of stages; each stage is a typed function guarded by prose verbs that carry the
run/skip/fail verdict.

- **`console`** — `rich` presentation: headers, status lines, key/value, progress bars, panels, the confirm prompt, and
  a terminal-bell / OS-notifier attention API.
- **`stage`** — the engine: `@stage`, the guard verbs `require` / `ensure` / `done_if`, the `Abort` advisory failure
  (rendered as a clean error, never a traceback), the `dry_run` contextvar, and `@entrypoint` (tyro CLI + Abort→exit).
- **`device`** — the ssh transport behind an injected `Exec` seam. `read` always runs, so probes reflect real device
  state even under `--dry-run`; `run` / `push` are mutations that log-and-skip under `--dry-run`.
- **`nix`** — local nix operations behind an injected `Nix` seam: discover / resolve / build / copy.
- **`image`** — a local firmware sysupgrade tarball and the metadata read from it.
- **`catalog`** — the reusable stages the procedures compose.
- **`procedures/`** — the procedures themselves (`init`, `deploy`, `sysupgrade`), each a standalone program; `cli`
  unions them into the single `deck` entry point.

The injected seams (`Exec`, `Nix`) let the stages run under unit tests with no real ssh or nix.

## Key design decisions

- **Stages are typed functions plus guard verbs.** `require(cond, hint)` aborts with a human-actionable remedy;
  `ensure(check, remedy)` auto-remediates then re-checks; `done_if(cond)` makes a stage idempotent. Synchronous and
  sequential — the verbs read as prose and carry the verdict, so a stage body stays a flat list of intentions.
- **Dry-run lives at the seam, not the call site.** A `dry_run` contextvar flips mutating commands to log-and-skip while
  read-only probes still execute, so `--dry-run` reflects real device state. `nix build` is the exception — building the
  closure *is* the verification — so only `copy` and `register` skip.
- **The deploy set is discovered from nix-owned metadata.** The default set is `core` plus every `category == "widget"`
  deck package, read straight from the flake, so new widgets are picked up without touching the harness and all widgets
  ship together.
- **Closures ship via the device's own `nix-store`.** `nix copy` drives the device's `nix-store` remotely over ssh, so
  the device needs only the store, not a full nix; registration is one `bmc-nix-cli add-packages` call.
- **One dispatch app.** `nix run .#deck` unions the procedures over a tyro subcommand, backed by a light bmc-tui-only
  venv (rich + tyro). Each procedure stays independently runnable (`python -m bmc_tui.procedures.deploy`).
- **Confirmation with attention.** Irreversible steps gate behind a confirm prompt (`--yes` to skip) that rings the
  terminal bell and fires a best-effort OS notification — silenced on quick runs and when not a TTY, so a looked-away
  operator notices without being spammed.
- **Non-committal stage status.** A stage's success line states its objective, not a completion claim, so it reads true
  whether the mutation ran or was logged-and-skipped under `--dry-run`.
- **Tooling via the flake.** ruff / ty ship as prebuilt binaries that can't exec under pure-nix CI, so they live in the
  nix dev shell rather than the venv; `.envrc` (`use flake`) auto-enters it so `just python` works without a manual
  `nix develop`.
