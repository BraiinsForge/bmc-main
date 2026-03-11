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

## uboot-stm32

Since we are going to use U-Boot env to detect the factory reset and
store that Nix has been initialized in a uboot variable, we need to
make sure this variable is not cleared on every firmware update. We do
this by adding the variable to the clearenv list of variables that it
should not clear.

## openwrt

We will need a custom COMMAND for the tarball to downlolad and extract
the initial Nix store. Ideally it should be capable of checking the
contents. We should choose a format that supports verifying the hashes
of the output files. This will be inside of the image check. But for
the time being we're constrained by what is in the last major firmware
version on the Deck. It supports only gz. So we use that for now. We
should add a custom hash checker to check the contents of the tarball.

Something like
```
# Ensure mount
# TODO check /mnt/data mounted. If not, we need to mount it. The partition should already be available

# TODO what if it already exists
mkdir -p /mnt/data/nix
mkdir /nix
mount -o bind /mnt/data/nix /nix

wget https://cache.braiins.com/initial-nix-tarball.tar.gz -O /mnt/data/nix-tarball.tar.gz || return 1
tar xzvf /tmp/nix-tarball.tar.gz -C / || return 1

# Do not forget to set the U-Boot environment so that factory reset doesn't happen.
fw_setenv nix_init 1

```

We cannot activate the profile on the current firmware, we always have
to wait for the reboot. Otherwise this could lead to issues, having
the old firmware with monolithic application from BOS and trying to
activate the application from Nix. We also generally cannot provide
tarball with the profile activated as sometimes we might need to
append to some files or do something that's not captured on the file
system level, such as starting a service.

After reboot, `nix-initializer` will activate the first profile
generation when there is no current profile symlink. This is the sole
reason it can activate the generation. Because we cannot do the activation
on previous boot.
