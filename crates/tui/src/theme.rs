//! The inks the interface draws itself in.
//!
//! Only the ones that have to be a particular shade, and only because more than one module draws
//! them: a second literal of the same colour is how two parts of one interface come to disagree
//! about it. Where the meaning is the terminal's own the named colours are still used in place,
//! green for finished, red for failed, dim grey for an aside, since those are read against
//! whatever palette the user chose rather than against each other.

use ratatui::style::Color;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

/// A colour as the three channels it is mixed from, for the one place that interpolates between
/// two of them, and for the inks that change with the terminal's background.
pub type Rgb = (u8, u8, u8);

/// Brave orange, which opens the wordmark and draws every note the session makes.
///
/// A literal because the sixteen named colours have nothing near it.
pub const BRAND: Rgb = (255, 86, 1);

/// The deeper orange the wordmark fades into by the end of its branded half.
pub const BRAND_DEEP: Rgb = (255, 64, 0);

/// Brand primary on a dark background: the line being typed, the echo of it, and the chrome
/// that belongs to the person at the keyboard.
pub const BRAND_PRIMARY_DARK: Rgb = (0x76, 0x86, 0xEC);

/// Brand primary on a light background. The dark-background shade washes out there, so this
/// one is deeper rather than the same colour hoped to work on both.
pub const BRAND_PRIMARY_LIGHT: Rgb = (0x43, 0x4F, 0xCF);

/// What the session says in its own voice: the trust answer, an unavailable confinement, a status
/// report.
///
/// Yellow is the obvious alternative and is spoken for twice over: a call still running, and the
/// margin down every block of content the planner may not read. A note from the interface itself
/// is neither of those, and drawing it in the same ink said it was.
pub const NOTE: Color = Color::Rgb(BRAND.0, BRAND.1, BRAND.2);

static LIGHT: AtomicBool = AtomicBool::new(false);
static SENSED: AtomicBool = AtomicBool::new(false);

/// Brand primary, given the background last sensed.
///
/// Named Cyan is a slot the terminal repaints, and it was the same slot on a light theme and a
/// dark one. These two shades are mixed so each stays distinct against the background it is for.
pub fn brand_primary() -> Color {
    brand_primary_on(LIGHT.load(Ordering::Relaxed))
}

fn brand_primary_on(light: bool) -> Color {
    rgb(if light {
        BRAND_PRIMARY_LIGHT
    } else {
        BRAND_PRIMARY_DARK
    })
}

/// Read whether the terminal is light, once, before anything is drawn in brand primary.
///
/// Later handovers of the tty (an editor, then back) must not query again: a round trip on every
/// return would delay the first frame after an edit, and the background does not change for it.
pub fn sense(out: &mut impl Write) {
    if SENSED.swap(true, Ordering::Relaxed) {
        return;
    }
    LIGHT.store(light_background(out), Ordering::Relaxed);
}

fn rgb(channels: Rgb) -> Color {
    Color::Rgb(channels.0, channels.1, channels.2)
}

fn light_background(out: &mut impl Write) -> bool {
    if let Some(light) = colorfgbg() {
        return light;
    }
    #[cfg(unix)]
    {
        if let Some(light) = query_osc11(out) {
            return light;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = out;
    }
    false
}

/// `COLORFGBG` is `foreground;background`, each an ANSI colour index. 7 and 15 are white.
fn colorfgbg() -> Option<bool> {
    light_from_colorfgbg(&std::env::var("COLORFGBG").ok()?)
}

fn light_from_colorfgbg(value: &str) -> Option<bool> {
    let bg = value.rsplit(';').next()?.split(':').next()?;
    let index: u8 = bg.parse().ok()?;
    Some(index == 7 || index == 15)
}

/// Rec. 709 luma. Integer so a threshold is a comparison and not a float that two call sites
/// could round differently.
fn light_from_rgb((r, g, b): Rgb) -> bool {
    2126u32 * u32::from(r) + 7152 * u32::from(g) + 722 * u32::from(b) > 1_270_000
}

#[cfg(unix)]
fn query_osc11(out: &mut impl Write) -> Option<bool> {
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};

    write!(out, "\x1b]11;?\x07").ok()?;
    out.flush().ok()?;

    let fd = std::io::stdin().as_raw_fd();
    let deadline = Instant::now() + Duration::from_millis(80);
    let mut buf = Vec::new();
    let mut tmp = [0u8; 64];

    while Instant::now() < deadline {
        let remain = deadline.saturating_duration_since(Instant::now());
        let ms = i32::try_from(remain.as_millis()).unwrap_or(0);
        let mut fds = [libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        }];
        // poll/read on the tty itself, because crossterm drops OSC replies and a blocking
        // read would hang on a terminal that never answers.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 1, ms) };
        if n <= 0 {
            break;
        }
        let got = unsafe { libc::read(fd, tmp.as_mut_ptr().cast(), tmp.len()) };
        let Ok(n) = usize::try_from(got) else {
            break;
        };
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if osc_complete(&buf) {
            return parse_osc11(&buf).map(light_from_rgb);
        }
        if buf.len() > 128 {
            break;
        }
    }
    None
}

fn osc_complete(buf: &[u8]) -> bool {
    buf.contains(&0x07) || buf.windows(2).any(|w| w == [0x1b, b'\\'])
}

fn parse_osc11(buf: &[u8]) -> Option<Rgb> {
    let text = std::str::from_utf8(buf).ok()?;
    let rest = text.split("11;").nth(1)?;
    let rest = rest.trim_end_matches(['\u{7}', '\u{1b}', '\\', '\r', '\n']);
    if let Some(hex) = rest.strip_prefix('#') {
        channel6(hex)
    } else if let Some(spec) = rest.strip_prefix("rgb:") {
        channels_slash(spec)
    } else if let Some(spec) = rest.strip_prefix("rgba:") {
        channels_slash(spec)
    } else {
        None
    }
}

fn channel6(hex: &str) -> Option<Rgb> {
    let hex = hex.get(..6)?;
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some((
        u8::try_from(n >> 16).ok()?,
        u8::try_from((n >> 8) & 0xff).ok()?,
        u8::try_from(n & 0xff).ok()?,
    ))
}

fn channels_slash(spec: &str) -> Option<Rgb> {
    let mut parts = spec.split('/');
    let r = channel(parts.next()?)?;
    let g = channel(parts.next()?)?;
    let b = channel(parts.next()?)?;
    Some((r, g, b))
}

fn channel(hex: &str) -> Option<u8> {
    let v = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        1 => u8::try_from(v * 17).ok(),
        2 => u8::try_from(v).ok(),
        3 => u8::try_from(v >> 4).ok(),
        4 => u8::try_from(v >> 8).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shade here is mixed rather than named, which is the whole reason the module exists.
    /// One of the sixteen names is a slot the terminal repaints: yellow is the most likely
    /// substitution for a note, and it already means a call still running and the margin down
    /// every block of content the planner may not read.
    ///
    /// Pins the kind and not the value, so the shade stays somebody's to choose.
    #[test]
    fn a_note_is_a_shade_and_not_a_slot_a_terminal_repaints() {
        assert!(
            matches!(NOTE, Color::Rgb(..)),
            "a note took a named colour, which the terminal chooses: {NOTE:?}"
        );
    }

    /// Cyan is the slot that used to carry brand primary, and terminals disagree about what it
    /// looks like. Pinning the kind keeps a named colour from returning under a different spelling.
    #[test]
    fn brand_primary_is_a_shade_and_not_a_slot_a_terminal_repaints() {
        assert!(
            matches!(brand_primary(), Color::Rgb(..)),
            "brand primary took a named colour, which the terminal chooses: {:?}",
            brand_primary()
        );
    }

    #[test]
    fn a_dark_background_takes_the_brighter_brand_primary() {
        assert_eq!(brand_primary_on(false), Color::Rgb(0x76, 0x86, 0xEC));
    }

    #[test]
    fn a_light_background_takes_the_deeper_brand_primary() {
        assert_eq!(brand_primary_on(true), Color::Rgb(0x43, 0x4F, 0xCF));
    }

    #[test]
    fn colorfgbg_with_a_black_background_is_dark() {
        assert_eq!(light_from_colorfgbg("15;0"), Some(false));
        assert_eq!(light_from_colorfgbg("7;8"), Some(false));
    }

    #[test]
    fn colorfgbg_with_a_white_background_is_light() {
        assert_eq!(light_from_colorfgbg("0;15"), Some(true));
        assert_eq!(light_from_colorfgbg("0;7"), Some(true));
    }

    #[test]
    fn an_osc_reply_with_a_pale_background_is_light() {
        assert!(light_from_rgb(
            parse_osc11(b"\x1b]11;rgb:ffff/ffff/ffff\x07").expect("pale")
        ));
        assert!(light_from_rgb(
            parse_osc11(b"\x1b]11;#f8f8f8\x07").expect("hash")
        ));
    }

    #[test]
    fn an_osc_reply_with_a_dark_background_is_dark() {
        assert!(!light_from_rgb(
            parse_osc11(b"\x1b]11;rgb:1e1e/1e1e/1e1e\x07").expect("dark")
        ));
    }
}
