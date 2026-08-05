//! Colour conversion and perceptual distance.
//!
//! Matching a Source texture to a Minecraft block is a nearest-colour search,
//! and doing that in sRGB gives poor answers: sRGB distance is dominated by
//! brightness and treats a green shift as far smaller than it looks. Oklab is
//! near-uniform perceptually, so plain Euclidean distance in it behaves like
//! "how different do these look".

/// A colour in Oklab: `l` is lightness in 0..1, `a` and `b` are the opponent
/// axes, roughly -0.4..0.4.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklab {
    pub l: f64,
    pub a: f64,
    pub b: f64,
}

/// sRGB channel (0..1) to linear light.
pub fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light to an sRGB channel (0..1).
pub fn linear_to_srgb(c: f64) -> f64 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Linear RGB, each channel 0..1, to Oklab.
pub fn oklab_from_linear(rgb: [f64; 3]) -> Oklab {
    let [r, g, b] = rgb;
    let l = 0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b;
    let m = 0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b;
    let s = 0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b;

    let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
    Oklab {
        l: 0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        a: 1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        b: 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    }
}

/// An 8-bit sRGB triple to Oklab.
pub fn oklab_from_srgb(rgb: [u8; 3]) -> Oklab {
    oklab_from_linear([
        srgb_to_linear(rgb[0] as f64 / 255.0),
        srgb_to_linear(rgb[1] as f64 / 255.0),
        srgb_to_linear(rgb[2] as f64 / 255.0),
    ])
}

/// Linear RGB to an 8-bit sRGB triple, for display.
pub fn srgb_from_linear(rgb: [f64; 3]) -> [u8; 3] {
    rgb.map(|c| (linear_to_srgb(c.clamp(0.0, 1.0)) * 255.0).round() as u8)
}

/// Squared perceptual distance. Squared because only the ordering matters and
/// the square root would be pure cost.
pub fn distance_squared(a: Oklab, b: Oklab) -> f64 {
    let (dl, da, db) = (a.l - b.l, a.a - b.a, a.b - b.b);
    dl * dl + da * da + db * db
}

/// `#rrggbb`, for reports.
pub fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_and_linear_round_trip() {
        for step in 0..=100 {
            let c = step as f64 / 100.0;
            let back = linear_to_srgb(srgb_to_linear(c));
            assert!((back - c).abs() < 1e-9, "{c} -> {back}");
        }
    }

    /// The two transfer curves must agree where they change over, or mid-greys
    /// come out visibly wrong.
    #[test]
    fn transfer_curve_is_continuous_at_the_join() {
        let below = srgb_to_linear(0.040_44);
        let above = srgb_to_linear(0.040_46);
        assert!((above - below).abs() < 1e-5);
    }

    #[test]
    fn white_is_light_and_neutral() {
        let white = oklab_from_srgb([255, 255, 255]);
        assert!((white.l - 1.0).abs() < 1e-3, "{white:?}");
        assert!(white.a.abs() < 1e-3 && white.b.abs() < 1e-3, "{white:?}");
    }

    #[test]
    fn black_is_the_origin() {
        let black = oklab_from_srgb([0, 0, 0]);
        assert!(black.l.abs() < 1e-6 && black.a.abs() < 1e-6 && black.b.abs() < 1e-6);
    }

    /// The point of Oklab here: a grey must be nearer another grey than a
    /// saturated colour of the same lightness.
    #[test]
    fn greys_are_nearer_greys_than_hues() {
        let grey = oklab_from_srgb([128, 128, 128]);
        let other_grey = oklab_from_srgb([150, 150, 150]);
        let red = oklab_from_srgb([190, 60, 60]);
        assert!(distance_squared(grey, other_grey) < distance_squared(grey, red));
    }

    #[test]
    fn linear_display_round_trips_through_srgb() {
        let rgb = srgb_from_linear([
            srgb_to_linear(0.5),
            srgb_to_linear(0.25),
            srgb_to_linear(1.0),
        ]);
        assert_eq!(rgb, [128, 64, 255]);
    }

    #[test]
    fn hex_is_lowercase_and_padded() {
        assert_eq!(hex([0, 15, 255]), "#000fff");
    }
}
