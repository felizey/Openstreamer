use super::*;

#[test]
fn full_hd_progressive_60_uses_cta16_even_without_edid() {
    let mode = find("1920x1080@60.00").unwrap();
    assert_eq!(mode.size(), (1920, 1080));
    assert_eq!(mode.clock(), 148500);
    assert_eq!(mode.hsync(), (2008, 2052, 2200));
    assert_eq!(mode.vsync(), (1084, 1089, 1125));
    assert!(!mode.flags().contains(ModeFlags::INTERLACE));
    assert_eq!(describe(&mode).refresh, 60.0);
}

#[test]
fn interlaced_fields_are_not_confused_with_progressive_frames() {
    let progressive = find("1920x1080@30.00").unwrap();
    let interlaced = find("1920x1080i@60.00").unwrap();
    assert_ne!(progressive, interlaced);
    assert_eq!(interlaced.vsync(), (1084, 1094, 1125));
    assert_eq!(describe(&interlaced).refresh, 60.0);
    assert!(describe(&interlaced).interlaced);
    assert!(!describe(&progressive).interlaced);
    for id in [
        "1920x1080i@50.00",
        "1920x1080i@59.94",
        "720x576i@50.00",
        "720x480i@59.94",
    ] {
        assert!(describe(&find(id).unwrap()).interlaced, "{id}");
    }
}

#[test]
fn catalogue_survives_no_screen_and_keeps_fractional_rates_distinct() {
    let modes = available(Vec::new());
    assert_eq!(modes.len(), 31);
    let ids: std::collections::HashSet<_> = modes.iter().map(|mode| &mode.id).collect();
    assert_eq!(ids.len(), modes.len());
    for id in [
        "1920x1080@60.00",
        "1920x1080@59.94",
        "3840x2160@30.00",
        "3840x2160@29.97",
    ] {
        assert!(ids.contains(&id.to_string()));
    }
    assert!(modes.iter().all(|mode| mode.manual && !mode.advertised));
}

#[test]
fn screen_modes_supplement_presets_and_keep_preference_metadata() {
    let mut reported = describe(&find("1920x1080@60.00").unwrap());
    reported.preferred = true;
    reported.fits_screen = false;
    let mut custom = reported.clone();
    custom.id = "custom-screen-mode".into();
    let modes = available(vec![reported, custom]);
    assert_eq!(modes.len(), 32);
    let preset = modes
        .iter()
        .find(|mode| mode.id == "1920x1080@60.00")
        .unwrap();
    assert!(preset.manual && preset.advertised && preset.preferred);
    assert!(!preset.fits_screen);
    let custom = modes
        .iter()
        .find(|mode| mode.id == "custom-screen-mode")
        .unwrap();
    assert!(!custom.manual && custom.advertised);
    assert!(find("custom-screen-mode").is_none());
    assert!(find("auto").is_none());
}

#[test]
fn every_preset_has_ordered_sync_ranges_and_consistent_field_rate() {
    for mode in presets() {
        let (hs, he, ht) = mode.hsync();
        let (vs, ve, vt) = mode.vsync();
        let (w, h) = mode.size();
        assert!(w < hs && hs < he && he < ht, "{mode:?}");
        assert!(h < vs && vs < ve && ve < vt, "{mode:?}");
        assert_eq!(describe(&mode).refresh.round() as u32, mode.vrefresh());
        assert!(!mode.flags().contains(ModeFlags::DBLSCAN));
    }
}
