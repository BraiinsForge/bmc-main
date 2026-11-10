# Network Discovery

A Deck announces itself on the local network over mDNS, so its owner reaches the web UI by name instead of hunting for
an IP address. Out of the box the name is `braiins-deck`, which makes the address `http://braiins-deck.local`; the owner
can change the name in the web UI.

Miners that run the BMC application next to Braiins OS (BMM100, BMM101) are out of scope here: Braiins OS announces
those devices and owns their hostname.

## User stories

### Open the web UI by name

> As a Deck owner, I want to open the web UI at a name I can remember, so that I do not have to look up the device's IP
> address.

- The Deck answers at `<hostname>.local` on the network it is connected to. A factory-fresh Deck answers at
  `braiins-deck.local`.
- The settings tray on the display shows the same `.local` name next to the QR code, where the display has the room.
- The Network page of the web UI shows the address under the hostname field.

### Find Decks on the network

> As a Deck owner, I want my Decks to show up when I browse the network, so that I can tell them apart and see what they
> run.

- The Deck is listed as an `_http._tcp` service that points at its web UI. It is also listed under the
  `_braiins-deck._sub._http._tcp` subtype, so a browser can ask for Decks only.
- The listing carries a marker that identifies a Braiins Deck (`braiins_deck=1`), the device model
  (`device=Braiins Deck`), its hostname, the firmware version (`fw_version`) and the version of the BMC application
  packages (`bmc_version`). A version the device cannot read is left out.
- The Deck withdraws the listing when it shuts down or restarts, so browsers do not keep a device that is gone.

### Rename the Deck

> As a Deck owner, I want to give each Deck its own name, so that several of them can share one network.

- The hostname is edited on the Network page, in the Network Settings tab, and saved on its own with **Save hostname**.
- A name the network cannot carry is refused with the reason shown at the field, and the previous name stays.
- After a rename the Deck answers at the new name and stops answering at the old one. The network restarts as part of
  the rename, so the switch takes a few seconds.
- A web UI opened under the old `.local` name moves to the new one after the rename and asks the owner to sign in again;
  for a hostname with dots it moves to the IP address instead. A web UI opened by IP address stays where it is.
- The hostname cannot be changed during the initial setup.

## Constraints

- The name is taken as typed when it is free on the link. When another device already answers to it, such as a second
  Deck still on the factory name, the Deck appends a suffix taken from its MAC address. The web UI and the settings tray
  do not know about the suffix and keep showing the name without it.
- A hostname with dots is valid for the system but not as a single mDNS label; the Deck announces it with each dot
  replaced by a hyphen, and the web UI and the tray again show the name as typed.
- mDNS reaches only the local link. It does not cross routers or a VPN.
