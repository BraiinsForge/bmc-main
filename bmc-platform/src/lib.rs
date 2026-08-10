// Copyright (C) 2025  Braiins Systems s.r.o.
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

#[cfg(feature = "backlight")]
pub mod backlight;
#[cfg(feature = "backlight")]
pub mod generic_backlight_driver;
#[cfg(feature = "linux-input")]
pub mod linux_input;
pub mod serial_number;

use index_bmc::BmcPlatform as IndexBmcPlatform;
use serial_number::{BoardSerial, PcbVersion};
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

impl Product {
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Bmc100 => "Braiins Deck",
            Self::Bmm100 | Self::Bmm101 => "Mini Miner",
            Self::Bfm100 => "Femto Miner",
        }
    }

    #[must_use]
    pub fn default_http_port(self) -> u16 {
        match self {
            Self::Bmc100 => 80,
            Self::Bmm100 | Self::Bmm101 | Self::Bfm100 => 81,
        }
    }
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

/// A parsed `--hardware-profile` value: either a specific product code or
/// `auto`, which defers to the platform detected at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareProfileSelection {
    Auto,
    Platform(BosPlatform),
}

impl FromStr for HardwareProfileSelection {
    type Err = UnknownProfile;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_uppercase().as_str() {
            "AUTO" => Ok(Self::Auto),
            "BMC100" => Ok(Self::Platform(BosPlatform::Bmc1)),
            "BMM100" => Ok(Self::Platform(BosPlatform::Am2)),
            "BMM101" => Ok(Self::Platform(BosPlatform::Bmm1)),
            "BFM100" => Ok(Self::Platform(BosPlatform::Bfm1)),
            _ => Err(UnknownProfile(value.to_owned())),
        }
    }
}

impl From<HardwareProfileSelection> for Option<BosPlatform> {
    fn from(selection: HardwareProfileSelection) -> Self {
        match selection {
            HardwareProfileSelection::Auto => None,
            HardwareProfileSelection::Platform(platform) => Some(platform),
        }
    }
}

#[derive(Error, Debug)]
#[error("unknown hardware profile: {0}")]
pub struct UnknownProfile(String);

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

/// Byte order the compositor must write to the scanout buffer.
///
/// `Bgr565` means the produced pixels carry red and blue swapped (`B<<11 | G<<5 | R`).
/// The DRM scanout buffer is still tagged `Rgb565`, because that is the only 565 format
/// the ST7365P plane advertises; the swap lives in the pixels, not the fourcc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayPixelFormat {
    Xrgb8888,
    Bgr565,
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
    pub pixel_format: DisplayPixelFormat,
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

/// ESP32 WiFi over SDIO (BMM101): a mac80211 device on the STM32 SD/MMC controller.
const WIFI_SDIO_ESP32: &str =
    "/sys/devices/platform/soc/48004000.sdmmc/mmc_host/mmc2/mmc2:0001/mmc2:0001:1";
/// Realtek USB WiFi behind the on-board USB hub (BMC100 hubbed revision and BFM100).
const WIFI_USB_HUBBED: &str =
    "/sys/devices/platform/soc/5800d000.usbh-ehci/usb3/3-1/3-1.1/3-1.1:1.0";
/// Realtek USB WiFi wired directly to the EHCI root port (BMC100 hubless revision).
const WIFI_USB_HUBLESS: &str = "/sys/devices/platform/soc/5800d000.usbh-ehci/usb3/3-1/3-1:1.0";

/// BMC100 boards carry the USB hub from PCB version `000200` on;
/// later revisions are expected to keep it.
const BMC100_FIRST_HUBBED_VERSION: PcbVersion = PcbVersion::new(0, 0x02, 0);

/// The WiFi radio a board carries and where it sits on the bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WifiChip {
    /// USB-attached nl80211 radio (BMC100, BFM100).
    UsbNl80211 { syspath: PathBuf },
    /// ESP32 companion radio on SDIO (BMM101).
    SdioEsp32 { syspath: PathBuf },
}

impl WifiChip {
    fn usb(syspath: &str) -> Self {
        Self::UsbNl80211 {
            syspath: PathBuf::from(syspath),
        }
    }

    #[must_use]
    pub fn syspath(&self) -> &Path {
        match self {
            Self::UsbNl80211 { syspath } | Self::SdioEsp32 { syspath } => syspath,
        }
    }
}

enum WifiLocation {
    Fixed(WifiChip),
    Probe(Vec<WifiChip>),
}

impl WifiLocation {
    fn locate_with(self, exists: impl Fn(&Path) -> bool) -> WifiChip {
        match self {
            Self::Fixed(chip) => chip,
            Self::Probe(candidates) => candidates
                .iter()
                .find(|chip| exists(chip.syspath()))
                .cloned()
                .or_else(|| candidates.into_iter().next())
                .expect("BUG: probe candidate lists are built non-empty"),
        }
    }
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
                    pixel_format: DisplayPixelFormat::Xrgb8888,
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
                    pixel_format: DisplayPixelFormat::Bgr565,
                },
                slot_grid: None,
                led_strip: None,
                paths,
            },
            Product::Bmm101 => Self {
                product,
                display: DisplayProfile {
                    logical_width: 480,
                    logical_height: 320,
                    advertised_width: 480,
                    advertised_height: 320,
                    shape: DisplayShape::Rectangular,
                    dpi: 165,
                    scanout_transform: DisplayTransform::Deg0,
                    touch_transform: TouchTransform::Deg0,
                    visible_area: VisibleArea {
                        x: 0,
                        y: 0,
                        width: 480,
                        height: 320,
                    },
                    seam_overlap_px: 0,
                    pixel_format: DisplayPixelFormat::Bgr565,
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
                    pixel_format: DisplayPixelFormat::Xrgb8888,
                },
                slot_grid: None,
                led_strip: None,
                paths,
            },
        }
    }

    /// The board's WiFi radio, or `None` when the board carries no radio at all —
    /// not merely one that has yet to enumerate.
    ///
    /// A Deck serial pins BMC100 to the hubbed or hubless syspath
    /// by PCB revision; without one the candidates are probed for existence,
    /// falling back to the primary when the radio has not enumerated yet;
    /// the syspath is opened lazily.
    #[must_use]
    pub fn locate_wifi_chip(&self, serial: Option<&BoardSerial>) -> Option<WifiChip> {
        Some(self.wifi_location(serial)?.locate_with(Path::exists))
    }

    fn wifi_location(&self, serial: Option<&BoardSerial>) -> Option<WifiLocation> {
        match self.product {
            Product::Bmc100 => {
                let deck_version = serial
                    .filter(|serial| serial.product() == serial_number::PRODUCT_DECK)
                    .map(BoardSerial::pcb_version);
                Some(match deck_version {
                    Some(version) if version >= BMC100_FIRST_HUBBED_VERSION => {
                        WifiLocation::Fixed(WifiChip::usb(WIFI_USB_HUBBED))
                    }
                    Some(_) => WifiLocation::Fixed(WifiChip::usb(WIFI_USB_HUBLESS)),
                    None => WifiLocation::Probe(vec![
                        WifiChip::usb(WIFI_USB_HUBBED),
                        WifiChip::usb(WIFI_USB_HUBLESS),
                    ]),
                })
            }
            Product::Bmm100 => None,
            Product::Bmm101 => Some(WifiLocation::Fixed(WifiChip::SdioEsp32 {
                syspath: PathBuf::from(WIFI_SDIO_ESP32),
            })),
            Product::Bfm100 => Some(WifiLocation::Fixed(WifiChip::usb(WIFI_USB_HUBBED))),
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
    fn default_http_port_is_80_only_for_bmc100() {
        assert_eq!(Product::Bmc100.default_http_port(), 80);
        assert_eq!(Product::Bmm100.default_http_port(), 81);
        assert_eq!(Product::Bmm101.default_http_port(), 81);
        assert_eq!(Product::Bfm100.default_http_port(), 81);
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
    fn non_bmc100_profiles_have_expected_display_geometry() {
        let cases = [
            (
                Product::Bmm100,
                320,
                240,
                DisplayShape::Rectangular,
                DisplayTransform::Deg0,
            ),
            (
                Product::Bmm101,
                480,
                320,
                DisplayShape::Rectangular,
                DisplayTransform::Deg0,
            ),
            (
                Product::Bfm100,
                480,
                480,
                DisplayShape::Round,
                DisplayTransform::Deg90,
            ),
        ];

        for (product, width, height, shape, transform) in cases {
            let profile = HardwareProfile::for_product(product);
            assert_eq!(
                (
                    profile.display.logical_width,
                    profile.display.logical_height
                ),
                (width, height),
                "{product:?}: logical display"
            );
            assert_eq!(
                (
                    profile.display.advertised_width,
                    profile.display.advertised_height
                ),
                (width, height),
                "{product:?}: advertised mode"
            );
            assert_eq!(
                (
                    profile.display.visible_area.x,
                    profile.display.visible_area.y,
                    profile.display.visible_area.width,
                    profile.display.visible_area.height,
                ),
                (0, 0, width, height),
                "{product:?}: visible area"
            );
            assert_eq!(profile.display.shape, shape, "{product:?}: shape");
            assert_eq!(
                profile.display.scanout_transform, transform,
                "{product:?}: scanout transform"
            );
        }
    }

    /// A synthetic Deck serial `BF0001B00yy00B0000000000`
    /// with the given packed `yy` version component.
    fn deck_serial(version_yy: u8) -> BoardSerial {
        let mut raw = [
            0xBF, 0x00, 0x01, 0xB0, 0x00, 0x00, 0x0B, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        raw[4] |= version_yy >> 4;
        raw[5] = version_yy << 4;
        BoardSerial::parse(raw).expect("BUG: synthetic deck serial must parse")
    }

    fn usb_chip(syspath: &str) -> WifiChip {
        WifiChip::UsbNl80211 {
            syspath: PathBuf::from(syspath),
        }
    }

    #[test]
    fn serial_revision_pins_bmc100_wifi_chip() {
        let cases = [
            (0x01, WIFI_USB_HUBLESS),
            (0x02, WIFI_USB_HUBBED),
            (0x03, WIFI_USB_HUBBED),
        ];
        for (version_yy, expected) in cases {
            let chip = HardwareProfile::for_product(Product::Bmc100)
                .locate_wifi_chip(Some(&deck_serial(version_yy)));
            assert_eq!(
                chip,
                Some(usb_chip(expected)),
                "version yy={version_yy:#04x}"
            );
        }
    }

    #[test]
    fn bmc100_without_serial_probes_first_existing_candidate() {
        let location = HardwareProfile::for_product(Product::Bmc100)
            .wifi_location(None)
            .expect("BUG: BMC100 carries a WiFi radio");
        let chip = location.locate_with(|path| path == Path::new(WIFI_USB_HUBLESS));
        assert_eq!(chip, usb_chip(WIFI_USB_HUBLESS));
    }

    #[test]
    fn bmc100_probe_falls_back_to_hubbed_when_nothing_exists() {
        let location = HardwareProfile::for_product(Product::Bmc100)
            .wifi_location(None)
            .expect("BUG: BMC100 carries a WiFi radio");
        let chip = location.locate_with(|_| false);
        assert_eq!(
            chip,
            usb_chip(WIFI_USB_HUBBED),
            "hubbed is the primary candidate"
        );
    }

    #[test]
    fn bmc100_probe_prefers_hubbed_when_both_exist() {
        let location = HardwareProfile::for_product(Product::Bmc100)
            .wifi_location(None)
            .expect("BUG: BMC100 carries a WiFi radio");
        let chip = location.locate_with(|_| true);
        assert_eq!(chip, usb_chip(WIFI_USB_HUBBED), "hubbed outranks hubless");
    }

    #[test]
    fn non_deck_serial_probes_bmc100_candidates() {
        let mut raw = [
            0xBF, 0x00, 0x01, 0xB0, 0x00, 0x20, 0x0B, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        raw[2] = 0x02;
        let foreign = BoardSerial::parse(raw).expect("BUG: product 0002 serial must parse");
        let location = HardwareProfile::for_product(Product::Bmc100)
            .wifi_location(Some(&foreign))
            .expect("BUG: BMC100 carries a WiFi radio");
        let chip = location.locate_with(|path| path == Path::new(WIFI_USB_HUBLESS));
        assert_eq!(
            chip,
            usb_chip(WIFI_USB_HUBLESS),
            "a foreign serial must fall back to probing, not pin a path"
        );
    }

    #[test]
    fn fixed_products_ignore_the_serial() {
        let cases = [
            (
                Product::Bmm101,
                WifiChip::SdioEsp32 {
                    syspath: PathBuf::from(WIFI_SDIO_ESP32),
                },
            ),
            (Product::Bfm100, usb_chip(WIFI_USB_HUBBED)),
        ];
        for (product, expected) in cases {
            let chip =
                HardwareProfile::for_product(product).locate_wifi_chip(Some(&deck_serial(0x01)));
            assert_eq!(chip, Some(expected), "{product:?}");
        }
    }

    #[test]
    fn bmm100_carries_no_wifi_radio() {
        let profile = HardwareProfile::for_product(Product::Bmm100);
        assert_eq!(profile.locate_wifi_chip(None), None);
        assert_eq!(profile.locate_wifi_chip(Some(&deck_serial(0x01))), None);
    }

    #[test]
    fn pixel_format_is_bgr565_only_for_bmm() {
        let cases = [
            (Product::Bmc100, DisplayPixelFormat::Xrgb8888),
            (Product::Bmm100, DisplayPixelFormat::Bgr565),
            (Product::Bmm101, DisplayPixelFormat::Bgr565),
            (Product::Bfm100, DisplayPixelFormat::Xrgb8888),
        ];
        for (product, expected) in cases {
            let profile = HardwareProfile::for_product(product);
            assert_eq!(profile.display.pixel_format, expected, "{product:?}");
        }
    }

    #[test]
    fn only_the_deck_has_an_upgrade_asset() {
        assert!(IndexBmcPlatform::try_from(BosPlatform::Bmc1).is_ok());
        for platform in [BosPlatform::Am2, BosPlatform::Bmm1, BosPlatform::Bfm1] {
            assert!(IndexBmcPlatform::try_from(platform).is_err());
        }
    }

    #[test]
    fn profile_override_maps_codes_and_auto() {
        assert_eq!(
            "auto"
                .parse::<HardwareProfileSelection>()
                .expect("BUG: \"auto\" is a valid profile override"),
            HardwareProfileSelection::Auto
        );
        assert_eq!(
            "bmc100"
                .parse::<HardwareProfileSelection>()
                .expect("BUG: \"bmc100\" is a valid profile override"),
            HardwareProfileSelection::Platform(BosPlatform::Bmc1)
        );
        assert_eq!(
            "BFM100"
                .parse::<HardwareProfileSelection>()
                .expect("BUG: \"BFM100\" is a valid profile override"),
            HardwareProfileSelection::Platform(BosPlatform::Bfm1)
        );
        assert!("nope".parse::<HardwareProfileSelection>().is_err());
        assert_eq!(
            Option::<BosPlatform>::from(HardwareProfileSelection::Auto),
            None
        );
        assert_eq!(
            Option::<BosPlatform>::from(HardwareProfileSelection::Platform(BosPlatform::Am2)),
            Some(BosPlatform::Am2)
        );
    }
}
