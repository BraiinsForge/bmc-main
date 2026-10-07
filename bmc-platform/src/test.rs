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
fn connectivity_and_mining_capabilities_per_product() {
    let cases = [
        (Product::Bmc100, true, false, false, false),
        (Product::Bmm100, false, true, true, true),
        (Product::Bmm101, true, true, true, true),
        (Product::Bfm100, true, true, true, true),
    ];
    for (product, wifi, ethernet, mining, boser) in cases {
        let caps = HardwareProfile::for_product(product).capabilities();
        assert_eq!(caps.wifi_supported, wifi, "{product:?}: wifi");
        assert_eq!(caps.ethernet_supported, ethernet, "{product:?}: ethernet");
        assert_eq!(caps.mining_supported, mining, "{product:?}: mining");
        assert_eq!(caps.boser_managed, boser, "{product:?}: boser");
    }
}

#[tokio::test]
async fn reset_button_ownership_follows_provisioning_on_every_product() {
    for (product, after_setup) in [
        (Product::Bmc100, ResetButtonOwner::Bmc),
        (Product::Bmm100, ResetButtonOwner::Boser),
        (Product::Bmm101, ResetButtonOwner::Boser),
        (Product::Bfm100, ResetButtonOwner::Boser),
    ] {
        let caps = HardwareProfile::for_product(product).capabilities();
        for (state, expected) in [
            (BmcState::FactoryDefault, ResetButtonOwner::Bmc),
            (BmcState::SetupPending, ResetButtonOwner::Bmc),
            (BmcState::Operational, after_setup),
            (BmcState::WifiReconfiguration, after_setup),
            (BmcState::Unsupported, after_setup),
        ] {
            assert_eq!(
                caps.reset_button_owner(std::future::ready(state)).await,
                expected,
                "{product:?} in {state:?}: BMC owns initial setup; the configured owner handles other states"
            );
        }
    }
}

#[tokio::test]
async fn self_managed_reset_button_never_reads_provisioning() {
    let caps = HardwareProfile::for_product(Product::Bmc100).capabilities();
    let owner = caps
        .reset_button_owner(async {
            panic!("self-managed reset must not depend on a provisioning read")
        })
        .await;
    assert_eq!(owner, ResetButtonOwner::Bmc);
}

#[test]
fn alarm_support_follows_sound_not_the_led_strip() {
    let sound_only = HardwareProfile {
        led_strip: None,
        ..HardwareProfile::for_product(Product::Bmc100)
    }
    .capabilities();
    assert!(sound_only.sound_supported);
    assert!(!sound_only.led_supported);
    assert!(sound_only.alarm_supported);

    let led_only = HardwareProfile {
        led_strip: HardwareProfile::for_product(Product::Bmc100).led_strip,
        ..HardwareProfile::for_product(Product::Bmm101)
    }
    .capabilities();
    assert!(!led_only.sound_supported);
    assert!(led_only.led_supported);
    assert!(
        !led_only.alarm_supported,
        "an LED strip alone does not make an alarm"
    );
}

#[test]
fn alarm_output_capabilities_per_product() {
    let cases = [
        (Product::Bmc100, true, true, true),
        (Product::Bmm100, false, false, false),
        (Product::Bmm101, false, false, false),
        (Product::Bfm100, false, false, false),
    ];
    for (product, sound, led, alarm) in cases {
        let caps = HardwareProfile::for_product(product).capabilities();
        assert_eq!(caps.sound_supported, sound, "{product:?}: sound");
        assert_eq!(caps.led_supported, led, "{product:?}: LED");
        assert_eq!(caps.alarm_supported, alarm, "{product:?}: alarm");
    }
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
        let chip = HardwareProfile::for_product(product).locate_wifi_chip(Some(&deck_serial(0x01)));
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
fn readability_adjustment_is_enabled_only_for_bmm_panels() {
    for product in [
        Product::Bmc100,
        Product::Bmm100,
        Product::Bmm101,
        Product::Bfm100,
    ] {
        let adjustment = HardwareProfile::for_product(product)
            .display
            .color_adjustment;
        assert_eq!(
            adjustment.is_some(),
            matches!(product, Product::Bmm100 | Product::Bmm101),
            "only BMM panels should use the readability adjustment: {product:?}"
        );
    }
}

#[test]
fn bmm101_preserves_both_ticker_gradient_fills() {
    let adjustment = HardwareProfile::for_product(Product::Bmm101)
        .display
        .color_adjustment
        .expect("BUG: BMM101 must carry its tested readability profile");
    let floor_byte = (adjustment.shadow_floor * 255.0).round();
    for peak in [190.0_f32, 250.0] {
        let fill_byte = (0.15 * peak).round();
        assert!(
            fill_byte <= floor_byte,
            "preserve both trend colors throughout the 2–15% ticker fade: \
             fill {fill_byte} above floor {floor_byte}"
        );
    }
}

#[test]
#[should_panic(expected = "requires 0 < floor < input <= output < 1")]
fn darkening_adjustment_cannot_be_constructed() {
    let _ = ColorAdjustment::new(0.1, 0.5, 0.4);
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
