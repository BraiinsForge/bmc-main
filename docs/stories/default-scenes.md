# Default Scenes

Every device ships with a factory default scene set matched to the product it runs on. The defaults give a freshly
provisioned or factory-reset device meaningful content out of the box, before the user configures anything.

## User stories

### Boot into relevant content out of the box

> As a user, I want a freshly provisioned device to show useful scenes immediately so I don't have to configure it
> before it does anything.

- The factory default scene set is built from the detected product instead of a single shared configuration.
- BMC100 defaults to a digital clock, weather, and a combined scene with an analog clock, block height, and weather.
- BFM100 defaults to the mining-info geek view and the mining clock, rendered round fullscreen.
- BMM100 and BMM101 default to an analog clock, weather, and the four mining-info views (mining, geek, network, info
  overload).

### See the default scenes rotate automatically

> As a user, I want the default scenes to cycle on their own so the device shows all of its content without any
> interaction.

- Automatic scene cycling is enabled by default on all platforms.
- Each scene shows for 30 seconds and changes with a slide transition.

### Get mining data without entering credentials

> As a user with a deck mounted on a miner, I want the default mining widgets to work immediately so I don't have to
> enter the miner address first.

- The default mining-info and mining-clock widgets point at the miner API on localhost with the factory password.

### Keep my own configuration untouched

> As a user, I want factory defaults to apply only when the device has no configuration so my own scenes are never
> overwritten.

- Platform defaults apply when no configuration file exists — on first boot or after a factory reset.
- An existing configuration is loaded as-is; defaults are never merged into it.
- A configuration that exists but cannot be loaded is backed up next to the original and replaced with the platform
  defaults.

## Constraints

- The default scene set is fixed per product at build time.
- Defaults reference only widgets shipped in the factory image; the init tarball carries every shippable widget so the
  default scenes are always satisfiable.
- The default weather location is Prague; the user can change it per widget.
