# Repository AI Instructions

This file provides shared guidance to AI coding agents working in this repository. It is the canonical content behind
both `AGENTS.md` and `CLAUDE.md`, and `.ai/` is the canonical repo-owned directory for shared AI instructions and
skills.

New skills go in `.ai/skills/<name>/SKILL.md` - never directly under `.claude/skills/` or `.agents/skills/`. Those
tool-specific directories contain per-skill symlinks back to `.ai/skills/`. See the `ai-shared-layout` skill for the
full layout, the `*.local.*` convention for per-user state, and the symlink-restore procedure.

## Overview

This is the Braiins clock codebase - a Rust-based embedded system for a smart clock device with a web frontend. The
project consists of a modular Rust backend running on OpenWRT (ARMv7), a React/TypeScript frontend, and uses Wayland for
the display UI.

**📖 For detailed architecture information, see [`docs/architecture/overview.md`](docs/architecture/overview.md)** - This
contains comprehensive documentation of the display system, state management, gesture handling, and performance
characteristics.

## Architecture

### Backend Structure (Rust)

The backend is organized as a Cargo workspace with the following main components:

- **`bmc`**: Core BMC library that orchestrates all other components (audio, LED, display, button management, scheduler,
  upgrades). Contains the main business logic, configuration, and system management.
- **`bmc-openwrt`**: Main binary for the OpenWRT control board - integrates all hardware drivers and BMC core
  functionality for the actual device.
- **`bmc-mock`**: Mock binary for development/testing on x86_64 with simulated hardware.
- **`bmc-grpc`**: gRPC service definitions (protobuf in `bmc-grpc/proto/web/`) for frontend-backend communication.
- **`bmc-scheduler`**: Alarm scheduling and cron-like functionality.
- **`bmc-audio`**, **`bmc-led`**, **`bmc-button`**, **`bmc-gpio`**, **`bmc-kobject`**: Hardware abstraction layers.
- **`bmc-platform`**: Platform-specific abstractions.
- **`bmc-upgrade`**: Firmware upgrade management.
- **`bmc-shared/`**: Shared libraries (`stopwatch`, `time`, `utils`).
- **`bmc-net/`**: Networking crates, shared with bos-main:
  - **`bmc-net`**: the `NetworkManager` facade — network config, provisioning state machine, setup AP and captive portal
    — with the `openwrt` (UCI), `buildroot` and `mock` backends.
  - **`bmc-net-types`**: dependency-light value types (`MacAddr`, network protocol config, WiFi status/scan).
  - **`bmc-net-drv`**: interface enumeration plus the `WifiDriver` backends (`nl80211`, `esp32`).
  - **`bmc-net-dns`**: the `IiResolver` DNS/NTP resolver.
  - **`bmc-net-observe`**: synchronous, read-only connectivity probes for OS-driven overlays.
  - **`bmc-net-diag`**: network diagnostics for the support archive (ifconfig, public IP, ping, pcap).

### Frontend Structure (TypeScript/React)

See `frontend/CLAUDE.md`

### Communication Layer

Frontend and backend communicate via gRPC-Web using Protocol Buffers defined in `bmc-grpc/proto/web/`. The backend runs
a tonic gRPC server with tonic-web middleware. Frontend uses ConnectRPC (Connect-Web) for type-safe RPC calls.

## Build System

This project uses **Nix Flakes** as the primary build system.

### Building Components

```bash
# Build frontend (prints output)
nix build -L .#frontend --print-out-paths --no-link

# Build OpenWRT binary (ARMv7 release)
nix build .#bmc-openwrt-armv7-glibc-release

# Build OpenWRT binary (ARMv7 debug)
nix build .#bmc-openwrt-armv7-glibc-debug
```

### Development with Nix

```bash
# Enter default development shell (provides rust toolchain)
nix develop

# Enter ARMv7 release cross-compilation shell
nix develop .#armv7-glibc-release

# Enter ARMv7 debug cross-compilation shell
nix develop .#armv7-glibc-debug
```

### Deploying to Device

Deploying to, inspecting, and iterating on a real Deck — the `nix run .#deck` harness (`init`/`deploy`/`sysupgrade`),
the `nix-cargo-deploy.sh` fast path, and on-device log/cache/config — is documented in
**[`docs/deployment.md`](docs/deployment.md) ("Deck Device Operations")**.

### Cargo Commands

Standard cargo commands work within the nix development shells:

```bash
# Check for compilation errors
cargo check

# Run all tests
cargo test

# Run clippy lints (matches CI: workspace lints from Cargo.toml + tests, all warnings are errors)
cargo clippy --workspace --tests -- -D warnings

# Format code
nix fmt

# Build (use within nix develop shell for correct toolchain)
cargo build
cargo build --release
```

### CI/Nix Checks

We use GitLab, the checks are in `.gitlab-ci.yml`, mostly using flake outputs.

## Code Style and Linting

### Rust

Workspace-level lints are defined in `Cargo.toml`:

- Most of `clippy::pedantic` is enabled (with specific exceptions documented in the workspace config)
- Additional notable lints beyond pedantic: `wildcard_enum_match_arm`, `allow_attributes` (use `#[expect(...)]` instead
  of `#[allow(...)]`), `str_to_string`, `string_add`, `string_slice`, `get_unwrap`
- CI runs clippy with `-D warnings` on the full workspace including tests
- Local equivalent: `cargo clippy --workspace --tests -- -D warnings`
- Uses `rustfmt` for formatting (config in `rustfmt.toml`)
- Rust toolchain version specified in `rust-toolchain.toml`
- Prefer `usize` for counts and indices — avoid `u32`/`u16`/etc. unless required by an external API or wire format
- Never use `#[serde(deny_unknown_fields)]` — it breaks backward compatibility: files written by a newer version (with
  new fields) stop parsing in older code. Deserialized structs must tolerate unknown keys.

**Module System**: This project uses Rust 2018 module style. Instead of `folder/mod.rs`, use a file named after the
folder at the same level:

```
src/
├── compositor.rs      # Module file for compositor/ (NOT compositor/mod.rs)
├── compositor/
│   ├── state.rs
│   ├── render.rs
│   └── protocol.rs
└── main.rs
```

Always run the Nix formatter (treefmt) after changing any file — don't assume a file type is exempt; the one exclusion
is `frontend/`, which formats itself with Biome:

```bash
nix fmt
```

### Frontend

Uses Biome for linting and formatting (config in `frontend/biome.json`). Frontend has its own formatting rules separate
from the Nix formatter.

## Commit Message Format

All commit messages follow strict formatting guidelines:

### Rules

- **Must** follow imperative style (Linus' style) with no exceptions
- **Must** reference the ticket/tickets (e.g., `#BDK-55`, `#BOS-56`)
- **Must** include a blank line between subject and body
- **Must** limit subject and body lines to 72 characters
- **Must** commit contains Author with full name (with diacritics) and company email
- **Must** start subject after topic with uppercase letter
- **Must** write all sentences in the imperative (similar to subject)
- **Must** start each sentence in the body with a lowercase letter
- **Never** add "Generated with Claude Code" or "Co-Authored-By: Claude" to commit messages
- Use "-" for each line in the body (no leading space at the beginning)
- Add ticket reference at the end as an alternative approach, but do not mix styles - be consistent
- For multiple topics, chain them: `topic1: topic2: topic3: Subject description`

### Examples

**Single topic with ticket in subject:**

```
bmc-display: Fix analog clock font rendering issue #BDK-70

- update font weight calculation to match design specifications
- replace incorrect weight values with the expected ones
- prevent rendering artifacts on the display
```

**Multiple topics chained:**

```
frontend: settings: Add dark mode toggle #BOS-123

- implement theme switching functionality across all components
- update CSS variables to support both light and dark themes
- add user preference persistence to local storage
```

**Ticket reference at end (alternative style):**

```
bmc-led: Update LED effect for preview scene

- adjust brightness levels for better visibility
- modify transition timing to feel more responsive
- #BDK-93
```

## Shared Crate Verification

`./scripts/verify_crates.sh` verifies vendored crates against their upstream repositories, driven by
`crate-verification.config.json`. Upstream tracking of `ii-net`/`ii-net-drv` ended when networking was rewritten into
the in-repo `bmc-net` crates (BOS-3938); no vendored subtrees are currently tracked. If a crate is vendored again, add
it to the config and verify with:

```bash
nix-shell -p jq getopt --run "./scripts/verify_crates.sh --summary"
```

## Protocol Buffer Workflow

Proto files are in `bmc-grpc/proto/web/`. Changes to `.proto` files require

## Cross-Compilation Notes

- The main target platform is ARMv7 (`armv7-unknown-linux-gnueabihf`)
- Nix handles cross-compilation setup automatically via the `armv7-glibc-release` and `armv7-glibc-debug` profiles
- Build profiles are defined in `workspace.nix`
- `bmc-openwrt` is always cross-compiled for ARM
- `bmc-mock` is compiled for the native platform (x86_64-linux or aarch64-darwin)

## Testing Strategy

- Rust unit tests are colocated with code
- Integration tests use standard Cargo test structure
- What a widget or overlay renders is reviewed in the gallery — `just gallery::run` for the window, or
  `just gallery::capture` for headless shots at knob values a recipe sets. See
  [`bmc-gallery/README.md`](bmc-gallery/README.md), which also spells out what capture cannot catch.
- Frontend tests run on `@rstest/core` (Vitest-compatible API) with React Testing Library; specs are `*.spec.tsx`, run
  via `just fe::test` — see [`frontend/CLAUDE.md`](frontend/CLAUDE.md) for the setup
- The CI runs both nextest and standard cargo test

# Development Guidelines

## Philosophy

### Core Beliefs

- **Incremental progress over big bangs** - Small changes that compile and pass tests
- **Learning from existing code** - Study and plan before implementing
- **Pragmatic over dogmatic** - Adapt to project reality
- **Clear intent over clever code** - Be boring and obvious

### Simplicity Means

- Single responsibility per function/class
- Avoid premature abstractions
- No clever tricks - choose the boring solution
- If you need to explain it, it's too complex

### When Stuck (After 3 Attempts)

**CRITICAL**: Maximum 3 attempts per issue, then STOP.

1. **Document what failed**:

   - What you tried
   - Specific error messages
   - Why you think it failed

2. **Research alternatives**:

   - Find 2-3 similar implementations
   - Note different approaches used

3. **Question fundamentals**:

   - Is this the right abstraction level?
   - Can this be split into smaller problems?
   - Is there a simpler approach entirely?

4. **Try different angle**:

   - Different library/framework feature?
   - Different architectural pattern?
   - Remove abstraction instead of adding?

## Technical Standards

### Architecture Principles

- **Composition over inheritance** - Use dependency injection
- **Interfaces over singletons** - Enable testing and flexibility
- **Explicit over implicit** - Clear data flow and dependencies
- **Test-driven when possible** - Never disable tests, fix them

### Code Quality

- **Every commit must**:

  - Compile successfully
  - Pass all existing tests
  - Include tests for new functionality
  - Follow project formatting/linting

- **Before committing**:

  - Run `just validate` (formats, runs clippy and tests)
  - Self-review changes
  - Invoke the `comment-discipline` skill and run its comment pass over the diff
  - Ensure commit message explains "why"

### Error Handling

- Fail fast with descriptive messages
- Include context for debugging
- Handle errors at appropriate level
- Never silently swallow exceptions

## Decision Framework

When multiple valid approaches exist, choose based on:

1. **Testability** - Can I easily test this?
2. **Readability** - Will someone understand this in 6 months?
3. **Consistency** - Does this match project patterns?
4. **Simplicity** - Is this the simplest solution that works?
5. **Reversibility** - How hard to change later?

## Quality Gates

### Definition of Done

- [ ] Tests written and passing
- [ ] Code follows project conventions
- [ ] No linter/formatter warnings
- [ ] Commit messages are clear
- [ ] Implementation matches plan
- [ ] No TODOs without issue numbers

### Test Guidelines

- Test behavior, not implementation
- One assertion per test when possible
- Clear test names describing scenario
- Use existing test utilities/helpers
- Tests should be deterministic

## Important Reminders

**NEVER**:

- **NEVER** Use `--no-verify` to bypass commit hooks
- **NEVER** Disable tests instead of fixing them
- **NEVER** Commit code that doesn't compile
- **NEVER** Make assumptions - verify with existing code
- **NEVER** introduce new tools without strong justification
- **NEVER** settle a standardised-format parser on your own — surface the choice and let the developer decide

**ALWAYS**:

- Use same libraries/utilities when possible
- Follow existing test patterns
- write generated and throwaway artifacts to `.tmp/<domain>/`, never beside the source that made them — `.tmp` is
  gitignored whole, so nothing generated reaches the source tree or `git status`
- Commit working code incrementally
- Update plan documentation as you go
- Learn from existing implementations
- Stop after 3 failed attempts and reassess
- do not use unwrap() but use expect("BUG: $reason") with description why it is bug
- prefer self-documenting code that runs over a comment: `assert!(cond, "why")`, `expect("BUG: …")`, and descriptive
  names/consts beat cryptic code plus an explanatory comment — the intent can't drift and shows on failure.
- keep comments minimal: invoke the `comment-discipline` skill any time you finish an implementation, are about to
  commit code, or do a code review. It is the single source for comment hygiene — whether to comment and how — so
  comments never restate the code, state the trivial, sprawl, or carry plan/staging or call-graph ("who calls this")
  notes, and read as a human wrote them.
- treat `bmc-netsim` profiles and the widget's family adapters (`widgets-wasm/fleet-management/src/families/`) as
  deliberate **subsets** of the upstream device APIs (BOS+ boser REST, uBOS, ESP-Miner) — a field missing from them does
  not mean upstream lacks it; verify against the upstream openapi/firmware before concluding a field is unavailable
- ask the developer which parser a standardised format should use — see below

## Standardised formats — ask before parsing one by hand

When you need to parse a format that has an international standard — URLs and IRIs (RFC 3986, WHATWG), IP addresses and
CIDR ranges (RFC 4632/4291), IDNA and punycode (UTS-46), dates and times (RFC 3339, ISO 8601), MIME types, character
encodings, semantic versions — **stop and ask the developer which way to go** rather than deciding on your own. Say
which authoritative parser exists, what pulling it in would cost, and what a hand-rolled version would have to get
right. Then let them choose.

Do not treat "it's only thirty lines" or "it avoids a dependency" as settling it by itself. Weigh it against "never
introduce new tools": for a standardised format, the standard parser may well *be* the strong justification that rule
asks for — but that is the developer's call, not a default to assume in either direction.

What makes it worth asking is not tidiness. These formats are adversarial in ways a hand-rolled splitter never
anticipates — userinfo before an `@`, backslashes, percent-encoded separators, bracketed IPv6 literals, IDN homographs —
and a bespoke parser that disagrees with the one downstream is not merely buggy. Where the disagreement decides a
security question, such as which host an egress pin approves versus which host the HTTP client actually dials, the gap
between the two parses *is* the vulnerability. Flag that consequence when you ask.

Two cases that do not need asking:

- a grammar this repo defines itself (the `{{ credential.… }}` substitution subset) has no authoritative parser by
  definition, and a deliberately restricted subset must not be widened into a general engine just to reuse one;
- a convention with no canonical implementation (matching a host against a `*.example.com` pattern) may be written by
  hand — but it should operate on input an authoritative parser has already normalised.
