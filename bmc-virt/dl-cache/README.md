# OpenWrt Feed Cache

Vendored download cache for the OpenWrt ImageBuilder — `.ipk` packages and feed indexes committed in Git LFS so the VM
image build is fully offline and reproducible.

## Why

The ImageBuilder previously used a Nix fixed-output derivation (FOD) to cache downloads from upstream OpenWrt feeds.
This broke reproducibility because **OpenWrt release feeds are not immutable** — packages get security updates and
rebuilds even within a point release (e.g. openssl 3.0.15 -> 3.0.19, htop 3.4.0 -> 3.4.1 within 24.10.6). Different
machines building at different times would get different feed content, causing hash mismatches.

Inspired by `bos/vendor-packages/openwrt-dl` which vendors source tarballs for full OpenWrt builds, this takes a lighter
approach for the ImageBuilder:

- A **separate flake** (`flake.nix`) runs the ImageBuilder's `make image` to download all required packages (explicit +
  profile defaults + transitive deps). The built image is discarded — only the `dl/` download cache is kept.
- Archives are **plain tar** (no compression — `.ipk` files are already internally compressed, so raw ~= archived).
- A **sha256 fingerprint** of the package list is committed alongside each archive. The main VM flake validates it at
  eval time and fails with a clear message if the cache is stale.
- **`openwrt-config.nix`** is the single source of truth for the package list, OpenWrt version, and the manifest hash
  function — imported by both the cache builder and the VM image builder.

## File layout

```
dl-cache/
  openwrt-config.nix   # shared config (version, packages, hash function)
  flake.nix            # cache builder (standalone tooling)
  flake.lock
  data/
    x86_64.tar         # feed cache for x86_64 guest   (Git LFS)
    x86_64.sha256      # package-list fingerprint
    aarch64.tar         # feed cache for aarch64 guest  (Git LFS)
    aarch64.sha256      # package-list fingerprint
```

## Updating the cache

After changing `packageList` or `openwrtVersion` in `openwrt-config.nix`:

```bash
cd bmc-virt
just update-cache
git add dl-cache/data/
git commit
```

The `update-cache` target builds the dl-cache flake for both architectures (requires `--option sandbox false` for
network access) and copies the results into `data/`.

**Do not edit files in `data/` manually** — they are generated artifacts.
