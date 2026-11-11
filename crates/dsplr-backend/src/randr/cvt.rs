//! VESA CVT timings for a mode the X server has to be given whole.
//!
//! Transcribed from `libxcvt_gen_mode_info` (`lib/libxcvt.c`, libxcvt
//! 0.1.3), the generator behind `cvt`, which `xrandr --newmode` lines are
//! usually made with. libxcvt is MIT-licensed and asks for its notice in
//! copies: it is in `src/randr/LICENSES/libxcvt.txt`, verbatim from the
//! release's `COPYING`. Only its normal-blanking, progressive branch is
//! here, with margins off as libxcvt has them. The arithmetic keeps C's
//! types step by step (which sums are `float`, which `double`, where an
//! `int` truncates), since a step done in the wrong width moves a timing by
//! one, and the tests hold it to libxcvt's own output.

use super::resources::{ModeInfo, ModeXid, mode_flags};

/// Character cell granularity: the width is rounded down to it.
const H_GRANULARITY: i32 = 8;
/// Minimum vertical front porch, lines.
const MIN_V_PORCH: i32 = 3;
/// Minimum vertical back porch, lines.
const MIN_V_BPORCH: i32 = 6;
/// Minimum vertical sync plus back porch, microseconds.
const MIN_VSYNC_BP: f64 = 550.0;
/// Nominal horizontal sync width, percent of the line.
const HSYNC_PERCENTAGE: i32 = 8;
/// The blanking formula's `C'`: `(C - J) * K / 256 + J` with libxcvt's
/// defaults C 40, J 20, K 128.
const C_PRIME: i32 = (40 - 20) * 128 / 256 + 20;
/// The blanking formula's `M'`: `M * K / 256` with M 600, K 128.
const M_PRIME: i32 = 600 * 128 / 256;
/// The pixel clock is a multiple of this, kilohertz.
const CLOCK_STEP: u64 = 250;
/// The refresh CVT assumes when none is asked.
const DEFAULT_REFRESH: f32 = 60.0;

/// A CVT mode of `width` by `height` at `refresh` hertz (60 when `0`),
/// named as `cvt` names it (`1280x720_60.00`), with its id unset. `None`
/// for a zero size, when CVT gives no mode of that size (a width off the
/// 8-pixel grid, other than 1366 by 768), or when the timings do not fit a
/// `MODEINFO`.
pub fn mode(width: u16, height: u16, refresh: f32) -> Option<ModeInfo> {
    if width == 0 || height == 0 {
        return None;
    }
    let vrefresh = if refresh == 0.0 {
        DEFAULT_REFRESH
    } else {
        refresh
    };
    let hdisplay_asked = i32::from(width);
    let vdisplay = i32::from(height);

    // 2. Horizontal pixels.
    let mut hdisplay = hdisplay_asked - hdisplay_asked % H_GRANULARITY;

    // Determine vsync width from the aspect ratio.
    let aspect = |num: i32, den: i32| vdisplay % den == 0 && vdisplay * num / den == hdisplay;
    let vsync = if aspect(4, 3) {
        4
    } else if aspect(16, 9) {
        5
    } else if aspect(16, 10) {
        6
    } else if aspect(5, 4) || aspect(15, 9) {
        7
    } else {
        10
    };

    // 8. Estimated horizontal period, microseconds.
    let hperiod = (1_000_000.0 / f64::from(vrefresh) - MIN_VSYNC_BP) as f32
        / int_to_float(vdisplay + MIN_V_PORCH);

    // 9. Lines in sync and back porch.
    let estimated = (MIN_VSYNC_BP / f64::from(hperiod)) as i32 + 1;
    let vsync_and_back_porch = estimated.max(vsync + MIN_V_BPORCH);

    // 11. Lines in the field.
    let vtotal = vdisplay + vsync_and_back_porch + MIN_V_PORCH;

    // 12, 13. Blanking duty cycle, at least 20 percent.
    let hblank_percentage =
        (f64::from(C_PRIME) - f64::from(int_to_float(M_PRIME) * hperiod) / 1000.0) as f32;
    let hblank_percentage = hblank_percentage.max(20.0);
    let hblank = (f64::from(int_to_float(hdisplay) * hblank_percentage)
        / (100.0 - f64::from(hblank_percentage))) as i32;
    let hblank = hblank - hblank % (2 * H_GRANULARITY);

    // 14. Pixels in a line, and the sync pulses.
    let htotal = hdisplay + hblank;
    let mut hsync_end = hdisplay + hblank / 2;
    let hsync_width = htotal * HSYNC_PERCENTAGE / 100;
    let mut hsync_start = hsync_end - (hsync_width - hsync_width % H_GRANULARITY);
    let vsync_start = vdisplay + MIN_V_PORCH;
    let vsync_end = vsync_start + vsync;

    // 15. Pixel clock, kilohertz, down to its step.
    let dot_clock = (f64::from(htotal) * 1000.0 / f64::from(hperiod)) as u64;
    let dot_clock = dot_clock - dot_clock % CLOCK_STEP;

    // libxcvt's FWXGA hack: CVT cannot say 1366, so 1360 by 768 becomes it.
    if hdisplay == 1360 && vdisplay == 768 {
        hdisplay = 1366;
        hsync_start -= 1;
        hsync_end -= 1;
    }
    if hdisplay != hdisplay_asked {
        return None;
    }

    let timing = |value: i32| u16::try_from(value).ok();
    Some(ModeInfo {
        id: ModeXid(0),
        width,
        height,
        dot_clock: u32::try_from(dot_clock.checked_mul(1000)?).ok()?,
        hsync_start: timing(hsync_start)?,
        hsync_end: timing(hsync_end)?,
        htotal: timing(htotal)?,
        hskew: 0,
        vsync_start: timing(vsync_start)?,
        vsync_end: timing(vsync_end)?,
        vtotal: timing(vtotal)?,
        flags: mode_flags::HSYNC_NEGATIVE | mode_flags::VSYNC_POSITIVE,
        name: format!("{width}x{height}_{vrefresh:.2}"),
    })
}

/// C's conversion of an `int` to `float`, which rounds to the nearest
/// float; the values here are pixel and line counts, exact in an `f32`.
#[allow(clippy::cast_precision_loss)]
const fn int_to_float(value: i32) -> f32 {
    value as f32
}

#[cfg(test)]
mod tests {
    use super::mode;
    use crate::randr::resources::mode_flags;

    /// A size and refresh asked of CVT.
    type Asked = (u16, u16, f32);
    /// A dot clock in kilohertz, then hsync start, hsync end, htotal,
    /// vsync start, vsync end and vtotal.
    type Timings = (u32, [u16; 6]);

    /// `libxcvt_gen_mode_info(width, height, refresh, false, false)` from
    /// libxcvt 0.1.3, as `(width, height, refresh)` and `(dot clock kHz,
    /// hsync start, hsync end, htotal, vsync start, vsync end, vtotal)`.
    /// The last two are among the sizes where the estimated line period
    /// kept in a `double` rather than libxcvt's `float` moves the clock.
    const LIBXCVT: [(Asked, Timings); 18] = [
        (
            (1280, 720, 60.0),
            (74_500, [1344, 1472, 1664, 723, 728, 748]),
        ),
        (
            (1920, 1080, 60.0),
            (173_000, [2048, 2248, 2576, 1083, 1088, 1120]),
        ),
        (
            (2560, 1440, 144.0),
            (808_750, [2792, 3072, 3584, 1443, 1448, 1568]),
        ),
        (
            (1366, 768, 60.0),
            (84_750, [1431, 1567, 1776, 771, 781, 798]),
        ),
        (
            (1024, 768, 75.0),
            (82_000, [1088, 1192, 1360, 771, 775, 805]),
        ),
        (
            (3840, 2160, 30.0),
            (338_750, [4080, 4488, 5136, 2163, 2168, 2200]),
        ),
        (
            (1920, 1200, 59.95),
            (193_250, [2056, 2256, 2592, 1203, 1209, 1245]),
        ),
        (
            (1280, 1024, 60.0),
            (109_000, [1360, 1496, 1712, 1027, 1034, 1063]),
        ),
        (
            (1600, 1200, 60.0),
            (161_000, [1712, 1880, 2160, 1203, 1207, 1245]),
        ),
        (
            (1680, 1050, 60.0),
            (146_250, [1784, 1960, 2240, 1053, 1059, 1089]),
        ),
        (
            (1440, 900, 60.0),
            (106_500, [1520, 1672, 1904, 903, 909, 934]),
        ),
        ((800, 600, 60.0), (38_250, [832, 912, 1024, 603, 607, 624])),
        (
            (2560, 1080, 75.0),
            (294_000, [2744, 3016, 3472, 1083, 1093, 1130]),
        ),
        (
            (1366, 768, 0.0),
            (84_750, [1431, 1567, 1776, 771, 781, 798]),
        ),
        ((640, 480, 59.94), (23_750, [656, 720, 800, 483, 487, 500])),
        (
            (1920, 1080, 0.5),
            (1_250, [1968, 2160, 2400, 1083, 1088, 1094]),
        ),
        (
            (1048, 591, 100.0),
            (87_750, [1112, 1224, 1400, 594, 604, 629]),
        ),
        (
            (3104, 1747, 100.0),
            (799_750, [3368, 3712, 4320, 1750, 1760, 1852]),
        ),
    ];

    #[test]
    fn timings_are_libxcvts() {
        for ((width, height, refresh), (khz, [hss, hse, ht, vss, vse, vt])) in LIBXCVT {
            let mode = mode(width, height, refresh).expect("a mode");
            let got = (
                mode.dot_clock,
                [
                    mode.hsync_start,
                    mode.hsync_end,
                    mode.htotal,
                    mode.vsync_start,
                    mode.vsync_end,
                    mode.vtotal,
                ],
            );
            assert_eq!(
                got,
                (khz * 1000, [hss, hse, ht, vss, vse, vt]),
                "{width}x{height}@{refresh}"
            );
            assert_eq!((mode.width, mode.height), (width, height));
            assert_eq!(
                mode.flags,
                mode_flags::HSYNC_NEGATIVE | mode_flags::VSYNC_POSITIVE
            );
        }
    }

    #[test]
    fn named_as_cvt_names_it() {
        assert_eq!(
            mode(1280, 720, 60.0).expect("a mode").name,
            "1280x720_60.00"
        );
        assert_eq!(
            mode(1920, 1200, 59.95).expect("a mode").name,
            "1920x1200_59.95"
        );
        assert_eq!(mode(1366, 768, 0.0).expect("a mode").name, "1366x768_60.00");
    }

    #[test]
    fn a_width_off_the_grid_has_no_mode() {
        // libxcvt rounds 1930 down to 1928, which is not the size asked.
        assert_eq!(mode(1930, 1080, 60.0), None);
        assert!(mode(1928, 1080, 60.0).is_some());
    }

    #[test]
    fn its_refresh_is_the_one_cvt_reaches() {
        // libxcvt reports 59.855125 Hz for 1280x720 asked at 60.
        assert_eq!(mode(1280, 720, 60.0).expect("a mode").refresh_mhz(), 59_855);
    }
}
