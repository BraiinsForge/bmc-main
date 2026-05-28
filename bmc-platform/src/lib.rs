// Copyright (C) 2025  Braiins Systems s.r.o.

#[cfg(feature = "backlight")]
pub mod backlight;
#[cfg(feature = "backlight")]
pub mod generic_backlight_driver;
#[cfg(feature = "linux-input")]
pub mod linux_input;

use index_bmc::BmcPlatform as IndexBmcPlatform;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::{fs, io};
use strum::{EnumIter, EnumMessage, EnumString};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LoadError {
    #[error("I/O error loading {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Parse(#[from] strum::ParseError),
}

#[derive(Debug, Clone, Copy, EnumString, Eq, PartialEq, EnumMessage, EnumIter, Hash)]
pub enum BosPlatform {
    #[strum(serialize = "stm32mp157c-ii3-bmc1")]
    Bmc1,
    #[strum(serialize = "stm32mp157c-ii1-am2")]
    Am2,
    #[strum(serialize = "stm32mp157c-ii2-bmm1")]
    Bmm1,
    #[strum(serialize = "stm32mp157c-ii4-bfm1")]
    Bfm1,
}

impl Display for BosPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Write first serialization, if available (should be always true).
        // In an unlikely case of no serialization, report the platform as unknown.
        write!(
            f,
            "{}",
            self.get_serializations().first().unwrap_or(&"unknown")
        )
    }
}

#[derive(Error, Debug)]
#[error("no upgrade asset for platform {0}")]
pub struct NoUpgradeAsset(BosPlatform);

impl TryFrom<BosPlatform> for IndexBmcPlatform {
    type Error = NoUpgradeAsset;

    fn try_from(value: BosPlatform) -> Result<Self, Self::Error> {
        match value {
            BosPlatform::Bmc1 => Ok(IndexBmcPlatform::Stm32mp157cIi3Bmc1),
            BosPlatform::Am2 | BosPlatform::Bmm1 | BosPlatform::Bfm1 => Err(NoUpgradeAsset(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    Bmc100,
    Bmm100,
    Bmm101,
    Bfm100,
}

impl BosPlatform {
    #[must_use]
    pub fn product(self) -> Product {
        match self {
            Self::Bmc1 => Product::Bmc100,
            Self::Am2 => Product::Bmm100,
            Self::Bmm1 => Product::Bmm101,
            Self::Bfm1 => Product::Bfm100,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayShape {
    Rectangular,
    Round,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayTransform {
    Deg0,
    Deg90,
    Deg270,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchTransform {
    Deg0,
    Deg90,
    Deg270,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibleArea {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Panel-only display snapshot delivered to widgets via the Wayland
/// `display_info` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    pub shape: DisplayShape,
    pub dpi: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayProfile {
    pub logical_width: u32,
    pub logical_height: u32,
    pub advertised_width: u32,
    pub advertised_height: u32,
    pub shape: DisplayShape,
    pub dpi: u32,
    pub scanout_transform: DisplayTransform,
    pub touch_transform: TouchTransform,
    pub visible_area: VisibleArea,
    /// Scene-transition overlap compensating for GC400 edge-sampling under rotated scanout.
    pub seam_overlap_px: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotGrid {
    pub columns: usize,
    pub rows: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedStripKind {
    Apa102,
}

#[derive(Debug, Clone)]
pub struct LedStripProfile {
    pub kind: LedStripKind,
    pub device: PathBuf,
    pub led_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardwareCapabilities {
    pub display: DisplayInfo,
    pub slot_grid: Option<SlotGrid>,
}

#[derive(Debug, Clone)]
pub struct PlatformPaths {
    pub backlight: Option<PathBuf>,
    pub scanout_node: PathBuf,
    pub render_node: PathBuf,
}

#[derive(Debug, Clone)]
pub struct HardwareProfile {
    pub product: Product,
    pub display: DisplayProfile,
    pub slot_grid: Option<SlotGrid>,
    pub led_strip: Option<LedStripProfile>,
    pub paths: PlatformPaths,
}

impl HardwareProfile {
    #[must_use]
    #[expect(clippy::too_many_lines)]
    pub fn for_product(product: Product) -> Self {
        let paths = PlatformPaths {
            backlight: Some(PathBuf::from("/sys/class/backlight/display-bl")),
            scanout_node: PathBuf::from("/dev/dri/card1"),
            render_node: PathBuf::from("/dev/dri/renderD128"),
        };
        match product {
            Product::Bmc100 => Self {
                product,
                display: DisplayProfile {
                    logical_width: 1_280,
                    logical_height: 480,
                    advertised_width: 600,
                    advertised_height: 1_280,
                    shape: DisplayShape::Rectangular,
                    dpi: 217,
                    scanout_transform: DisplayTransform::Deg270,
                    touch_transform: TouchTransform::Deg0,
                    visible_area: VisibleArea {
                        x: 0,
                        y: 0,
                        width: 480,
                        height: 1_280,
                    },
                    seam_overlap_px: 4,
                },
                slot_grid: Some(SlotGrid {
                    columns: 4,
                    rows: 2,
                }),
                led_strip: Some(LedStripProfile {
                    kind: LedStripKind::Apa102,
                    device: PathBuf::from("/dev/spidev0.0"),
                    led_count: 10,
                }),
                paths,
            },
            Product::Bmm100 => Self {
                product,
                display: DisplayProfile {
                    logical_width: 320,
                    logical_height: 240,
                    advertised_width: 320,
                    advertised_height: 240,
                    shape: DisplayShape::Rectangular,
                    dpi: 141,
                    scanout_transform: DisplayTransform::Deg0,
                    touch_transform: TouchTransform::Deg0,
                    visible_area: VisibleArea {
                        x: 0,
                        y: 0,
                        width: 320,
                        height: 240,
                    },
                    seam_overlap_px: 0,
                },
                slot_grid: None,
                led_strip: None,
                paths,
            },
            Product::Bmm101 => Self {
                product,
                display: DisplayProfile {
                    logical_width: 320,
                    logical_height: 480,
                    advertised_width: 320,
                    advertised_height: 480,
                    shape: DisplayShape::Rectangular,
                    dpi: 165,
                    scanout_transform: DisplayTransform::Deg0,
                    touch_transform: TouchTransform::Deg0,
                    visible_area: VisibleArea {
                        x: 0,
                        y: 0,
                        width: 320,
                        height: 480,
                    },
                    seam_overlap_px: 0,
                },
                slot_grid: None,
                led_strip: None,
                paths,
            },
            Product::Bfm100 => Self {
                product,
                display: DisplayProfile {
                    logical_width: 480,
                    logical_height: 480,
                    advertised_width: 480,
                    advertised_height: 480,
                    shape: DisplayShape::Round,
                    dpi: 229,
                    scanout_transform: DisplayTransform::Deg90,
                    touch_transform: TouchTransform::Deg90,
                    visible_area: VisibleArea {
                        x: 0,
                        y: 0,
                        width: 480,
                        height: 480,
                    },
                    seam_overlap_px: 0,
                },
                slot_grid: None,
                led_strip: None,
                paths,
            },
        }
    }

    #[must_use]
    pub fn capabilities(&self) -> HardwareCapabilities {
        HardwareCapabilities {
            display: DisplayInfo {
                width: self.display.logical_width,
                height: self.display.logical_height,
                shape: self.display.shape,
                dpi: self.display.dpi,
            },
            slot_grid: self.slot_grid,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BosVersion {
    pub full: String,
    pub major: String,
    pub is_bos_plus: bool,
}

impl BosVersion {
    const BOS_PLUS_SUFFIX: &str = "-plus";

    pub fn new<T: ToString>(full: &T, major: &T) -> Self {
        // TODO: do better parsing of BOS version
        let full = full.to_string();
        let major = major.to_string();
        Self {
            is_bos_plus: full.contains(Self::BOS_PLUS_SUFFIX),
            full,
            major,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BmcInfo {
    pub bmc_platform: BosPlatform,
    pub bos_version: BosVersion,
}

impl BmcInfo {
    const BOS_PLATFORM_PATH: &str = "etc/bos_platform";
    const BOS_VERSION_PATH: &str = "etc/bos_version";
    const BOS_MAJOR_VERSION_PATH: &str = "etc/bos_major";

    /// Standard BosInfo loader that ignores any path prefix
    pub fn load() -> Result<Self, LoadError> {
        Self::load_with_path_prefix::<&Path>(None)
    }

    /// Optional `path_prefix` to be appended to the configuration path of each element
    pub fn load_with_path_prefix<P: AsRef<Path>>(
        path_prefix: Option<P>,
    ) -> Result<Self, LoadError> {
        let path_prefix = path_prefix
            .map_or(PathBuf::from(std::path::MAIN_SEPARATOR.to_string()), |p| {
                p.as_ref().to_owned()
            });
        Ok(Self {
            bmc_platform: BosPlatform::from_str(&Self::read_to_string(
                &path_prefix,
                Self::BOS_PLATFORM_PATH,
            )?)?,
            bos_version: BosVersion::new(
                &Self::read_to_string(&path_prefix, Self::BOS_VERSION_PATH)?,
                &Self::read_to_string(&path_prefix, Self::BOS_MAJOR_VERSION_PATH)
                    .unwrap_or_default(),
            ),
        })
    }

    fn read_to_string(
        path_prefix: impl AsRef<Path>,
        info_path: &'static str,
    ) -> Result<String, LoadError> {
        let final_path = path_prefix.as_ref().join(info_path);
        fs::read_to_string(&final_path)
            .map(|s| s.trim().to_owned())
            .map_err(|e| LoadError::Io {
                source: e,
                path: final_path,
            })
    }

    #[must_use]
    pub fn new(bmc_platform: BosPlatform, bos_version: BosVersion) -> Self {
        Self {
            bmc_platform,
            bos_version,
        }
    }

    #[inline]
    #[must_use]
    pub fn is_bos_plus(&self) -> bool {
        self.bos_version.is_bos_plus
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn platforms_parse_and_map_to_products() {
        let cases = [
            ("stm32mp157c-ii3-bmc1", BosPlatform::Bmc1, Product::Bmc100),
            ("stm32mp157c-ii1-am2", BosPlatform::Am2, Product::Bmm100),
            ("stm32mp157c-ii2-bmm1", BosPlatform::Bmm1, Product::Bmm101),
            ("stm32mp157c-ii4-bfm1", BosPlatform::Bfm1, Product::Bfm100),
        ];
        for (raw, platform, product) in cases {
            assert_eq!(
                BosPlatform::from_str(raw).expect("BUG: parse platform"),
                platform
            );
            assert_eq!(platform.product(), product);
        }
    }

    #[test]
    fn bmc100_profile_has_grid_and_led_others_do_not() {
        let bmc = HardwareProfile::for_product(Product::Bmc100);
        assert_eq!(
            (bmc.display.logical_width, bmc.display.logical_height),
            (1_280, 480)
        );
        assert_eq!(bmc.slot_grid.map(|g| (g.columns, g.rows)), Some((4, 2)));
        assert_eq!(bmc.led_strip.as_ref().map(|l| l.led_count), Some(10));
        assert_eq!(bmc.display.seam_overlap_px, 4);

        for product in [Product::Bmm100, Product::Bmm101, Product::Bfm100] {
            let p = HardwareProfile::for_product(product);
            assert!(p.slot_grid.is_none());
            assert!(p.led_strip.is_none());
            assert_eq!(p.display.seam_overlap_px, 0);
        }
    }

    #[test]
    fn display_shape_matches_per_product() {
        let cases = [
            (Product::Bmc100, DisplayShape::Rectangular),
            (Product::Bmm100, DisplayShape::Rectangular),
            (Product::Bmm101, DisplayShape::Rectangular),
            (Product::Bfm100, DisplayShape::Round),
        ];
        for (product, expected) in cases {
            let profile = HardwareProfile::for_product(product);
            assert_eq!(profile.display.shape, expected, "{product:?}");
        }
    }

    #[test]
    fn capabilities_mirror_the_profile() {
        let caps = HardwareProfile::for_product(Product::Bmc100).capabilities();
        assert_eq!((caps.display.width, caps.display.height), (1_280, 480));
        assert_eq!(caps.display.shape, DisplayShape::Rectangular);
        assert_eq!(caps.slot_grid.map(|g| (g.columns, g.rows)), Some((4, 2)));

        let bfm = HardwareProfile::for_product(Product::Bfm100).capabilities();
        assert_eq!(bfm.display.shape, DisplayShape::Round);
    }

    #[test]
    fn only_the_deck_has_an_upgrade_asset() {
        assert!(IndexBmcPlatform::try_from(BosPlatform::Bmc1).is_ok());
        for platform in [BosPlatform::Am2, BosPlatform::Bmm1, BosPlatform::Bfm1] {
            assert!(IndexBmcPlatform::try_from(platform).is_err());
        }
    }
}
