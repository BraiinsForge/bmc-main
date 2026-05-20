# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

This is the Braiins clock codebase - a Rust-based embedded system for a smart clock device with a web frontend. The
project consists of a modular Rust backend running on OpenWRT (ARMv7), a React/TypeScript frontend, and uses Wayland for
the display UI.

**📖 For detailed architecture information, see [`docs/architecture.md`](docs/architecture.md)** - This contains
comprehensive documentation of the display system, state management, gesture handling, and performance characteristics.

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
- **`bmc-shared/`**: Shared libraries (`esp32`, `ii-net`, `ii-net-drv`, `time`, `utils`).

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

For deploying and running on the Braiins Deck, see [`docs/nix-device-scripts.md`](docs/nix-device-scripts.md). Key
scripts:

- `scripts/nix-init.sh` — one-time Nix store setup
- `scripts/nix-deploy.sh` — deploy Nix packages
- `scripts/nix-cargo-deploy.sh` — fast iterative deploy of cargo-built native binaries (compositor and native widgets
  only; wasm widgets are never deployed through this script)

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

Format code using Nix formatter (formats Rust, Nix, Python, Shell, Protobuf, TOML, YAML):

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

Some shared crates (`bmc-shared/ii-net` and `bmc-shared/ii-net-drv`) are vendored/forked. Verify they match upstream:

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
- Frontend tests use React Testing Library and Vitest
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

- Do not reinvent the wheel, use libraries when they exist for the task at hand
- More code needs good justification, prefer well-tested well-known third-party libraries to ad-hoc implementations
- Unsafe code always needs good justification

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

**ALWAYS**:

- Use same libraries/utilities when possible
- Follow existing test patterns
- Commit working code incrementally
- Update plan documentation as you go
- Learn from existing implementations
- Stop after 3 failed attempts and reassess
- do not use unwrap() but use expect("BUG: $reason") with description why it is bug
