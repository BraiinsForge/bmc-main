# BOS Miner Model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Also invoke the `wasm-widget-development` skill and read `docs/devel/wasm-widgets/best-practices.md` before changing render/driver code.

**Goal:** Populate and display the per-device BOS miner model in the fleet-management widget, sourced from endpoints the widget already polls.

**Architecture:** A new `ModelAccumulator` (all-`Option` fields) is filled across the `/miner/details` and `/miner/hw/hashboards` responses during each round-robin pass (mirroring how `TelemetryReading` is filled). It converts to a `MinerModel` (whose `id`/`name` are required `String`) only when both the platform slug and product name are known, and the result is stamped onto the device. Device-level absence stays represented by `KnownDevice.model: Option<MinerModel>`. The product model name is shown as an interim device-list column; the platform slug is stored as the future grouping key.

**Tech Stack:** Rust, `bmc-wasm-sdk`, the fleet-management WASM widget (`widgets-wasm/fleet-management`).

**Spec:** `docs/superpowers/specs/2026-06-04-bos-miner-model-design.md`

**Conventions (from best-practices.md):**
- Build strings with the SDK `fmt!` macro, never `std`'s `format!`/`write!` (the `no-fmt-in-wasm` gate rejects them).
- Pure logic (parsing, the model types) lives in host-testable modules; only `render` and the driver are `#[cfg(target_arch = "wasm32")]`.
- Model missing data with `Option`; render `N/A` for absent values.

**Commands (run sandboxed, from the repo root):**
- Host tests: `nix develop -c bash -c 'cd widgets-wasm && cargo test -p fleet-management'`
- WASM lint (compiles the gated render/driver code): `nix develop -c bash -c 'cd widgets-wasm && cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings'`
- Do **not** run `cargo test` and `cargo clippy` concurrently (shared `target/` produces phantom errors).

---

### Task 1: Add `ModelAccumulator` (keep `MinerModel` fields required)

**Files:**
- Modify: `widgets-wasm/fleet-management/src/model.rs`

`MinerModel.id` and `name` stay required `String` — device-level absence is already `KnownDevice.model: Option<MinerModel>`. The accumulator carries `Option` fields during a pass and only builds a model once both `id` and `name` are present.

- [ ] **Step 1: Replace the test module with tests for the new shape**

Replace the existing `#[cfg(test)] mod tests { ... }` block in `model.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulator_missing_id_or_name_yields_no_model() {
        let no_name = ModelAccumulator {
            id: Some("am2-s17".to_owned()),
            chip_count: Some(76),
            ..ModelAccumulator::default()
        };
        assert_eq!(no_name.into_model(), None);

        let no_id = ModelAccumulator {
            name: Some("BMM 101".to_owned()),
            ..ModelAccumulator::default()
        };
        assert_eq!(no_id.into_model(), None);
    }

    #[test]
    fn accumulator_with_id_and_name_builds_a_model_carrying_every_field() {
        let acc = ModelAccumulator {
            id: Some("stm32mp157c-ii2-bmm1".to_owned()),
            name: Some("BMM 101".to_owned()),
            chip_type: Some("BM1370".to_owned()),
            chip_count: Some(152),
        };
        let model = acc.into_model().expect("id and name present builds a model");
        assert_eq!(model.id, "stm32mp157c-ii2-bmm1");
        assert_eq!(model.name, "BMM 101");
        assert_eq!(model.chip_type.as_deref(), Some("BM1370"));
        assert_eq!(model.chip_count, Some(152));
        assert_eq!(model.nominal_hashrate_ths, None);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop -c bash -c 'cd widgets-wasm && cargo test -p fleet-management'`
Expected: FAIL — `ModelAccumulator` not found.

- [ ] **Step 3: Add the accumulator and trim the model's constructor**

Keep the `MinerModel` struct's fields as `id: String` and `name: String`. Remove the existing `impl MinerModel { ... }` block (the `new` constructor — it is no longer used; models are built via `ModelAccumulator::into_model`). The struct keeps its required fields; annotate the ones not yet read on `wasm32`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct MinerModel {
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "grouping key; consumed once model grouping lands")
    )]
    pub id: String,
    pub name: String,
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "surfaced in the future per-model view")
    )]
    pub chip_type: Option<String>,
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "surfaced in the future per-model view")
    )]
    pub chip_count: Option<u32>,
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "sticker hashrate not yet sourced")
    )]
    pub nominal_hashrate_ths: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelAccumulator {
    pub id: Option<String>,
    pub name: Option<String>,
    pub chip_type: Option<String>,
    pub chip_count: Option<u32>,
}

impl ModelAccumulator {
    /// Convert a pass's accumulated fields into a model, only once both the
    /// platform slug and the product name are known. The remaining hardware
    /// fields ride along.
    #[must_use]
    pub fn into_model(self) -> Option<MinerModel> {
        let (Some(id), Some(name)) = (self.id, self.name) else {
            return None;
        };
        Some(MinerModel {
            id,
            name,
            chip_type: self.chip_type,
            chip_count: self.chip_count,
            nominal_hashrate_ths: None,
        })
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop -c bash -c 'cd widgets-wasm && cargo test -p fleet-management'`
Expected: PASS — including the two new `model::tests`.

- [ ] **Step 5: Commit**

```bash
git add widgets-wasm/fleet-management/src/model.rs
git commit -F - <<'EOF'
fleet-management: Add a model accumulator #BDK-506

- add ModelAccumulator to build a model across telemetry endpoints
- build a model only once both platform slug and name are known
- gate not-yet-read model fields behind a wasm dead_code expectation
EOF
```

---

### Task 2: Map the platform integer enum to its slug

**Files:**
- Modify: `widgets-wasm/fleet-management/src/families/bos.rs`

The BOS REST API serializes `platform` as the raw `proto::Platform` integer (verified: `ii-bos-plus-proto` uses `tonic_build` + `serde::Serialize`, no pbjson). Values come from `ii-bos-plus-proto` `proto/bos/v1/miner.proto`.

- [ ] **Step 1: Add the mapping test**

Add inside the existing `#[cfg(test)] mod tests` block in `bos.rs`:

```rust
    #[test]
    fn platform_slug_maps_every_known_platform() {
        assert_eq!(platform_slug(1), Some("am1-s9"));
        assert_eq!(platform_slug(2), Some("am2-s17"));
        assert_eq!(platform_slug(3), Some("am3-bbb"));
        assert_eq!(platform_slug(4), Some("am3-aml"));
        assert_eq!(platform_slug(5), Some("stm32mp157c-ii1-am2"));
        assert_eq!(platform_slug(6), Some("cvitek-bm1-am2"));
        assert_eq!(platform_slug(7), Some("zynq-bm3-am2"));
        assert_eq!(platform_slug(8), Some("stm32mp157c-ii2-bmm1"));
    }

    #[test]
    fn platform_slug_rejects_unspecified_and_unknown() {
        assert_eq!(platform_slug(0), None);
        assert_eq!(platform_slug(99), None);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop -c bash -c 'cd widgets-wasm && cargo test -p fleet-management'`
Expected: FAIL — `platform_slug` not found.

- [ ] **Step 3: Add the mapping function**

Add to `bos.rs`, just below the `BOS_TELEMETRY_ENDPOINTS` constant:

```rust
/// Map the BOS `Platform` enum integer (as serialized over REST) to its
/// stable slug. Mirrors `proto::Platform` in `ii-bos-plus-proto`; an
/// `Unspecified`/`0` or unrecognized value has no slug.
#[must_use]
fn platform_slug(platform: i64) -> Option<&'static str> {
    match platform {
        1 => Some("am1-s9"),
        2 => Some("am2-s17"),
        3 => Some("am3-bbb"),
        4 => Some("am3-aml"),
        5 => Some("stm32mp157c-ii1-am2"),
        6 => Some("cvitek-bm1-am2"),
        7 => Some("zynq-bm3-am2"),
        8 => Some("stm32mp157c-ii2-bmm1"),
        _ => None,
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop -c bash -c 'cd widgets-wasm && cargo test -p fleet-management'`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add widgets-wasm/fleet-management/src/families/bos.rs
git commit -F - <<'EOF'
fleet-management: Map BOS platform integer to a slug #BDK-506

- translate the platform enum value to its stable BosPlatform slug
- treat unspecified and unrecognized platforms as having no slug
EOF
```

---

### Task 3: Parse the model from the BOS responses

**Files:**
- Modify: `widgets-wasm/fleet-management/src/adapter.rs`
- Modify: `widgets-wasm/fleet-management/src/families/bos.rs`

- [ ] **Step 1: Add `parse_model` tests**

Add inside the `#[cfg(test)] mod tests` block in `bos.rs`:

```rust
    #[test]
    fn parses_details_into_id_and_name() {
        let mut j = MapJson::default();
        j.ints.insert("/platform", 8);
        j.strings
            .insert("/miner_identity/miner_model", "BMM 101");
        let mut acc = ModelAccumulator::default();
        BosAdapter.parse_model("/miner/details", &j, &mut acc);
        assert_eq!(acc.id.as_deref(), Some("stm32mp157c-ii2-bmm1"));
        assert_eq!(acc.name.as_deref(), Some("BMM 101"));
    }

    #[test]
    fn details_without_miner_model_leaves_name_none() {
        let mut j = MapJson::default();
        j.ints.insert("/platform", 2);
        let mut acc = ModelAccumulator::default();
        BosAdapter.parse_model("/miner/details", &j, &mut acc);
        assert_eq!(acc.id.as_deref(), Some("am2-s17"));
        assert_eq!(acc.name, None);
    }

    #[test]
    fn parses_hashboards_into_chip_type_and_summed_count() {
        let mut j = MapJson::default();
        j.strings.insert("/hashboards/0/chip_type", "BM1370");
        j.ints.insert("/hashboards/0/chips_count", 76);
        j.strings.insert("/hashboards/1/chip_type", "BM1370");
        j.ints.insert("/hashboards/1/chips_count", 76);
        let mut acc = ModelAccumulator::default();
        BosAdapter.parse_model("/miner/hw/hashboards", &j, &mut acc);
        assert_eq!(acc.chip_type.as_deref(), Some("BM1370"));
        assert_eq!(acc.chip_count, Some(152));
    }
```

- [ ] **Step 2: Add the trait method (default no-op) to `adapter.rs`**

In `adapter.rs`, add the `ModelAccumulator` import and a default-bodied trait method. Update the import line:

```rust
use crate::model::{MinerModel, ModelAccumulator};
```

(If `model` is not yet imported in `adapter.rs`, add the line above; `MinerModel` may be unused there — if clippy flags it, import only `ModelAccumulator`.)

Add this method to the `FamilyAdapter` trait, after `reset_telemetry`:

```rust
    /// Fill model fields from one telemetry response. Default no-op so families
    /// that do not report a model are unaffected.
    fn parse_model(&self, _endpoint: &str, _json: &dyn JsonLookup, _model: &mut ModelAccumulator) {
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `nix develop -c bash -c 'cd widgets-wasm && cargo test -p fleet-management'`
Expected: FAIL — `BosAdapter::parse_model` still uses the no-op default, so the assertions fail (e.g. `acc.id` is `None`).

- [ ] **Step 4: Implement `parse_model` on `BosAdapter`**

In `bos.rs`, ensure the import line brings in `ModelAccumulator`:

```rust
use crate::model::ModelAccumulator;
```

Add this method inside `impl FamilyAdapter for BosAdapter`, after `reset_telemetry`:

```rust
    fn parse_model(&self, endpoint: &str, json: &dyn JsonLookup, model: &mut ModelAccumulator) {
        match endpoint {
            EP_DETAILS => {
                if let Some(slug) = json.i64("/platform").and_then(platform_slug) {
                    model.id = Some(slug.to_owned());
                }
                if let Some(name) = json
                    .str("/miner_identity/miner_model")
                    .filter(|s| !s.is_empty())
                {
                    model.name = Some(name);
                }
            }
            EP_HASHBOARDS => {
                let mut total: Option<u32> = None;
                let mut i = 0usize;
                loop {
                    let type_path = bmc_wasm_sdk::fmt!("/hashboards/{}/chip_type", i);
                    let count_path = bmc_wasm_sdk::fmt!("/hashboards/{}/chips_count", i);
                    let chip_type = json.str(&type_path).filter(|s| !s.is_empty());
                    let chips = json.i64(&count_path).and_then(|v| u32::try_from(v).ok());
                    if chip_type.is_none() && chips.is_none() {
                        break;
                    }
                    if model.chip_type.is_none() {
                        model.chip_type = chip_type;
                    }
                    if let Some(chips) = chips {
                        total = Some(total.unwrap_or(0).saturating_add(chips));
                    }
                    i += 1;
                }
                if total.is_some() {
                    model.chip_count = total;
                }
            }
            _ => {}
        }
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `nix develop -c bash -c 'cd widgets-wasm && cargo test -p fleet-management'`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add widgets-wasm/fleet-management/src/adapter.rs widgets-wasm/fleet-management/src/families/bos.rs
git commit -F - <<'EOF'
fleet-management: Parse BOS miner model #BDK-506

- add a default no-op parse_model hook to the family adapter
- read the platform slug and product model name from details
- read the chip type and summed chip count from hashboards
EOF
```

---

### Task 4: Stamp and clear the model on the device list

**Files:**
- Modify: `widgets-wasm/fleet-management/src/device.rs`

- [ ] **Step 1: Add the tests**

Add inside the `#[cfg(test)] mod tests` block in `device.rs`:

```rust
    fn model(name: &str) -> MinerModel {
        MinerModel {
            id: "stm32mp157c-ii2-bmm1".to_owned(),
            name: name.to_owned(),
            chip_type: None,
            chip_count: None,
            nominal_hashrate_ths: None,
        }
    }

    #[test]
    fn apply_model_stamps_model_onto_device() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.apply_model(&DeviceId::new("a._http._tcp.local."), model("BMM 101"));
        let dev = list.iter().next().expect("device present");
        assert_eq!(
            dev.model.as_ref().map(|m| m.name.as_str()),
            Some("BMM 101")
        );
    }

    #[test]
    fn clear_all_telemetry_also_clears_model() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.apply_model(&DeviceId::new("a._http._tcp.local."), model("BMM 101"));
        list.clear_all_telemetry();
        assert!(list.iter().next().expect("present").model.is_none());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop -c bash -c 'cd widgets-wasm && cargo test -p fleet-management'`
Expected: FAIL — `apply_model` not found, and `clear_all_telemetry` does not clear `model`.

- [ ] **Step 3: Add `apply_model` and clear the model**

In `device.rs`, add this method to `impl DeviceList`, right after `apply_telemetry`:

```rust
    /// Stamp the latest model onto a device. Independent of telemetry: a model
    /// is only applied when one was obtained, so a failed pass keeps the last.
    pub fn apply_model(&mut self, id: &DeviceId, model: MinerModel) {
        if let Some(dev) = self.devices.iter_mut().find(|d| &d.identity.id == id) {
            dev.model = Some(model);
        }
    }
```

In the existing `clear_all_telemetry`, add the `model` reset:

```rust
    pub fn clear_all_telemetry(&mut self) {
        for dev in &mut self.devices {
            dev.telemetry = None;
            dev.model = None;
            dev.reachable = false;
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop -c bash -c 'cd widgets-wasm && cargo test -p fleet-management'`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add widgets-wasm/fleet-management/src/device.rs
git commit -F - <<'EOF'
fleet-management: Stamp and clear the device model #BDK-506

- add apply_model to record the latest model on a device
- clear the model alongside telemetry on a credential change
EOF
```

---

### Task 5: Accumulate and apply the model in the polling driver

**Files:**
- Modify: `widgets-wasm/fleet-management/src/session.rs`

This code is `#[cfg(target_arch = "wasm32")]` and is not host-tested; it is verified by the WASM clippy gate, which compiles it.

- [ ] **Step 1: Import the accumulator**

In the `mod driver` block of `session.rs`, add to the `use crate::...` imports:

```rust
    use crate::model::ModelAccumulator;
```

- [ ] **Step 2: Add the accumulator to the driver state**

In `struct Driver`, add a field after `reading`:

```rust
        model: ModelAccumulator,
```

In `Driver::idle()`, add the field to the constructed value (it is a `const fn`, so use a literal):

```rust
            model: ModelAccumulator {
                id: None,
                name: None,
                chip_type: None,
                chip_count: None,
            },
```

- [ ] **Step 3: Reset the accumulator at the start of each device**

In `begin_device`, in the closure that resets per-device state (where `d.reading = Driver::idle().reading;` is set), add:

```rust
            d.model = ModelAccumulator::default();
```

- [ ] **Step 4: Parse the model alongside telemetry**

In `on_endpoint`, replace the success branch:

```rust
        if response.ok() {
            let doc = response.json();
            DRIVER.with(|d| BosAdapter.parse_telemetry(ep, &doc, &mut d.borrow_mut().reading));
            record_endpoint(true);
        } else {
```

with:

```rust
        if response.ok() {
            let doc = response.json();
            DRIVER.with(|d| {
                let mut d = d.borrow_mut();
                let d = &mut *d;
                BosAdapter.parse_telemetry(ep, &doc, &mut d.reading);
                BosAdapter.parse_model(ep, &doc, &mut d.model);
            });
            record_endpoint(true);
        } else {
```

- [ ] **Step 5: Apply the model when finalizing the device**

In `finalize_device`, replace the body inside `if let Some(id) = id { ... }`:

```rust
        if let Some(id) = id {
            let (reading, reachable, model) = DRIVER.with(|d| {
                let d = d.borrow();
                (d.reading, pass_reachable(&d.endpoint_oks), d.model.clone())
            });
            crate::DEVICES.with(|devs| {
                let mut devs = devs.borrow_mut();
                devs.apply_telemetry(&id, reading, reachable);
                if let Some(model) = model.into_model() {
                    devs.apply_model(&id, model);
                }
            });
            request_frame();
        }
```

- [ ] **Step 6: Verify the WASM build and lints**

Run: `nix develop -c bash -c 'cd widgets-wasm && cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings'`
Expected: PASS, no warnings.

Then re-run host tests to confirm nothing regressed:
Run: `nix develop -c bash -c 'cd widgets-wasm && cargo test -p fleet-management'`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add widgets-wasm/fleet-management/src/session.rs
git commit -F - <<'EOF'
fleet-management: Accumulate and apply the BOS model #BDK-506

- collect model fields across the details and hashboards endpoints
- reset the accumulator per device and apply the model on finalize
EOF
```

---

### Task 6: Show the model column in the device list

**Files:**
- Modify: `widgets-wasm/fleet-management/src/render.rs`

This code is `#[cfg(target_arch = "wasm32")]`; verify via the WASM clippy gate and (where a GPU is available) the visual-regression gate.

- [ ] **Step 1: Import `MinerModel` and add the model cell helper**

In `render.rs`, add to the imports:

```rust
use crate::model::MinerModel;
```

Add this helper alongside the other `*_cell` functions:

```rust
fn model_cell(model: Option<&MinerModel>) -> String {
    match model.map(|m| m.name.clone()) {
        Some(name) => name,
        None => "N/A".to_owned(),
    }
}
```

- [ ] **Step 2: Insert the model column into each device row**

In `view`, inside the `for dev in devices.iter()` loop, change the per-device setup to also grab the model:

```rust
        let reading = dev.telemetry.as_ref().map(|s| &s.reading);
        let model = dev.model.as_ref();
```

Then insert a model `text` cell immediately after the device-name cell in the `row(...)` children (between the name `text(...)` and the `hashrate_cell` `text(...)`):

```rust
                text(
                    model_cell(model),
                    style!(size: 20, color: GRAY_40),
                ),
```

- [ ] **Step 3: Verify the WASM build and lints**

Run: `nix develop -c bash -c 'cd widgets-wasm && cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings'`
Expected: PASS, no warnings.

- [ ] **Step 4: Verify no-fmt gate and (if a GPU is present) the visual regression**

Run: `just validate-wasm-no-fmt`
Expected: PASS.

Run (only meaningful with `/dev/dri`; otherwise it is a CI-only gate): `just wasm::verify fleet-management`
Expected: renders the device list with a model column showing the product name (or `N/A`).

- [ ] **Step 5: Commit**

```bash
git add widgets-wasm/fleet-management/src/render.rs
git commit -F - <<'EOF'
fleet-management: Show the miner model column #BDK-506

- add an interim model column after the device name
- fall back to N/A when the model is not yet known
EOF
```

---

## Self-Review

**Spec coverage:**
- Data sourced (platform→id, miner_model→name, chip_type, summed chips_count, nominal skipped) — Tasks 2, 3.
- Parsing/accumulation via `parse_model` + accumulator, applied at finalize, model re-parsed each pass, cleared on password change — Tasks 1, 3, 4, 5.
- `id`/`name` stay required `String`; the accumulator holds `Option`s and `into_model` requires both — Task 1.
- Display: model column after the name showing the product name, `N/A` fallback; chip_type/chip_count stored not shown — Task 6.
- Tests: platform mapping, parse_model, apply_model/clear, render — Tasks 2, 3, 4, 6.

**Placeholder scan:** None. The one conditional (the `MinerModel` import in `adapter.rs`) gives exact fallback code.

**Type consistency:** `ModelAccumulator { id, name, chip_type, chip_count }` and `MinerModel { id, name, chip_type, chip_count, nominal_hashrate_ths }` are used consistently across Tasks 1, 3, 4, 5, 6; `parse_model(&self, &str, &dyn JsonLookup, &mut ModelAccumulator)` matches between the trait (Task 3 Step 2) and the impl (Task 3 Step 4) and the call site (Task 5 Step 4); `apply_model(&DeviceId, MinerModel)` matches between definition (Task 4) and call (Task 5).

## Notes / risks

- The `platform` wire form (integer) is verified from the proto codegen, not from a live miner. If a live `/miner/details` response shows `platform` as a string slug instead, replace the `json.i64("/platform").and_then(platform_slug)` read in Task 3 with `json.str("/platform")` and drop the mapping table.
- `chip_count` sums `chips_count` across boards; boards with no `hw_details` contribute nothing.
