---
name: code-search
description: Search for patterns inside file contents using ripgrep (rg). Use when available.
allowed-tools: Bash, Read, Task
---

# Code Search Skill

Use **ripgrep (`rg`)** for searching patterns inside file contents. For finding files by name, use the `file-nav` skill
instead.

## Availability

Use this skill whenever `rg` is available. If it isn't installed, don't block — fall back to the Grep tool (or `grep`)
for the search. You may optionally hint that installing ripgrep gives faster, gitignore-aware search:

```bash
# Arch Linux
sudo pacman -S ripgrep

# Ubuntu/Debian
sudo apt install ripgrep

# macOS
brew install ripgrep

# Cargo
cargo install ripgrep
```

## Quick Reference

| Task                | Command             |
| ------------------- | ------------------- |
| Search content      | `rg "pattern"`      |
| List matching files | `rg -l "pattern"`   |
| Search with context | `rg -C 3 "pattern"` |

## Core rg Options

### Output Control

```bash
rg "pattern"              # Show matching lines (default)
rg -l "pattern"           # List files only (--files-with-matches)
rg -c "pattern"           # Count matches per file
rg -o "pattern"           # Only show matched text
```

### Context Lines

```bash
rg -A 5 "pattern"         # 5 lines after
rg -B 5 "pattern"         # 5 lines before
rg -C 5 "pattern"         # 5 lines before and after
```

### Filtering by File Type

```bash
rg -t clojure "pattern"   # Only Clojure files
rg -t js "pattern"        # Only JavaScript files
rg -t py "pattern"        # Only Python files
rg -t rust "pattern"      # Only Rust files
rg -T test "pattern"      # Exclude test files

rg -g "*.clj" "pattern"   # Glob filter
rg -g "!*.test.*" "pat"   # Exclude glob pattern
```

### Scope

```bash
rg "pattern" src/         # Search specific directory
rg "pattern" file.clj     # Search specific file
```

### Case Sensitivity

```bash
rg -i "pattern"           # Case insensitive
rg -s "pattern"           # Case sensitive (default)
rg -S "pattern"           # Smart case (insensitive if all lowercase)
```

### Line Numbers

```bash
rg -n "pattern"           # Show line numbers (default)
rg -N "pattern"           # Hide line numbers
```

### Multiline

```bash
rg -U "pattern"           # Multiline mode (pattern can span lines)
rg -U "start.*?end"       # Non-greedy multiline match
```

### Limiting Output

```bash
rg -m 5 "pattern"         # Max 5 matches per file
rg "pattern" | head -20   # First 20 lines of output
```

## Common Search Patterns

### Find Function Definitions

```bash
# Clojure
rg -t clojure "\(defn\s+my-function"
rg -t clojure "\(defn-?\s+my-function"  # defn or defn-

# JavaScript/TypeScript
rg -t js "function\s+myFunction|const\s+myFunction\s*="
rg -t ts "function\s+myFunction"

# Python
rg -t py "def\s+my_function"

# Go
rg -t go "func\s+MyFunction"

# Rust
rg -t rust "fn\s+my_function"
```

### Find Class/Type Definitions

```bash
# Clojure defrecord/deftype
rg -t clojure "\(def(record|type)\s+MyType"

# TypeScript interface/type
rg -t ts "(interface|type)\s+MyType"

# Python class
rg -t py "class\s+MyClass"

# Java/Kotlin class
rg -t java "class\s+MyClass"
```

### Find Usages/References

```bash
# Find all uses of a symbol
rg -C 2 "my-function"

# Word boundary match (avoid partial matches)
rg -w "user"

# Find require/import statements
rg "\[my\.namespace" -t clojure
rg "import.*MyModule" -t js
rg "from.*import.*my_func" -t py
```

### Find TODO/FIXME Comments

```bash
rg -i "(TODO|FIXME|HACK|XXX)"
rg -i "TODO" -C 1
```

### Find Configuration Values

```bash
rg "database|connection" -g "*.{yml,yaml,json,edn}"
rg "password|secret|key" -g "*.env*"
```

## Search Strategies

### 1. Broad to Narrow

Start with file listing, then search with context:

```bash
# Step 1: Find files containing pattern
rg -l "authentication"

# Step 2: Search specific file with context
rg -C 5 "authentication" src/auth.clj
```

### 2. Definition + Usages (run in parallel)

```bash
# Find definition
rg -t clojure "\(defn\s+create-user" -A 10

# Find usages
rg "create-user" -C 2
```

### 3. Exclude Tests/Vendor

```bash
rg "pattern" -g "!*test*" -g "!vendor/*" -g "!node_modules/*"
```

### 4. Multiple File Types

```bash
rg "pattern" -t clojure -t edn
rg "pattern" -g "*.{ts,tsx,js,jsx}"
```

## Regex Tips

- `\s+` for whitespace
- `\(` for literal parentheses
- `.*?` for non-greedy matching
- `[^)]+` for anything except closing paren
- `\{` and `\}` for literal braces
- `(?:...)` for non-capturing groups
- `\b` for word boundaries
- `^` and `$` for line start/end

## Performance Tips

1. **Use `-t type` filters** - Much faster than searching all files
2. **Use `-l` first** - Cheaper than showing content
3. **Specify directory** - `rg "pattern" src/` narrows scope
4. **Use `-m N`** - Stop after N matches per file
5. **Parallel searches** - Run independent rg commands in parallel
6. **Exclude directories** - `-g "!node_modules/*"` etc.

## Advanced Options

```bash
rg --hidden "pattern"     # Include hidden files
rg --no-ignore "pattern"  # Don't respect .gitignore
rg -uuu "pattern"         # Unrestricted (all files)
rg --json "pattern"       # JSON output for parsing
rg -p "pattern" | less -R # Pager with color
```
