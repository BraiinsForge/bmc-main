---
name: file-nav
description: Navigate and find files using fd. Use when available.
allowed-tools: Bash, Read, Task
---

# File Navigation Skill

Use **fd** for finding files and navigating directory structures. fd is a fast, user-friendly alternative to `find`.

## Availability

Use this skill whenever `fd` is available. If it isn't installed, don't block — fall back to the Glob tool (or `find`)
for file navigation. You may optionally hint that installing fd gives faster, friendlier file search:

```bash
# Arch Linux
sudo pacman -S fd

# Ubuntu/Debian (note: binary is 'fdfind', alias to 'fd')
sudo apt install fd-find
alias fd=fdfind

# macOS
brew install fd

# Cargo
cargo install fd-find
```

## Quick Reference

| Task                    | Command                         |
| ----------------------- | ------------------------------- |
| Find files by name      | `fd "pattern"`                  |
| List all files          | `fd`                            |
| List directory contents | `fd . directory/ --max-depth 1` |
| Find by extension       | `fd -e clj`                     |

## Core fd Options

### Basic Search

```bash
fd "pattern"              # Find files/dirs matching pattern
fd                        # List all files (like recursive ls)
fd . path/                # List all files under path/
fd "pattern" path/        # Search within specific directory
```

### Filtering by Type

```bash
fd -t f "pattern"         # Files only
fd -t d "pattern"         # Directories only
fd -t l "pattern"         # Symlinks only
fd -t x "pattern"         # Executables only
```

### Filtering by Extension

```bash
fd -e clj                 # All .clj files
fd -e ts -e tsx           # All .ts and .tsx files
fd -e md "readme"         # Markdown files matching "readme"
```

### Depth Control

```bash
fd --max-depth 1          # Current directory only (like ls)
fd --max-depth 2          # Current + one level down
fd --min-depth 2          # Skip current directory level
```

### Case Sensitivity

```bash
fd "pattern"              # Smart case (default)
fd -s "Pattern"           # Case sensitive
fd -i "pattern"           # Case insensitive
```

### Including Hidden/Ignored Files

```bash
fd -H "pattern"           # Include hidden files
fd -I "pattern"           # Include gitignored files
fd -u "pattern"           # Unrestricted (hidden + ignored)
```

### Exclusion

```bash
fd -E node_modules        # Exclude directory
fd -E "*.test.*"          # Exclude pattern
fd -E vendor -E dist      # Multiple exclusions
```

## Common Tasks

### List Directory Contents (like ls)

```bash
# Current directory only
fd . --max-depth 1

# With file types visible
fd . --max-depth 1 -t f   # Files only
fd . --max-depth 1 -t d   # Directories only

# Specific directory
fd . src/ --max-depth 1
```

### Explore Project Structure

```bash
# Show top-level structure
fd . --max-depth 1

# Show two levels deep
fd . --max-depth 2

# Show only directories (project skeleton)
fd -t d --max-depth 3
```

### Find Files by Name

```bash
# Find config files
fd config
fd "config\.(json|yaml|edn)"

# Find test files
fd "_test\."
fd "test" -t d            # Find test directories

# Find specific file anywhere
fd "^package\.json$"
fd -g "package.json"      # Glob mode (exact match)
```

### Find Files by Extension

```bash
# Single extension
fd -e clj

# Multiple extensions
fd -e ts -e tsx -e js -e jsx

# Extension in specific directory
fd -e sql db/
```

### Find Recently Modified

```bash
fd --changed-within 1d    # Modified in last day
fd --changed-within 1h    # Modified in last hour
fd --changed-before 1w    # Modified more than a week ago
```

### Find by Size

```bash
fd --size +1m             # Files larger than 1MB
fd --size -10k            # Files smaller than 10KB
```

## Output Formats

```bash
fd "pattern"              # One path per line (default)
fd -0 "pattern"           # Null-separated (for xargs -0)
fd -l "pattern"           # Long format (like ls -l)
fd --color always         # Force color output
```

## Executing Commands on Results

```bash
fd -e log -x rm           # Delete all .log files
fd -e clj -x wc -l        # Count lines in each Clojure file
fd -t f -x chmod 644      # Set permissions on all files
```

## Combining with Other Tools

```bash
# Find and grep
fd -e clj -x rg "defn"

# Find and read (use Read tool after fd)
fd "interface.clj"        # Find the file
# Then use Read tool on the result

# Count files by extension
fd -e clj | wc -l
```

## Search Patterns

fd uses regex by default:

```bash
fd "^test"                # Starts with "test"
fd "test$"                # Ends with "test"
fd "test.*spec"           # "test" followed by "spec"
fd "\d{4}"                # Contains 4 digits
```

For glob patterns:

```bash
fd -g "*.config.js"       # Glob mode
fd -g "src/**/*.ts"       # Glob with directory
```

## When to Use fd vs rg

| Task                      | Tool                        |
| ------------------------- | --------------------------- |
| Find files by **name**    | `fd`                        |
| Find files by **content** | `rg`                        |
| List directory structure  | `fd`                        |
| Search inside files       | `rg`                        |
| Find + search content     | `fd -e ext -x rg "pattern"` |

## Performance Tips

1. **Specify directory** - `fd pattern path/` is faster than searching everywhere
2. **Use extension filter** - `fd -e clj` skips irrelevant files
3. **Limit depth** - `--max-depth N` for large trees
4. **Exclude heavy dirs** - `-E node_modules -E .git`

## Examples

### Find all interface files in a project

```bash
fd "interface" -e clj
```

### Show source directory structure

```bash
fd -t d src/ --max-depth 2
```

### Find configuration files

```bash
fd -g "*.{json,yaml,yml,edn,toml}" --max-depth 2
```

### Find test files excluding vendor

```bash
fd "_test\." -E vendor -E node_modules
```

### Find large files

```bash
fd -t f --size +1m -l
```
