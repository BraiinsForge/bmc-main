# Changes in other repositories

## bos-packages

We can remove the bmc package, it's no longer necessary. We should add
bmc-nix-initializer package that will have its own service. This
service should provide a binary in /usr/bin and an OpenWrt service
that will start it on boot.

## bos-main

We need to point to the new bmc-nix-initializer package in bos-packages.
This package will be exposed by bmc-main flake, similarly to how
bmc-openwrt is now exposed.
The bmc will be completely removed from the target firmware.

## openwrt

We will need a custom COMMAND for the tarball to downlolad and extract
the initial Nix store. Ideally it should be capable of checking the contents.
We should choose a format that supports verifying the hashes of the output files.
This will be inside of the image check.

Something like
```
# Ensure mount
# TODO check /mnt/data mounted. If not, we need to mount it. The partition should already be available

mkdir -p /mnt/data/nix
mkdir /nix
mount -o bind /mnt/data/nix /nix

wget https://cache.braiins.com/initial-nix-tarball.tar.gz -O /tmp/nix-tarball.tar.gz || return 1
tar xzvf /tmp/nix-tarball.tar.gz -C / || return 1
```
In case we use tmp, the tarball has to be smaller than 100 MB. If it wasn't, we can put it to /mnt/data.

But note that this approach means we will have to
