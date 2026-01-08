# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

This is the Braiins clock codebase - a Rust-based embedded system for a smart clock device with a web frontend. The
project consists of a modular Rust backend running on OpenWRT (ARMv7), a React/TypeScript frontend, and uses Slint for
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
- **`bmc-display`**: Display management using Slint UI framework (`.slint` files in `bmc-display/ui/`).
- **`bmc-grpc`**: gRPC service definitions (protobuf in `bmc-grpc/proto/web/`) for frontend-backend communication.
- **`bmc-scheduler`**: Alarm scheduling and cron-like functionality.
- **`bmc-audio`**, **`bmc-led`**, **`bmc-button`**, **`bmc-gpio`**, **`bmc-kobject`**: Hardware abstraction layers.
- **`bmc-platform`**: Platform-specific abstractions.
- **`bmc-upgrade`**: Firmware upgrade management.
- **`bmc-shared/`**: Shared libraries (`esp32`, `ii-net`, `ii-net-drv`, `time`, `utils`).

### Frontend Structure (TypeScript/React)

Located in `frontend/`:

- React 19 with React Router for navigation
- gRPC-Web via ConnectRPC for backend communication
- Carbon Design System for UI components
- TypeScript with strict typing
- Yarn 4.x for package management

### Communication Layer

Frontend and backend communicate via gRPC-Web using Protocol Buffers defined in `bmc-grpc/proto/web/`. The backend runs
a tonic gRPC server with tonic-web middleware. Frontend uses ConnectRPC (Connect-Web) for type-safe RPC calls.

## Build System

This project uses **Nix Flakes** as the primary build system.

### Building Components

```bash
# Build frontend (outputs to ./result)
nix build -L .#frontend

# Build mock binary for x86_64 development
nix build .#bmc-mock

# Build OpenWRT binary (ARMv7 release)
nix build .#bmc-openwrt-armv7-release

# Build OpenWRT binary (ARMv7 debug)
nix build .#bmc-openwrt-armv7-debug
```

### Development with Nix

```bash
# Enter default development shell (provides rust toolchain)
nix develop

# Enter ARMv7 release cross-compilation shell
nix develop .#armv7-release

# Enter ARMv7 debug cross-compilation shell
nix develop .#armv7-debug
```

### Running the Mock Server

For local development without hardware:

```bash
# Build frontend first
nix build -L .#frontend

# Run mock server with built frontend assets
cargo run --bin bmc-mock -- --address=0.0.0.0:6070 --www-path=./result
```

### Running on Control Board

To deploy and run on the actual OpenWRT control board:

```bash
cd bmc-openwrt/
nix develop .#armv7-release

export MINER_IP=192.168.1.2
cargo run  # or 'cargo run -- <ARGS>'
# Terminate with Ctrl+C
```

### Cargo Commands

Standard cargo commands work within the nix development shells:

```bash
# Check for compilation errors
cargo check

# Run all tests
cargo test

# Run clippy lints
cargo clippy

# Format code
cargo fmt

# Build (use within nix develop shell for correct toolchain)
cargo build
cargo build --release
```

### Frontend Development

The frontend uses Yarn 4.x and requires Node.js 24.6.0 (managed by Volta):

```bash
cd frontend/

# Install dependencies
yarn install

# Development server (check package.json for available scripts)
yarn dev

# Build for production
yarn build

# Run tests
yarn test

# Lint/format (uses Biome)
yarn lint
yarn format
```

### CI/Nix Checks

The CI system (`.gitlab-ci.yml`) defines several checks that can also be run locally:

```bash
# Run all checks (slow)
nix flake check

# Individual checks
nix build .#checks.x86_64-linux.build      # Build check
nix build .#checks.x86_64-linux.clippy     # Clippy lints
nix build .#checks.x86_64-linux.test       # Rust tests
nix build .#checks.x86_64-linux.nextest    # Nextest runner
nix build .#checks.x86_64-linux.frontend   # Frontend build
nix build .#checks.x86_64-linux.lint       # Frontend lint
```

## Slint UI Development

The display UI uses Slint (version 1.10.0 pinned). By default, the project uses
`EmbedResourcesKind::EmbedForSoftwareRenderer` which generates large Rust files (~50MB) and is slow to compile.

### Speed Up Development

For faster compilation during development, activate the `slint-embed-files` feature:

**RustRover:**

- Open `bmc-display/Cargo.toml`
- Check the `slint-embed-files` feature checkbox
- Apply to custom run configurations if needed

**VSCode:** Create `.vscode/settings.json`:

```json
{
  "rust-analyzer.cargo.features": [
    "bmc-display/slint-embed-files"
  ]
}
```

Then restart rust-analyzer.

**Note:** Remember to also enable this feature in custom run configurations (e.g., manual clippy checks).

## Code Style and Linting

### Rust

Workspace-level lints are defined in `Cargo.toml`:

- Most of `clippy::pedantic` is enabled (with specific exceptions documented in the workspace config)
- Additional useful lints beyond pedantic are enabled
- Uses `rustfmt` for formatting (config in `rustfmt.toml`)
- Rust toolchain version specified in `rust-toolchain.toml`

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
- the previous implementation used incorrect weight values
- this caused rendering artifacts on the display
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

Proto files are in `bmc-grpc/proto/web/`. Changes to `.proto` files require:

1. **Backend**: Rebuild `bmc-grpc` (protobuf code is generated via `build.rs` using `tonic-build`)
2. **Frontend**: Regenerate TypeScript code using `@bufbuild/buf` and `@bufbuild/protoc-gen-es`

The frontend has build tooling configured for protobuf generation (check `frontend/buf.yaml` and frontend build
scripts).

## Cross-Compilation Notes

- The main target platform is ARMv7 (`armv7-unknown-linux-musleabihf`)
- Nix handles cross-compilation setup automatically via the `armv7-release` and `armv7-debug` profiles
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

## Process

### 1. Planning & Staging

Break complex work into 3-5 stages. Document in `IMPLEMENTATION_PLAN.md`:

```markdown
## Stage N: [Name]
**Goal**: [Specific deliverable]
**Success Criteria**: [Testable outcomes]
**Tests**: [Specific test cases]
**Status**: [Not Started|In Progress|Complete]
```

- Update status as you progress
- Remove file when all stages are done

### 2. Implementation Flow

1. **Understand** - Study existing patterns in codebase
2. **Test** - Write test first (red)
3. **Implement** - Minimal code to pass (green)
4. **Refactor** - Clean up with tests passing
5. **Commit** - With clear message linking to plan

### 3. When Stuck (After 3 Attempts)

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

  - Run `nix fmt` (formats Rust, Nix, Python, Shell, Protobuf, TOML, YAML)
  - Run `cargo clippy -- -D warnings` (zero warnings required)
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

## Project Integration

### Learning the Codebase

- Find 3 similar features/components
- Identify common patterns and conventions
- Use same libraries/utilities when possible
- Follow existing test patterns

### Tooling

- Use project's existing build system
- Use project's test framework
- Use project's formatter/linter settings
- Don't introduce new tools without strong justification

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

- Use `--no-verify` to bypass commit hooks
- Disable tests instead of fixing them
- Commit code that doesn't compile
- Make assumptions - verify with existing code

**ALWAYS**:

- Commit working code incrementally
- Update plan documentation as you go
- Learn from existing implementations
- Stop after 3 failed attempts and reassess
- do not use unwrap() but use expect("BUG: $reson") with description why it is bug
