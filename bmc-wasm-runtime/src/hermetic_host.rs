// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! A host that runs a widget module away from a device with nothing live in it:
//! no network, no state shared between instances, and a fixed clock.

use std::path::Path;

use anyhow::{Context as _, Result};
use bmc_wasm_thin_protocol::WIDGET_CACHE_BUCKET_MAX_BYTES;
use chrono::{DateTime, FixedOffset, TimeZone as _, Utc};

use crate::platform_catalog::Target;
use crate::prepared_widget::PreparedWidget;
use crate::{DiskCache, EntryPolicy, RuntimeConfig, WasmWidgetModule, WasmWidgetRuntime};

/// A widget module compiled once, entered as often as its caller needs.
#[expect(missing_debug_implementations)]
pub struct HermeticHost {
    module: WasmWidgetModule,
    widget: PreparedWidget,
    cache_root: tempfile::TempDir,
}

impl HermeticHost {
    /// What one call into the module may spend unless its config says otherwise:
    /// the budget a widget frame gets on the device.
    pub const FUEL_PER_TEST: u64 = WasmWidgetRuntime::FUEL_PER_FRAME;

    /// Unpack the widget module at `wasm_path` and compile it.
    pub fn prepare(wasm_path: &Path) -> Result<Self> {
        let widget = PreparedWidget::new(wasm_path, None)?;
        let wasm = std::fs::read(widget.wasm_path())
            .with_context(|| format!("read {}", widget.wasm_path().display()))?;
        let module = WasmWidgetModule::compile(&wasm)
            .with_context(|| format!("compile {}", wasm_path.display()))?;
        let cache_root = tempfile::Builder::new()
            .prefix("bmc-hermetic-cache-")
            .tempdir()
            .context("create the hermetic cache root")?;
        Ok(Self {
            module,
            widget,
            cache_root,
        })
    }

    /// A config for one instance: refused live I/O, a seeded RNG, KV kept in memory,
    /// and an asset cache bucket no other instance sees.
    pub fn config(&self) -> Result<RuntimeConfig> {
        let bucket = tempfile::tempdir_in(self.cache_root.path())
            .context("create an instance cache bucket")?
            .keep();
        Ok(RuntimeConfig {
            hermetic: true,
            rng_seed: Some(1),
            kv_store_path: None,
            asset_cache: Some(DiskCache::new(bucket, WIDGET_CACHE_BUCKET_MAX_BYTES)),
            package_assets: Some(self.widget.asset_store()),
            fuel_per_frame: Self::FUEL_PER_TEST,
            instance_token: Some("test".to_owned()),
            ..RuntimeConfig::default()
        })
    }

    /// Enter the module at `target`'s geometry and [`Self::system_time`].
    pub fn instantiate(
        &self,
        entry: EntryPolicy,
        target: Target,
        config: RuntimeConfig,
    ) -> Result<WasmWidgetRuntime> {
        WasmWidgetRuntime::from_module_with_entry(
            &self.module,
            target.viewport.width,
            target.viewport.height,
            target.viewport.runtime_viewport_shape(),
            target.platform.runtime_display_info(),
            Self::system_time(),
            config,
            entry,
        )
    }

    /// The instant every instance starts at.
    /// Noon UTC keeps the calendar date the same in nearly every zone a test can pick.
    #[must_use]
    pub fn system_time() -> DateTime<FixedOffset> {
        Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0)
            .single()
            .expect("BUG: a fixed UTC instant is unambiguous")
            .fixed_offset()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform_catalog;

    fn host() -> HermeticHost {
        let wasm = wat::parse_str(format!(
            r#"
            (module
              (memory (export "memory") 1)
              (func (export "__bmc_sdk_init") (result i64)
                i64.const {})
              (func (export "passes")))
            "#,
            bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
        ))
        .expect("BUG: the test-module WAT must parse");
        let directory = tempfile::tempdir().expect("BUG: a temporary directory is available");
        let path = directory.path().join("tests.wasm");
        std::fs::write(&path, wasm).expect("BUG: the temporary directory is writable");
        HermeticHost::prepare(&path).expect("BUG: a plain test module prepares")
    }

    fn target() -> Target {
        let platform = platform_catalog::default_platform();
        Target {
            platform,
            viewport: &platform.viewports[0],
        }
    }

    #[test]
    fn a_prepared_module_runs_its_tests() {
        let host = host();
        let config = host.config().expect("BUG: a config builds");
        let mut runtime = host
            .instantiate(EntryPolicy::TestModule, target(), config)
            .expect("BUG: the test module enters");

        let outcome = runtime.call_test_export("passes");

        assert!(outcome.is_ok(), "{outcome:?}");
    }

    #[test]
    fn instances_share_no_cache_entries() {
        let host = host();
        let cache = |host: &HermeticHost| {
            host.config()
                .expect("BUG: a config builds")
                .asset_cache
                .expect("BUG: a hermetic config carries a cache")
        };
        let first = cache(&host);
        let second = cache(&host);
        first
            .put("entry", 1, b"", b"bytes")
            .expect("BUG: an empty bucket takes a small entry");

        assert!(second.get("entry").is_none());
    }
}
