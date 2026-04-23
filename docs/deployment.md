# Development Deployment

NOTE: This document documents the outdated deployment from pre-Nix era. It is not used for the compositor or standalone
widgets applications, only for the monolithic application.

Depending on what part of the BMC you're modifying, there are different steps to take for deployment to the Deck. The
most common ones are the bmc application itself and frontend.

Then, there might be changes across the system, those ones are usually best handled by building a new firmware and
upgrading to it.

### Prerequisites

- Nix with flakes enabled
- SSH access to the device
- `jq` and `unzip` (for firmware artifact download)

```bash
DEVICE_IP=<device-ip>
```

## BMC Application

Enter the cross-compilation shell, build the release binary, copy it to the device, and on first deploy back up the
original and symlink `/usr/bin/bmc` to `/mnt/data/bmc`. Note that on firmware update, the binary will be replaced again,
so you need to do it again.

```bash
# On host — build
nix develop ".#"
cargo build --release --target armv7-unknown-linux-musleabihf -p bmc-openwrt

# On device
/etc/init.d/bmc stop

# On host - copy
scp target/armv7-unknown-linux-musleabihf/release/bmc-openwrt root@$DEVICE_IP:/mnt/data/bmc

# On device — first time after sysupgrade only
mv /usr/bin/bmc /mnt/data/bmc.orig
ln -sf /mnt/data/bmc /usr/bin/bmc

# On device — restart
/etc/init.d/bmc start
```

Note that you won't be able to copy the file if the application is runinng. In case you wanted to keep the application
running longer, remove the file first, the application will keep running, but you can now copy the file.

## Frontend

Build the frontend via Nix, tar it over SSH to `/mnt/data/www-bmc` on the device, and on first deploy back up `/www/bmc`
and symlink it to a new location.

```bash
# On host — build and copy
nix build ".#frontend"
cd result && tar czf - . | ssh root@$DEVICE_IP 'rm -rf /mnt/data/www-bmc && mkdir -p /mnt/data/www-bmc && tar xzf - -C /mnt/data/www-bmc'

# On device — first time only
mv /www/bmc /mnt/data/www-bmc.orig
ln -sf /mnt/data/www-bmc /www/bmc

# On device — restart
/etc/init.d/bmc restart
```

## Full Firmware

Commit and push your bmc-main changes. In bos-main, point the `bmc-main` flake input to your branch and push. Run the
`firmware-bmc100` job in the bos-main pipeline. Once finished, browse the job artifacts, download the firmware `.tar`
from the `feeds` folder, upload it to the device, and run sysupgrade.

First the bmc-main is pushed, to point to it in bos-main, modify the `flake.nix`'s `bmc-main` input correspondingly on
your own branch:

```bash
bmc-main.url = "git+ssh://git@gitlab.ii.zone/bos/bmc-main?ref=$BRANCH";
```

Then update the input

```
nix flake update bmc-main
```

and push.

You can now start the job from the GitLab UI or via `glab`. There is no need to create an MR.

### Using `glab` CLI

All `glab` commands below must be run from within your local bos-main checkout.

```bash
BRANCH=$(git rev-parse --abbrev-ref HEAD)
```

**Trigger the build:**

```bash
glab ci trigger firmware-bmc100 -b $BRANCH
```

**Check the pipeline / job status:**

```bash
# List recent pipelines for your branch
glab ci list -b $BRANCH

# View jobs in the latest pipeline (shows job IDs and statuses)
glab ci view -b $BRANCH
```

**Get the job URL** (to open in browser):

```bash
glab ci view -b $BRANCH --web
```

**Download the firmware artifact:**

```bash
JOB_ID=$(glab ci get -b $BRANCH -F json -d | jq '.jobs[] | select(.name == "firmware-bmc100") | .id')
tmpdir=$(mktemp -d) && trap 'rm -rf "$tmpdir"' EXIT
glab api "projects/:fullpath/jobs/$JOB_ID/artifacts" > "$tmpdir/artifacts.zip"
unzip -o "$tmpdir/artifacts.zip" -d "$tmpdir/extracted"
cp "$tmpdir"/extracted/*/feeds/*.tar .
```

### Deploy to device

Clean up any previous firmware files first to avoid glob ambiguity:

```bash
# On host
rm -f firmware_*_arm_cortex-a7_neon-vfpv4.tar
```

Then download (see above) and deploy:

```bash
scp firmware_*_arm_cortex-a7_neon-vfpv4.tar root@$DEVICE_IP:/tmp/

# On device — flash (-F forces upgrade, bypassing image checks; dev-only)
sysupgrade -F /tmp/firmware_*_arm_cortex-a7_neon-vfpv4.tar
```
