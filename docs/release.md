# Firmware releases

This document covers what needs to happen from our side to
successfully publish a new release.

Summary of what we need to do:
- Bump bmc-main in bos-main
- Write whatsnew-bmc.md (to `bos/bos-main`, `master` branch)
- Write description and release date in release template (`bos/releases`, `rls/bmc-YY.MM` branch)

## 1. MR against bos-main master bumping dependencies

The bos-main repository is the one that groups all the dependencies
in its flake and builds the firmware.

So when we change anything, we should bump it in bos-main if we want
to include it in the release. The bump MRs are always targeted to `master`
in this stage.

Create an MR that updates at least the `bmc-main` flake input, by executing
```
nix flake update bmc-main
```
in `bos-main` repository.

These MRs should use the release task `#BDK-XXX`.

## 2. Ask someone from BOS to start the release, creating a release candidate

We cannot make the releases ourselves yet. We need to ask someone to
do it for us. They run GitLab CI scripts to create stable branch in
`bos-main` (`bmc/stable-YY.MM`), in `bos/releases` (`rls/bmc-YY.MM`).
Afterwards the indexes are generated and along with artifacts they are
published to `https://downloads.braiins.com.ii.zone/`.

## 3. bos/bmc-main stable branch

We create a stable branch in `bos/bmc-main` ourselves. We should take
the `flake.lock` commit of `bmc-main` on the stable branch
(`bmc/stable-YY.MM`) in `bos-main` and create `bmc/stable-YY.MM` branch
in `bmc-main` from this commit.

## 4. Release template in bos/releases

As part of step 2, release template is generated, we need to fill it.
It's located in `bos/releases`, branch `rls/bmc-YY.MM` as
`bmc/template/YYYY/YY.MM.<minor>.toml`.

The description supports Markdown. It is visible to the users on the
download page and on the Deck itself.

## 5. whatsnew in bos/bos-main

There is a file for bookkeeping, covering what happened during the
release at `braiins-os-plus/whatsnew-bmc.md`. We should fill its
overview and the tasks finalized. The template might be generated through
`whatsnew-draft` CI job or `nix run ".#whatsnew"` (to be investigated
further the next release).

## 6. RC fixes

In case the RC is lacking and needs to be fixed, we cherry pick the
commits we want to add to the `bos/bmc-main` stable branch.

Then we make an MR against the stable branch of `bos/bos-main`, changing
the flake input to point to the stable branch in case it doesn't yet.
And then `nix flake update bmc-main`.

```
    bmc-main.url = "git+ssh://git@gitlab.ii.zone/bos/bmc-main?ref=bmc/stable-YY.MM";
```

## 7. Before release itself

Do not forget to change the release date in the `bos/releases` template to the final date!
