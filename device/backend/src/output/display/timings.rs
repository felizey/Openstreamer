use super::{ModeInfo, describe};
use drm::control::{Mode, ModeFlags, ModeTypeFlags};

const PP: u32 = ModeFlags::PHSYNC.bits() | ModeFlags::PVSYNC.bits();
const NN: u32 = ModeFlags::NHSYNC.bits() | ModeFlags::NVSYNC.bits();
const NP: u32 = ModeFlags::NHSYNC.bits() | ModeFlags::PVSYNC.bits();
const I: u32 = ModeFlags::INTERLACE.bits();
const D: u32 = ModeFlags::DBLCLK.bits();

#[derive(Clone, Copy)]
struct Timing {
    clock: u32,
    h: [u16; 4],
    v: [u16; 4],
    flags: u32,
    fractional: bool,
}

// Standard CTA-861 and VESA DMT timings, in kHz/pixels/lines. See the Linux
// DRM timing tables: drivers/gpu/drm/drm_edid.c (edid_cea_modes_1/drm_dmt_modes).
// Interlaced timings are distinct standards, never a flag added to a p timing.
const TIMINGS: &[Timing] = &[
    Timing {
        clock: 148500,
        h: [1920, 2008, 2052, 2200],
        v: [1080, 1084, 1089, 1125],
        flags: PP,
        fractional: true,
    },
    Timing {
        clock: 148500,
        h: [1920, 2448, 2492, 2640],
        v: [1080, 1084, 1089, 1125],
        flags: PP,
        fractional: false,
    },
    Timing {
        clock: 74250,
        h: [1920, 2008, 2052, 2200],
        v: [1080, 1084, 1089, 1125],
        flags: PP,
        fractional: true,
    },
    Timing {
        clock: 74250,
        h: [1920, 2448, 2492, 2640],
        v: [1080, 1084, 1089, 1125],
        flags: PP,
        fractional: false,
    },
    Timing {
        clock: 74250,
        h: [1920, 2558, 2602, 2750],
        v: [1080, 1084, 1089, 1125],
        flags: PP,
        fractional: true,
    },
    Timing {
        clock: 74250,
        h: [1920, 2008, 2052, 2200],
        v: [1080, 1084, 1094, 1125],
        flags: PP | I,
        fractional: true,
    },
    Timing {
        clock: 74250,
        h: [1920, 2448, 2492, 2640],
        v: [1080, 1084, 1094, 1125],
        flags: PP | I,
        fractional: false,
    },
    Timing {
        clock: 74250,
        h: [1280, 1390, 1430, 1650],
        v: [720, 725, 730, 750],
        flags: PP,
        fractional: true,
    },
    Timing {
        clock: 74250,
        h: [1280, 1720, 1760, 1980],
        v: [720, 725, 730, 750],
        flags: PP,
        fractional: false,
    },
    Timing {
        clock: 27000,
        h: [720, 732, 796, 864],
        v: [576, 581, 586, 625],
        flags: NN,
        fractional: false,
    },
    Timing {
        clock: 13500,
        h: [720, 732, 795, 864],
        v: [576, 580, 586, 625],
        flags: NN | I | D,
        fractional: false,
    },
    Timing {
        clock: 27027,
        h: [720, 736, 798, 858],
        v: [480, 489, 495, 525],
        flags: NN,
        fractional: true,
    },
    Timing {
        clock: 13514,
        h: [720, 739, 801, 858],
        v: [480, 488, 494, 525],
        flags: NN | I | D,
        fractional: true,
    },
    Timing {
        clock: 297000,
        h: [3840, 4016, 4104, 4400],
        v: [2160, 2168, 2178, 2250],
        flags: PP,
        fractional: true,
    },
    Timing {
        clock: 297000,
        h: [3840, 4896, 4984, 5280],
        v: [2160, 2168, 2178, 2250],
        flags: PP,
        fractional: false,
    },
    Timing {
        clock: 297000,
        h: [3840, 5116, 5204, 5500],
        v: [2160, 2168, 2178, 2250],
        flags: PP,
        fractional: true,
    },
    Timing {
        clock: 85500,
        h: [1360, 1424, 1536, 1792],
        v: [768, 771, 777, 795],
        flags: PP,
        fractional: false,
    },
    Timing {
        clock: 108000,
        h: [1280, 1328, 1440, 1688],
        v: [1024, 1025, 1028, 1066],
        flags: PP,
        fractional: false,
    },
    Timing {
        clock: 83500,
        h: [1280, 1352, 1480, 1680],
        v: [800, 803, 809, 831],
        flags: NP,
        fractional: false,
    },
    Timing {
        clock: 65000,
        h: [1024, 1048, 1184, 1344],
        v: [768, 771, 777, 806],
        flags: NN,
        fractional: false,
    },
    Timing {
        clock: 40000,
        h: [800, 840, 968, 1056],
        v: [600, 601, 605, 628],
        flags: PP,
        fractional: false,
    },
    Timing {
        clock: 25175,
        h: [640, 656, 752, 800],
        v: [480, 490, 492, 525],
        flags: NN,
        fractional: false,
    },
];

fn mode(t: Timing, fractional: bool) -> Mode {
    let clock = if fractional {
        // CTA's 480-line base modes are already at 1000/1001 rates.
        if t.v[0] == 480 && t.h[0] == 720 {
            if t.flags & I != 0 { 13500 } else { 27000 }
        } else {
            ((u64::from(t.clock) * 1000 + 500) / 1001) as u32
        }
    } else {
        t.clock
    };
    let fields = if t.flags & I != 0 { 2 } else { 1 };
    let mut raw = drm_ffi::drm_mode_modeinfo {
        clock,
        hdisplay: t.h[0],
        hsync_start: t.h[1],
        hsync_end: t.h[2],
        htotal: t.h[3],
        vdisplay: t.v[0],
        vsync_start: t.v[1],
        vsync_end: t.v[2],
        vtotal: t.v[3],
        flags: t.flags,
        type_: ModeTypeFlags::USERDEF.bits(),
        vrefresh: (f64::from(clock) * 1000.0 * f64::from(fields)
            / (f64::from(t.h[3]) * f64::from(t.v[3])))
        .round() as u32,
        ..Default::default()
    };
    let name = format!(
        "{}x{}{}",
        t.h[0],
        t.v[0],
        if t.flags & I != 0 { "i" } else { "" }
    );
    for (out, byte) in raw.name.iter_mut().zip(name.bytes()) {
        *out = byte as _;
    }
    raw.into()
}

fn presets() -> impl Iterator<Item = Mode> {
    TIMINGS
        .iter()
        .flat_map(|&t| std::iter::once(mode(t, false)).chain(t.fractional.then(|| mode(t, true))))
}

pub(super) fn find(id: &str) -> Option<Mode> {
    presets().find(|mode| describe(mode).id == id)
}

pub(super) fn catalogue() -> Vec<ModeInfo> {
    presets()
        .map(|mode| {
            let mut info = describe(&mode);
            info.manual = true;
            info.advertised = false;
            info
        })
        .collect()
}

pub(super) fn available(reported: Vec<ModeInfo>) -> Vec<ModeInfo> {
    let mut modes = catalogue();
    for mut info in reported {
        info.advertised = true;
        if let Some(existing) = modes.iter_mut().find(|mode| mode.id == info.id) {
            info.manual = true;
            *existing = info;
        } else {
            modes.push(info);
        }
    }
    modes
}

#[cfg(test)]
#[path = "timings.test.rs"]
mod tests;
