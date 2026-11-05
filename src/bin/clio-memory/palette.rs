//! The colors of the setup screen: the palette of Iván's channel, in true color when the terminal
//! says it has it, else the nearest of the xterm 256, and none with `NO_COLOR` or no terminal.

use console::Style;

/// How many colors the screen uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    TrueColor,
    Ansi256,
    None,
}

/// `NO_COLOR` wins over everything, then no terminal; `COLORTERM` `truecolor` or `24bit` is true
/// color, anything else the 256.
pub fn color_mode(colorterm: Option<&str>, no_color: bool, is_term: bool) -> ColorMode {
    if no_color || !is_term {
        ColorMode::None
    } else if matches!(colorterm, Some("truecolor" | "24bit")) {
        ColorMode::TrueColor
    } else {
        ColorMode::Ansi256
    }
}

/// A color of the palette, as `(r, g, b)`.
pub type Rgb = (u8, u8, u8);

pub const HUESO: Rgb = (0xE6, 0xE6, 0xE0);
pub const CENIZA_CLARA: Rgb = (0xB8, 0xBC, 0xC4);
pub const CENIZA: Rgb = (0x8A, 0x8F, 0x98);
pub const BORDE: Rgb = (0x2A, 0x2E, 0x35);
pub const AMBAR: Rgb = (0xFF, 0xB1, 0x3D);
pub const HIELO: Rgb = (0x7F, 0xD6, 0xFF);
pub const VERDE_SUAVE: Rgb = (0x7F, 0xA8, 0x8A);
pub const ROJO_APAGADO: Rgb = (0xB4, 0x55, 0x4B);
/// Only the letters of the «Clio» stamp: the screen never paints its own background.
pub const GRAFITO: Rgb = (0x0D, 0x0F, 0x12);

/// One `Style` per name of the palette, all for stderr (where the screen draws).
#[derive(Clone)]
pub struct Palette {
    pub mode: ColorMode,
    /// Main text.
    pub hueso: Style,
    /// Supporting text: details, figures.
    pub ceniza_clara: Style,
    /// Secondary: the guide line, the hints, what is not checked.
    pub ceniza: Style,
    /// What waits for an answer (◆, the selection).
    pub ambar: Style,
    /// The empty part of a progress bar.
    pub borde: Style,
    /// What is going on (the spinner, the download bar).
    pub hielo: Style,
    /// A step done (◇, ✓).
    pub verde_suave: Style,
    /// An error (✖).
    pub rojo_apagado: Style,
    /// « Clio »: grafito letters on ámbar.
    pub sello: Style,
}

/// The style of `fg` (and `bg`) in `mode`: plain in `None`.
fn paint(mode: ColorMode, fg: Rgb, bg: Option<Rgb>) -> Style {
    let s = Style::new().for_stderr();
    match mode {
        ColorMode::None => s,
        ColorMode::TrueColor => {
            let s = s.true_color(fg.0, fg.1, fg.2);
            match bg {
                Some((r, g, b)) => s.on_true_color(r, g, b),
                None => s,
            }
        }
        ColorMode::Ansi256 => {
            let s = s.color256(nearest_256(fg.0, fg.1, fg.2));
            match bg {
                Some((r, g, b)) => s.on_color256(nearest_256(r, g, b)),
                None => s,
            }
        }
    }
}

pub fn palette(mode: ColorMode) -> Palette {
    let p = |fg: Rgb| paint(mode, fg, None);
    Palette {
        mode,
        hueso: p(HUESO),
        ceniza_clara: p(CENIZA_CLARA),
        ceniza: p(CENIZA),
        borde: p(BORDE),
        ambar: p(AMBAR),
        hielo: p(HIELO),
        verde_suave: p(VERDE_SUAVE),
        rojo_apagado: p(ROJO_APAGADO),
        sello: paint(mode, GRAFITO, Some(AMBAR)),
    }
}

/// `color` as a style in an indicatif template (`{spinner:.<this>}`): `#rrggbb`, the 256 index,
/// or nothing in `None`.
pub fn template_color(mode: ColorMode, (r, g, b): Rgb) -> Option<String> {
    match mode {
        ColorMode::None => None,
        ColorMode::TrueColor => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        ColorMode::Ansi256 => Some(nearest_256(r, g, b).to_string()),
    }
}

/// The xterm-256 color of `index` (16..=255).
fn xterm(index: u8) -> Rgb {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    if index >= 232 {
        let v = 8 + 10 * (index - 232);
        return (v, v, v);
    }
    let i = usize::from(index.saturating_sub(16));
    (LEVELS[i / 36], LEVELS[i / 6 % 6], LEVELS[i % 6])
}

/// The nearest xterm-256 index to a color: the 6×6×6 cube (16..=231) and the gray ramp
/// (232..=255), by euclidean distance; on a tie, the lower index.
fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    let distance = |i: u8| {
        let (xr, xg, xb) = xterm(i);
        let d = |a: u8, x: u8| (i32::from(a) - i32::from(x)).pow(2);
        d(r, xr) + d(g, xg) + d(b, xb)
    };
    // `min_by_key` keeps the first of equal ones: the lower index.
    (16..=255).min_by_key(|&i| distance(i)).unwrap_or(16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_mode_cases() {
        use ColorMode::{Ansi256, None, TrueColor};
        // NO_COLOR wins over a true-color terminal.
        assert_eq!(color_mode(Some("truecolor"), true, true), None);
        assert_eq!(color_mode(Option::None, true, true), None);
        // No terminal: no color, whatever COLORTERM says.
        assert_eq!(color_mode(Some("truecolor"), false, false), None);
        assert_eq!(color_mode(Some("24bit"), false, false), None);
        assert_eq!(color_mode(Some("truecolor"), false, true), TrueColor);
        assert_eq!(color_mode(Some("24bit"), false, true), TrueColor);
        assert_eq!(color_mode(Some(""), false, true), Ansi256);
        assert_eq!(color_mode(Some("yes"), false, true), Ansi256);
        assert_eq!(color_mode(Option::None, false, true), Ansi256);
    }

    // The cube's levels are 0, 95, 135, 175, 215, 255 (index 16 + 36·r + 6·g + b); the ramp's
    // grays are 8 + 10·i (index 232 + i).
    #[test]
    fn nearest_256_of_palette() {
        // ámbar (255, 177, 61): r 255 → 5, g 177 → 175 (3), b 61 → 95 (1, 34 away; 0 is 61):
        // 16 + 180 + 18 + 1 = 215, at 2² + 34² = 1160; the nearest gray, 168, is far.
        assert_eq!(nearest_256(0xFF, 0xB1, 0x3D), 215);
        // hielo (127, 214, 255): 127 → 135 (2), 214 → 215 (4), 255 → 5: 16 + 72 + 24 + 5 = 117.
        assert_eq!(nearest_256(0x7F, 0xD6, 0xFF), 117);
        // ceniza (138, 143, 152): the cube's best is (135, 135, 135), 102, at 9 + 64 + 289 = 362;
        // the gray 148 (232 + 14 = 246) is at 100 + 25 + 16 = 141, nearer.
        assert_eq!(nearest_256(0x8A, 0x8F, 0x98), 246);
        // The cube's exact colors are themselves, the corners included.
        assert_eq!(nearest_256(0, 0, 0), 16);
        assert_eq!(nearest_256(255, 255, 255), 231);
        assert_eq!(nearest_256(95, 135, 175), 16 + 36 + 12 + 3);
        // A gray between two of the ramp's: 13 is 5 from 8 (232) and 5 from 18 (233), a tie that
        // goes to the lower; 14 is nearer 18.
        assert_eq!(nearest_256(13, 13, 13), 232);
        assert_eq!(nearest_256(14, 14, 14), 233);
    }

    // With NO_COLOR not one escape, even where console would paint (styling forced on).
    #[test]
    fn no_color_palette_paints_nothing() {
        let p = palette(ColorMode::None);
        for style in [
            &p.hueso,
            &p.ceniza_clara,
            &p.ceniza,
            &p.borde,
            &p.ambar,
            &p.hielo,
            &p.verde_suave,
            &p.rojo_apagado,
            &p.sello,
        ] {
            let painted = style
                .clone()
                .force_styling(true)
                .apply_to("Clio")
                .to_string();
            assert_eq!(painted, "Clio");
        }
        let ambar = palette(ColorMode::TrueColor).ambar.force_styling(true);
        assert_eq!(
            ambar.apply_to("◆").to_string(),
            "\x1b[38;2;255;177;61m◆\x1b[0m"
        );
        let ambar = palette(ColorMode::Ansi256).ambar.force_styling(true);
        assert_eq!(ambar.apply_to("◆").to_string(), "\x1b[38;5;215m◆\x1b[0m");
    }
}
