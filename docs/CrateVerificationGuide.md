# Crate Verification Guide

## Overview

The crate verification system ensures that vendored Rust crates in your repository match their upstream sources. The
system supports both explicit crate configuration and auto-discovery mode that mimics the behavior of the legacy
`verify_crate_hash.sh` script.

## Quick Start

### Verify all configured subtrees

```bash
./scripts/verify_crates.sh
```

### Check with verbose output and summary

```bash
./scripts/verify_crates.sh --verbose --summary
```

## Configuration

### Configuration File

The system uses a JSON configuration file (`crate-verification.config.json`) in the repository root:

```json
{
  "vendored_subtrees": {
    "tooling": {
      "repo": "ssh://git@gitlab.ii.zone/tooling/tooling.git",
      "commit": "0bf48952586c5e475368fad41d05b4ddb2b6a079",
      "auto_discover": true
    },
    "vendor/example-crate": {
      "repo": "ssh://git@gitlab.ii.zone/org/example-crate.git",
      "commit": "b64216cc00d2ae4a733d4555337bab06839464fb",
      "upstream_path": "upstream/path/to/example-crate"
    }
  }
}
```

#### Configuration Fields

| Field                                    | Description                                                 | Required |
| ---------------------------------------- | ----------------------------------------------------------- | -------- |
| `vendored_subtrees`                      | Map of local paths to upstream repository info              | Yes      |
| `vendored_subtrees.<path>.repo`          | Upstream repository URL (SSH or HTTPS)                      | Yes      |
| `vendored_subtrees.<path>.commit`        | Specific commit hash to verify against                      | Yes      |
| `vendored_subtrees.<path>.upstream_path` | Optional path mapping in upstream repo (for renamed crates) | No       |
| `vendored_subtrees.<path>.auto_discover` | If true, automatically finds all crates in the local path   | No       |

### Environment Variables

| Variable               | Description                    | Default                          |
| ---------------------- | ------------------------------ | -------------------------------- |
| `CRATE_VERIFY_CONFIG`  | Path to configuration file     | `crate-verification.config.json` |
| `CRATE_VERIFY_VERBOSE` | Enable verbose output          | `false`                          |
| `CRATE_VERIFY_SUMMARY` | Show summary table             | `false`                          |
| `CRATE_VERIFY_NO_DIFF` | Skip diff output on mismatches | `false`                          |

## Command Line Interface

### Synopsis

```bash
./scripts/verify_crates.sh [OPTIONS]
```

### Options

| Option          | Description                                                     |
| --------------- | --------------------------------------------------------------- |
| `--config FILE` | Path to configuration file                                      |
| `--verbose`     | Show detailed output including file diffs and debug information |
| `--summary`     | Show summary table at the end                                   |
| `--no-diff`     | Skip showing diff output on mismatches                          |
| `--help`        | Show help message                                               |

## Usage Examples

### Basic Verification

```bash
# Verify all configured subtrees
./scripts/verify_crates.sh

# Verify with detailed debug output
./scripts/verify_crates.sh --verbose

# Verify with summary table
./scripts/verify_crates.sh --summary

# Verify with summary but no diff output on failures
./scripts/verify_crates.sh --summary --no-diff

# Verify with full verbose output and summary
./scripts/verify_crates.sh --verbose --summary
```

### CI Integration

```bash
# In GitLab CI with nix-shell
nix-shell -p jq getopt --run "./scripts/verify_crates.sh"

# With custom config file
CRATE_VERIFY_CONFIG=ci-crates.json ./scripts/verify_crates.sh

# Alternative custom config
./scripts/verify_crates.sh --config path/to/config.json

# CI with summary output (no diff noise)
./scripts/verify_crates.sh --summary --no-diff
```

## Exit Codes

| Code | Description                              |
| ---- | ---------------------------------------- |
| 0    | All verifications passed                 |
| 1    | One or more crates differ from upstream  |
| 2    | Configuration error or invalid arguments |

## Adding New Vendored Crates

### Step 1: Vendor the Crate

```bash
# Clone and checkout specific commit
git clone https://github.com/org/upstream-crate.git /tmp/crate
cd /tmp/crate
git checkout abc123def456

# Copy to your repository
cp -r /tmp/crate/src new-vendor/upstream-crate
```

### Step 2: Update Configuration

Add the new subtree to `crate-verification.config.json`:

```json
{
  "vendored_subtrees": {
    "existing/path": {
      ...
    },
    "new-vendor/upstream-crate": {
      "repo": "ssh://git@gitlab.ii.zone/org/upstream-crate.git",
      "commit": "abc123def456"
    }
  }
}
```

### Step 3: Verify

```bash
./scripts/verify_crates.sh
```

## Advanced Usage

### Auto-Discovery Mode

Use auto-discovery to automatically find and verify all crates in a subtree (similar to `verify_crate_hash.sh`):

```json
{
  "vendored_subtrees": {
    "tooling": {
      "repo": "ssh://git@gitlab.ii.zone/tooling/tooling.git",
      "commit": "0bf48952586c5e475368fad41d05b4ddb2b6a079",
      "auto_discover": true
    }
  }
}
```

When `auto_discover` is enabled, the script will:

1. Find all `Cargo.toml` files under the specified local path
2. Check each found crate against the upstream repository
3. Skip crates that don't exist in upstream (without failing)

### Path Mapping for Renamed Crates

Use `upstream_path` when the local crate path differs from the upstream path:

```json
{
  "vendored_subtrees": {
    "vendor/example-crate": {
      "repo": "ssh://git@gitlab.ii.zone/org/example-crate.git",
      "commit": "b64216cc00d2ae4a733d4555337bab06839464fb",
      "upstream_path": "upstream/path/to/example-crate"
    }
  }
}
```

### Multiple Repository Verification

Configure multiple vendored subtrees from different repositories:

```json
{
  "vendored_subtrees": {
    "tooling": {
      "repo": "ssh://git@gitlab.ii.zone/tooling/tooling.git",
      "commit": "0bf48952586c5e475368fad41d05b4ddb2b6a079",
      "auto_discover": true
    },
    "vendor/lib-a": {
      "repo": "ssh://git@gitlab.ii.zone/org/lib-a.git",
      "commit": "abc123"
    },
    "vendor/lib-b": {
      "repo": "ssh://git@gitlab.ii.zone/team/lib-b.git",
      "commit": "def456",
      "upstream_path": "different/path/in/upstream"
    }
  }
}
```

### Repository Caching

The script automatically caches cloned repositories to avoid duplicate cloning when multiple crates reference the same
repository and commit. This significantly improves performance when verifying many crates from the same upstream source.

### Debug Output

Use `--verbose` to see detailed debug information:

- Repository caching behavior
- Path calculations for each crate
- Detailed verification steps
- Full diff output on mismatches

```bash
./scripts/verify_crates.sh --verbose
```
