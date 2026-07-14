# Touch discovery: HW vs VM capability survey

Records the empirical capability audit that validated the capability-based discovery predicate in
`bmc-platform::linux_input` (shared by compositor and VM relay) across both appliance targets.

## Question

`bmc_platform::linux_input::discover_touch_node` classifies a `/dev/input/eventN` node as a touchscreen iff it declares
`ABS_X + ABS_Y + BTN_TOUCH`. That predicate was introduced in #BDK-338 precisely to avoid name-matching — the VM and the
HW panel have different `input_dev.name` strings and no shared vendor identifier — but it only works uniformly if every
touchscreen we ship actually advertises those three bits.

Pure multi-touch protocol B devices that set only `ABS_MT_POSITION_X/Y` (no legacy `ABS_X/Y`) would slip through. We
needed to know whether the Goodix panel on the real Deck falls in that camp.

## Evidence

Live query against `192.168.1.183`:

```
$ cat /sys/class/input/event0/device/name
Goodix Capacitive TouchScreen

$ cat /sys/class/input/event0/device/capabilities/abs
2658000 3

$ cat /sys/class/input/event0/device/capabilities/key
400 0 0 0 0 0 0 20000000 1 f8000000 0
```

Only one `eventN` node on HW, which rules out the duplicate-handler dedup path being load-bearing for the Deck (it is
still required for the VM, where the virtio tablet exposes `event1` *and* `event3` on the same `input1`).

## Decode (32-bit words, high-first printing)

**abs bitmap** — two words, reversed to LSB-first:

- word 0 = `0x00000003` → bits 0, 1 set: **`ABS_X` (0x00)** and **`ABS_Y` (0x01)**
- word 1 = `0x02658000` → bits 47, 50, 53, 54, 57 set: `ABS_MT_SLOT`, `ABS_MT_POSITION_X`, `ABS_MT_POSITION_Y`,
  `ABS_MT_TRACKING_ID`, `ABS_MT_PRESSURE`

**key bitmap** — 11 words; only the BTN_TOUCH position matters:

- `BTN_TOUCH = 0x14a = 330`, word 10 offset 10
- word 10 (reversed) = `0x400` → bit 10 set: **`BTN_TOUCH` present**

## Consequence

The Goodix controller publishes **both legacy single-touch axes and the full multi-touch-B axis set**, which is the
conventional "compat" configuration libinput expects. The current predicate matches the Deck panel exactly.

VM side: QEMU's virtio tablet also sets `ABS_X + ABS_Y + BTN_TOUCH` (confirmed earlier during `bmc.log` triage). Both
targets classify correctly under the same rule.

For future-proofing — in case any revision ships a pure-protocol-B controller without legacy `ABS_X/Y` — the predicate
also accepts `(ABS_MT_POSITION_X + ABS_MT_POSITION_Y) + BTN_TOUCH` as an alternative branch. Already implemented in
`bmc_platform::linux_input::is_touchscreen`, so either profile passes.

End-to-end touch validation (Stage 3 of the implementation plan) ran green on both targets with the predicate as
committed.
