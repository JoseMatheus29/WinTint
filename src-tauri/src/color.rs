//! Small color math shared by the modules.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const WHITE: Rgb = Rgb { r: 255, g: 255, b: 255 };
    pub const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };

    /// Reads "#rrggbb", the only form the config uses.
    pub fn parse(hex: &str) -> Option<Rgb> {
        let digits = hex.strip_prefix('#')?;
        if digits.len() != 6 {
            return None;
        }
        let value = u32::from_str_radix(digits, 16).ok()?;
        Some(Rgb {
            r: (value >> 16) as u8,
            g: (value >> 8) as u8,
            b: value as u8,
        })
    }

    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    fn channels(self) -> [f64; 3] {
        [self.r, self.g, self.b].map(|c| c as f64 / 255.0)
    }

    pub fn luminance(self) -> f64 {
        let [r, g, b] = self.channels().map(|c| {
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        });
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// Hue in degrees, saturation and lightness from 0 to 1.
    pub fn hsl(self) -> (f64, f64, f64) {
        let [r, g, b] = self.channels();
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let l = (max + min) / 2.0;
        if delta == 0.0 {
            return (0.0, 0.0, l);
        }
        let s = delta / (1.0 - (2.0 * l - 1.0).abs());
        let sector = if max == r {
            ((g - b) / delta).rem_euclid(6.0)
        } else if max == g {
            (b - r) / delta + 2.0
        } else {
            (r - g) / delta + 4.0
        };
        (sector * 60.0, s, l)
    }

    pub fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
        let chroma = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let channel = |offset: f64| {
            let k = (offset + h / 30.0) % 12.0;
            let value = l - chroma * 0.5 * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0);
            (value * 255.0).round().clamp(0.0, 255.0) as u8
        };
        Rgb {
            r: channel(0.0),
            g: channel(8.0),
            b: channel(4.0),
        }
    }

    /// Moves `amount` of the way toward `other`.
    pub fn mix(self, other: Rgb, amount: f64) -> Rgb {
        let blend = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * amount).round() as u8;
        Rgb {
            r: blend(self.r, other.r),
            g: blend(self.g, other.g),
            b: blend(self.b, other.b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Rgb;

    #[test]
    fn hex_round_trip() {
        let color = Rgb::parse("#0078d4").unwrap();
        assert_eq!(color, Rgb { r: 0, g: 0x78, b: 0xd4 });
        assert_eq!(color.hex(), "#0078d4");
        assert_eq!(Rgb::parse("0078d4"), None);
        assert_eq!(Rgb::parse("#12345"), None);
    }

    #[test]
    fn hsl_round_trip() {
        for hex in ["#0078d4", "#f97316", "#e6004c", "#4ade80", "#808080"] {
            let color = Rgb::parse(hex).unwrap();
            let (h, s, l) = color.hsl();
            assert_eq!(Rgb::from_hsl(h, s, l), color, "{hex}");
        }
    }
}
