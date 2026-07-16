#!/usr/bin/env bash
# Copyright (C) 2025  Braiins Systems s.r.o.
# Copyright (C) 2026  Braiins Forge s.r.o.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#
# Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
# to grant any party a license to this program, or any part thereof,
# under any terms, and such a grant shall be considered distinct from
# the grant above.

# Script: verify_crates.sh
# Purpose: Generic crate verification against upstream repositories

# Colors for output (if terminal supports it)
if [[ -t 1 ]] && command -v tput >/dev/null 2>&1; then
    RED=$(tput setaf 1)
    GREEN=$(tput setaf 2)
    YELLOW=$(tput setaf 3)
    BLUE=$(tput setaf 4)
    BOLD=$(tput bold)
    RESET=$(tput sgr0)
else
    RED=""
    GREEN=""
    YELLOW=""
    BLUE=""
    BOLD=""
    RESET=""
fi

# Default values
CONFIG_FILE="${CRATE_VERIFY_CONFIG:-crate-verification.config.json}"
VERBOSE="${CRATE_VERIFY_VERBOSE:-false}"
SUMMARY="${CRATE_VERIFY_SUMMARY:-false}"
NO_DIFF="${CRATE_VERIFY_NO_DIFF:-false}"

# Cleanup on exit
TMP_ROOT=""
# shellcheck disable=SC2317,SC2329 # cleanup() is invoked indirectly via trap.
cleanup() {
    if [[ -n ${TMP_ROOT:-} ]] && [[ -d ${TMP_ROOT:-} ]]; then
        rm -rf "$TMP_ROOT" || true
    fi
}
trap cleanup EXIT

# Usage function
usage() {
    cat <<EOF
${BOLD}Usage:${RESET} $0 [OPTIONS]

${BOLD}Description:${RESET}
  Verify vendored crates against their upstream repositories using a JSON configuration.
  Supports both explicit crate verification and auto-discovery mode that mimics the
  behavior of verify_crate_hash.sh.
  
${BOLD}Options:${RESET}
  --config FILE      Path to configuration file (default: crate-verification.config.json)
  --verbose          Show detailed output including file diffs
  --summary          Show summary table at the end
  --help             Show this help message

${BOLD}Configuration:${RESET}
  The JSON configuration supports these fields per subtree:
  - repo: Git repository URL
  - commit: Specific commit hash to check against
  - upstream_path: Optional path mapping in upstream repo (for renamed crates)
  - auto_discover: If true, automatically finds all crates in the local path

${BOLD}Auto-Discovery Mode:${RESET}
  When "auto_discover": true is set in config, the script will:
  1. Find all Cargo.toml files under the specified local path
  2. Check each found crate against the upstream repository
  3. Skip crates that don't exist in upstream (same as verify_crate_hash.sh)

${BOLD}Environment Variables:${RESET}
  CRATE_VERIFY_CONFIG   Path to configuration file
  CRATE_VERIFY_VERBOSE  Enable verbose output (true/false)
  CRATE_VERIFY_SUMMARY  Show summary table (true/false)
  CRATE_VERIFY_NO_DIFF  Skip diff output (true/false)

${BOLD}Examples:${RESET}
  $0                                     # Verify all configured subtrees
  $0 --verbose --summary                 # Show detailed output and summary
  $0 --config my-config.json            # Use custom configuration file

${BOLD}Exit Codes:${RESET}
  0 - All verifications passed
  1 - One or more verifications failed
  2 - Configuration or argument error
EOF
    exit 2
}

# Parse arguments
parse_args() {
    # Use getopt for robust parsing
    local OPTS
    if ! OPTS=$(getopt -o "" --long config:,verbose,summary,no-diff,help -n "$0" -- "$@"); then
        usage
    fi

    eval set -- "$OPTS"

    while true; do
        case "$1" in
        --config)
            CONFIG_FILE="$2"
            shift 2
            ;;
        --verbose)
            VERBOSE=true
            shift
            ;;
        --summary)
            SUMMARY=true
            shift
            ;;
        --no-diff)
            NO_DIFF=true
            shift
            ;;
        --help)
            usage
            ;;
        --)
            shift
            break
            ;;
        *)
            echo "${RED}❌ Unknown option: $1${RESET}"
            usage
            ;;
        esac
    done
}

# Load configuration
load_config() {
    if [[ ! -f $CONFIG_FILE ]]; then
        echo "${RED}❌ Configuration file not found: $CONFIG_FILE${RESET}"
        return 1
    fi

    if ! jq empty "$CONFIG_FILE" 2>/dev/null; then
        echo "${RED}❌ Invalid JSON in configuration file: $CONFIG_FILE${RESET}"
        return 1
    fi

    return 0
}

# Get subtrees from config
get_subtrees() {
    jq -r '.vendored_subtrees | keys[]' "$CONFIG_FILE" 2>/dev/null
}

# Get repo and commit for a subtree
get_subtree_info() {
    local subtree="$1"
    local field="$2"

    jq -r ".vendored_subtrees[\"$subtree\"].$field" "$CONFIG_FILE" 2>/dev/null
}

# Debug output function
debug() {
    [[ $VERBOSE == "true" ]] && echo "[DEBUG] $*" >&2
}

# Hash directory function
hash_dir() {
    local dir="$1"
    find "$dir" -type f ! -path "*/target/*" ! -path "*/.git/*" -exec sha256sum {} \; | sed "s|$dir/||" | sort
}

# Global repository checkout cache
declare -A CHECKOUTS
REPO_CHECKOUT_DIR=""

# Clone or reuse repository
get_or_clone_repo() {
    local repo="$1"
    local commit="$2"
    local key="${repo}|${commit}"

    # Check if we already have this repo+commit combination
    if [[ -n ${CHECKOUTS[$key]:-} ]]; then
        echo "${BLUE}🔄 Reusing cached repository: $repo @ ${commit:0:8}${RESET}" >&2
        REPO_CHECKOUT_DIR="${CHECKOUTS[$key]}"
        return 0
    fi

    local checkout_dir
    checkout_dir="$TMP_ROOT/checkout_$(echo "$key" | sha256sum | cut -c1-8)"
    debug "Cloning $repo @ ${commit:0:8} to $checkout_dir"

    echo "${BLUE}📥 Cloning $repo @ ${commit:0:8}...${RESET}" >&2

    # Remove existing directory if it exists
    [[ -d $checkout_dir ]] && rm -rf "$checkout_dir"

    if ! git clone --quiet "$repo" "$checkout_dir" 2>"$TMP_ROOT/git-error.log"; then
        echo "${RED}❌ Failed to clone repository: $repo${RESET}" >&2
        echo "${RED}Error details:${RESET}" >&2
        cat "$TMP_ROOT/git-error.log" >&2
        return 1
    fi

    if ! git -C "$checkout_dir" checkout --quiet "$commit" 2>/dev/null; then
        echo "${RED}❌ Failed to checkout commit: $commit${RESET}" >&2
        return 1
    fi

    # Store the checkout directory for reuse
    CHECKOUTS[$key]="$checkout_dir"
    REPO_CHECKOUT_DIR="$checkout_dir"
    return 0
}

# Find crates in a directory
find_crates() {
    local dir="$1"
    find "$dir" -name Cargo.toml -exec dirname {} \; 2>/dev/null | sort
}

# Get relative path
get_relative_path() {
    local path="$1" base="$2"
    echo "${path#"$base"/}"
}

# Add crate to result list based on verification result
add_crate_result() {
    local result="$1" subtree="$2" crate="$3"
    local display_path
    display_path="$subtree/$(get_relative_path "$crate" "$(pwd)/$subtree")"

    case $result in
    0)
        ((success_crates++))
        success_list+=("$display_path")
        ;;
    2)
        ((skipped_crates++))
        skipped_list+=("$display_path")
        ;;
    *)
        ((failed_crates++))
        failed_list+=("$display_path")
        ;;
    esac
}

# Process subtree crates - find and verify all crates in a subtree
process_subtree_crates() {
    local subtree="$1" upstream_dir="$2" upstream_path="$3" auto_discover="$4"
    local crates_to_verify=()

    # Find crates (same logic for both auto_discover and default)
    mapfile -t crates_to_verify < <(find_crates "$subtree")

    if [[ $auto_discover == "true" ]]; then
        echo "${BLUE}🔍 Auto-discovering crates in $subtree...${RESET}"
        if [[ ${#crates_to_verify[@]} -eq 0 ]]; then
            echo "${YELLOW}⚠️  No crates found in $subtree${RESET}"
        else
            echo "${BLUE}📊 Found ${#crates_to_verify[@]} crate(s) in $subtree${RESET}"
        fi
    fi

    # Verify each crate
    for crate in "${crates_to_verify[@]}"; do
        ((total_crates++))
        debug "Processing crate: $crate"

        verify_crate "$crate" "$subtree" "$upstream_dir" "$upstream_path"
        add_crate_result $? "$subtree" "$crate"
    done
}

# Verify a single crate
verify_crate() {
    local local_crate="$1"
    local subtree="$2"
    local upstream_dir="$3"
    local upstream_path="$4" # Optional upstream path mapping

    # Compute upstream crate path and relative path for display
    local upstream_crate
    local relative_crate

    if [[ -n $upstream_path ]] && [[ $upstream_path != "null" ]]; then
        # Use explicit upstream path mapping
        if [[ $local_crate == "$subtree" ]]; then
            # Single crate case: local crate path equals subtree path
            upstream_crate="$upstream_dir/$upstream_path"
            relative_crate="" # No relative path for single crate case
        else
            # Multi-crate case: compute relative path from subtree and apply to upstream_path
            relative_crate="${local_crate#"${subtree}"/}"
            upstream_crate="$upstream_dir/$upstream_path/$relative_crate"
        fi
    else
        # Default behavior: compute path relative to the subtree root
        # For auto-discover mode, we need to strip the subtree prefix to get the relative path
        relative_crate="${local_crate#"${subtree}"/}"
        upstream_crate="$upstream_dir/$relative_crate"
    fi

    # Display the crate being checked
    local display_path
    if [[ -n $relative_crate ]]; then
        display_path="$subtree/$relative_crate"
    else
        display_path="$subtree"
    fi
    echo "${BLUE}🔄 Checking crate: $display_path${RESET}"

    debug "local_crate: $local_crate"
    debug "relative_crate: $relative_crate"
    debug "upstream_crate: $upstream_crate"

    if [[ ! -d $upstream_crate ]]; then
        echo "${YELLOW}⚠️  Skipping: crate does not exist in upstream${RESET}"
        return 2
    fi

    # Generate hashes
    local local_hashes
    local upstream_hashes
    local_hashes="$TMP_ROOT/local_$(echo "$local_crate" | sha256sum | cut -c1-8).hashes"
    upstream_hashes="$TMP_ROOT/upstream_$(echo "$local_crate" | sha256sum | cut -c1-8).hashes"

    hash_dir "$local_crate" >"$local_hashes"
    hash_dir "$upstream_crate" >"$upstream_hashes"

    # Compare hashes
    if diff -q "$local_hashes" "$upstream_hashes" >/dev/null 2>&1; then
        echo "${GREEN}✅ $display_path matches upstream${RESET}"
        return 0
    else
        echo "${RED}❌ $display_path differs from upstream!${RESET}"

        if [[ $NO_DIFF != "true" ]]; then
            if [[ $VERBOSE == "true" ]]; then
                diff "$upstream_hashes" "$local_hashes" 2>/dev/null || true
            else
                diff "$upstream_hashes" "$local_hashes" 2>/dev/null | head -n 20 || true
            fi
        fi

        return 1
    fi
}

# Main verification logic
main() {
    command -v jq >/dev/null || { echo "jq is missing!" && exit 1; }
    command -v getopt >/dev/null || { echo "getopt is missing!" && exit 1; }

    parse_args "$@"

    # Create temp directory
    TMP_ROOT=$(mktemp -d)

    # Load configuration
    if ! load_config; then
        echo "${RED}❌ Could not load config!"
        exit 2
    fi

    # Statistics
    local total_crates=0
    local success_crates=0
    local failed_crates=0
    local skipped_crates=0
    declare -a failed_list
    declare -a success_list
    declare -a skipped_list

    # Determine which subtrees to process
    local subtrees_to_process=()
    mapfile -t subtrees_to_process < <(get_subtrees)

    # Process each subtree
    for subtree in "${subtrees_to_process[@]}"; do
        subtree=$(echo "$subtree" | xargs) # Trim whitespace

        if [[ ! -d $subtree ]]; then
            echo "${YELLOW}⚠️  Subtree directory not found: $subtree${RESET}"
            continue
        fi

        # Get upstream info
        local repo commit upstream_path auto_discover
        repo=$(get_subtree_info "$subtree" "repo")
        commit=$(get_subtree_info "$subtree" "commit")
        upstream_path=$(get_subtree_info "$subtree" "upstream_path")
        auto_discover=$(get_subtree_info "$subtree" "auto_discover")

        if [[ -z $repo ]] || [[ -z $commit ]]; then
            echo "${YELLOW}⚠️  No configuration found for subtree: $subtree${RESET}"
            continue
        fi

        # Clone or reuse upstream repo
        local upstream_dir
        get_or_clone_repo "$repo" "$commit"
        local clone_result=$?
        if [[ $clone_result -ne 0 ]]; then
            echo "${RED}❌ Failed to prepare upstream for subtree: $subtree${RESET}"

            # Mark all crates in this subtree as failed since we can't get upstream
            local subtree_crates=()
            mapfile -t subtree_crates < <(find_crates "$subtree")

            for failed_crate in "${subtree_crates[@]}"; do
                ((total_crates++))
                ((failed_crates++))
                local crate_display_path
                crate_display_path="$subtree/$(get_relative_path "$failed_crate" "$(pwd)/$subtree")"
                failed_list+=("$crate_display_path (upstream clone failed)")
                echo "${RED}❌ $crate_display_path: upstream repository unavailable${RESET}"
            done

            continue
        fi
        upstream_dir="${REPO_CHECKOUT_DIR}"

        # Process all crates in this subtree
        process_subtree_crates "$subtree" "$upstream_dir" "$upstream_path" "$auto_discover"
    done

    # Show summary if requested or if there are failures
    if [[ $SUMMARY == "true" ]] || [[ $VERBOSE == "true" ]] || [[ $failed_crates -gt 0 ]]; then
        echo ""
        echo "${BOLD}═══════════════════════════════════════════${RESET}"
        echo "${BOLD}                  SUMMARY                   ${RESET}"
        echo "${BOLD}═══════════════════════════════════════════${RESET}"
        echo "${BOLD}Total crates checked:${RESET} $total_crates"
        echo "${GREEN}✅ Passed:${RESET} $success_crates"
        echo "${RED}❌ Failed:${RESET} $failed_crates"
        echo "${YELLOW}⚠️  Skipped:${RESET} $skipped_crates"

        if [[ $failed_crates -gt 0 ]]; then
            echo ""
            echo "${RED}${BOLD}Failed crates:${RESET}"
            for crate in "${failed_list[@]}"; do
                echo "  ${RED}✗${RESET} $crate"
            done
        fi

        if [[ $VERBOSE == "true" ]]; then
            if [[ $success_crates -gt 0 ]]; then
                echo ""
                echo "${GREEN}${BOLD}Successful crates:${RESET}"
                for crate in "${success_list[@]}"; do
                    echo "  ${GREEN}✓${RESET} $crate"
                done
            fi

            if [[ $skipped_crates -gt 0 ]]; then
                echo ""
                echo "${YELLOW}${BOLD}Skipped crates:${RESET}"
                for crate in "${skipped_list[@]}"; do
                    echo "  ${YELLOW}⚠${RESET} $crate"
                done
            fi
        fi

        echo "${BOLD}═══════════════════════════════════════════${RESET}"
    fi

    # Exit with appropriate code
    if [[ $failed_crates -gt 0 ]]; then
        echo ""
        echo "${RED}❌ One or more crates are out of sync with upstream.${RESET}"
        exit 1
    else
        echo ""
        echo "${GREEN}🎉 All vendored crates match their upstream versions!${RESET}"
        exit 0
    fi
}

# Run main function
main "$@"
