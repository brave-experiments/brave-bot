//! The inks the interface draws itself in.
//!
//! Under the `brave` theme, meaning-bearing shades are mixed rather than taken from the sixteen
//! named slots a terminal repaints, and green for finished, red for failed, dim grey for an aside
//! stay named so they read against whatever palette the person chose for their terminal. A named
//! theme chosen with `/theme` paints every role from that table, including the background, so two
//! roles cannot collapse because the terminal remapped a slot.

use ratatui::style::Color;
use serde::Deserialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// A colour as the three channels it is mixed from, for the one place that interpolates between
/// two of them, and for the inks that change with the terminal's background.
pub type Rgb = (u8, u8, u8);

/// Brave orange, which opens the wordmark and draws every note the session makes under `brave`.
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

/// What the session says in its own voice under `brave`: the trust answer, an unavailable
/// confinement, a status report.
///
/// Yellow is the obvious alternative and is spoken for twice over: a call still running, and the
/// margin down every block of content the planner may not read. A note from the interface itself
/// is neither of those, and drawing it in the same ink said it was.
pub const NOTE: Color = Color::Rgb(BRAND.0, BRAND.1, BRAND.2);

/// The name of the default theme: follow the terminal, mix only the inks that have to be a shade.
pub const BRAVE: &str = "brave";

/// Every semantic ink the interface draws itself in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub background: Color,
    pub text: Color,
    pub muted: Color,
    pub ok: Color,
    pub fail: Color,
    pub running: Color,
    pub accent: Color,
    pub note: Color,
    pub primary: Color,
    /// Whether the session frame fills with [`Self::background`] before chrome is drawn.
    pub paints_background: bool,
}

/// A theme on offer: its name and how it looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub name: String,
    pub palette: Palette,
}

impl Theme {
    fn builtin(name: &'static str, palette: Palette) -> Self {
        Self {
            name: name.to_string(),
            palette,
        }
    }
}

static LIGHT: AtomicBool = AtomicBool::new(false);
static SENSED: AtomicBool = AtomicBool::new(false);

fn current() -> &'static Mutex<(String, Palette)> {
    static CURRENT: OnceLock<Mutex<(String, Palette)>> = OnceLock::new();
    CURRENT.get_or_init(|| Mutex::new((BRAVE.to_string(), brave_palette(false))))
}

/// The palette in force right now.
pub fn palette() -> Palette {
    current()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .1
        .clone()
}

/// The name of the theme in force right now.
pub fn name() -> String {
    current()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .0
        .clone()
}

/// Put a theme in force. Does not write to disk.
pub fn apply(theme: &Theme) {
    let mut guard = current()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = (theme.name.clone(), theme.palette.clone());
}

/// Put `brave` back, with primary matched to the background last sensed.
pub fn apply_brave() {
    apply(&Theme::builtin(
        BRAVE,
        brave_palette(LIGHT.load(Ordering::Relaxed)),
    ));
}

fn with_palette<R>(f: impl FnOnce(&Palette) -> R) -> R {
    let guard = current()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&guard.1)
}

pub fn background() -> Color {
    with_palette(|p| p.background)
}

pub fn text() -> Color {
    with_palette(|p| p.text)
}

pub fn muted() -> Color {
    with_palette(|p| p.muted)
}

pub fn ok() -> Color {
    with_palette(|p| p.ok)
}

pub fn fail() -> Color {
    with_palette(|p| p.fail)
}

pub fn running() -> Color {
    with_palette(|p| p.running)
}

pub fn accent() -> Color {
    with_palette(|p| p.accent)
}

pub fn note() -> Color {
    with_palette(|p| p.note)
}

/// Brand primary, given the theme in force.
///
/// Under `brave`, named Cyan is a slot the terminal repaints, and it was the same slot on a light
/// theme and a dark one. These two shades are mixed so each stays distinct against the background
/// it is for. Under a named theme, the theme's own primary is used.
pub fn brand_primary() -> Color {
    with_palette(|p| p.primary)
}

pub fn paints_background() -> bool {
    with_palette(|p| p.paints_background)
}

/// The `brave` palette for a light or dark terminal background.
pub fn brave_palette(light: bool) -> Palette {
    Palette {
        background: Color::Reset,
        text: Color::Reset,
        muted: Color::DarkGray,
        ok: Color::Green,
        fail: Color::Red,
        running: Color::Yellow,
        accent: Color::Magenta,
        note: NOTE,
        primary: brand_primary_on(light),
        paints_background: false,
    }
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
///
/// After sensing, rebuilds the `brave` palette if that is what is in force, so primary matches.
pub fn sense(out: &mut impl Write) {
    if SENSED.swap(true, Ordering::Relaxed) {
        return;
    }
    let light = light_background(out);
    LIGHT.store(light, Ordering::Relaxed);
    let mut guard = current()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.0 == BRAVE {
        guard.1 = brave_palette(light);
    }
}

/// Load the stored theme name, or `brave` when nothing was chosen or the name is unknown.
pub fn restore_saved() {
    let chosen = crate::store::load_theme().unwrap_or_else(|| BRAVE.to_string());
    // `system` was the earlier name for this theme; keep reading it so an already-saved choice
    // does not silently fall back.
    let chosen = if chosen == "system" {
        BRAVE
    } else {
        chosen.as_str()
    };
    if let Some(theme) = find(chosen) {
        apply(&theme);
    } else {
        apply_brave();
    }
}

/// Every theme on offer: built-ins first, then user JSON from `~/.bravebot/themes`.
///
/// `brave` stays at the top; every other name is alphabetical so a growing list stays scannable.
pub fn offered() -> Vec<Theme> {
    let mut themes = builtins();
    for theme in load_user_themes() {
        if let Some(existing) = themes.iter_mut().find(|t| t.name == theme.name) {
            *existing = theme;
        } else {
            themes.push(theme);
        }
    }
    sort_brave_first(&mut themes);
    themes
}

/// Find a theme by name among those on offer.
pub fn find(name: &str) -> Option<Theme> {
    let name = if name == "system" { BRAVE } else { name };
    offered().into_iter().find(|theme| theme.name == name)
}

/// Built-in themes. Hex values are from each scheme's published terminal / palette ports
/// (Catppuccin, Tokyo Night, Gruvbox, Dracula, Nord, Rosé Pine, Kanagawa, Everforest, Ayu,
/// One Dark, Solarized, GitHub Primer, Monokai, Flexoki, Oxocarbon, Cobalt2).
pub fn builtins() -> Vec<Theme> {
    let light = LIGHT.load(Ordering::Relaxed);
    let mut themes = vec![
        Theme::builtin(BRAVE, brave_palette(light)),
        named(
            "catppuccin-mocha",
            (0x1e, 0x1e, 0x2e),
            (0xcd, 0xd6, 0xf4),
            (0x6c, 0x70, 0x86),
            (0xa6, 0xe3, 0xa1),
            (0xf3, 0x8b, 0xa8),
            (0xf9, 0xe2, 0xaf),
            (0xcb, 0xa6, 0xf7),
            (0xfa, 0xb3, 0x87),
            (0x89, 0xb4, 0xfa),
        ),
        named(
            "catppuccin-macchiato",
            (0x24, 0x27, 0x3a),
            (0xca, 0xd3, 0xf5),
            (0x6e, 0x73, 0x8d),
            (0xa6, 0xda, 0x95),
            (0xed, 0x87, 0x96),
            (0xee, 0xd4, 0x9f),
            (0xc6, 0xa0, 0xf6),
            (0xf5, 0xa9, 0x7f),
            (0x8a, 0xad, 0xf4),
        ),
        named(
            "catppuccin-latte",
            (0xef, 0xf1, 0xf5),
            (0x4c, 0x4f, 0x69),
            (0x9c, 0xa0, 0xb0),
            (0x40, 0xa0, 0x2b),
            (0xd2, 0x0f, 0x39),
            (0xdf, 0x8e, 0x1d),
            (0x88, 0x39, 0xef),
            (0xfe, 0x64, 0x0b),
            (0x1e, 0x66, 0xf5),
        ),
        named(
            "tokyonight",
            (0x1a, 0x1b, 0x26),
            (0xa9, 0xb1, 0xd6),
            (0x56, 0x5f, 0x89),
            (0x9e, 0xce, 0x6a),
            (0xf7, 0x76, 0x8e),
            (0xe0, 0xaf, 0x68),
            (0xbb, 0x9a, 0xf7),
            (0xff, 0x9e, 0x64),
            (0x7a, 0xa2, 0xf7),
        ),
        named(
            "tokyonight-storm",
            (0x24, 0x28, 0x3b),
            (0xc0, 0xca, 0xf5),
            (0x56, 0x5f, 0x89),
            (0x9e, 0xce, 0x6a),
            (0xf7, 0x76, 0x8e),
            (0xe0, 0xaf, 0x68),
            (0xbb, 0x9a, 0xf7),
            (0xff, 0x9e, 0x64),
            (0x7a, 0xa2, 0xf7),
        ),
        named(
            "gruvbox-dark",
            (0x28, 0x28, 0x28),
            (0xeb, 0xdb, 0xb2),
            (0x92, 0x83, 0x74),
            (0xb8, 0xbb, 0x26),
            (0xfb, 0x49, 0x34),
            (0xfa, 0xbd, 0x2f),
            (0xd3, 0x86, 0x9b),
            (0xfe, 0x80, 0x19),
            (0x83, 0xa5, 0x98),
        ),
        named(
            "gruvbox-light",
            (0xfb, 0xf1, 0xc7),
            (0x3c, 0x38, 0x36),
            (0x92, 0x83, 0x74),
            (0x79, 0x74, 0x0e),
            (0x9d, 0x00, 0x06),
            (0xb5, 0x76, 0x14),
            (0x8f, 0x3f, 0x71),
            (0xaf, 0x3a, 0x03),
            (0x07, 0x66, 0x78),
        ),
        named(
            "dracula",
            (0x28, 0x2a, 0x36),
            (0xf8, 0xf8, 0xf2),
            (0x62, 0x72, 0xa4),
            (0x50, 0xfa, 0x7b),
            (0xff, 0x55, 0x55),
            (0xf1, 0xfa, 0x8c),
            (0xbd, 0x93, 0xf9),
            (0xff, 0xb8, 0x6c),
            (0x8b, 0xe9, 0xfd),
        ),
        named(
            "nord",
            (0x2e, 0x34, 0x40),
            (0xd8, 0xde, 0xe9),
            (0x4c, 0x56, 0x6a),
            (0xa3, 0xbe, 0x8c),
            (0xbf, 0x61, 0x6a),
            (0xeb, 0xcb, 0x8b),
            (0xb4, 0x8e, 0xad),
            (0xd0, 0x87, 0x70),
            (0x88, 0xc0, 0xd0),
        ),
        named(
            "rose-pine",
            (0x19, 0x17, 0x24),
            (0xe0, 0xde, 0xf4),
            (0x6e, 0x6a, 0x86),
            (0x31, 0x74, 0x8f),
            (0xeb, 0x6f, 0x92),
            (0xf6, 0xc1, 0x77),
            (0xc4, 0xa7, 0xe7),
            (0xeb, 0xbc, 0xba),
            (0x9c, 0xcf, 0xd8),
        ),
        named(
            "kanagawa",
            (0x1f, 0x1f, 0x28),
            (0xdc, 0xd7, 0xba),
            (0x72, 0x71, 0x69),
            (0x98, 0xbb, 0x6c),
            (0xc3, 0x40, 0x43),
            (0xe6, 0xc3, 0x84),
            (0x95, 0x7f, 0xb8),
            (0xff, 0xa0, 0x66),
            (0x7e, 0x9c, 0xd8),
        ),
        named(
            "everforest",
            (0x2d, 0x35, 0x3b),
            (0xd3, 0xc6, 0xaa),
            (0x85, 0x92, 0x89),
            (0xa7, 0xc0, 0x80),
            (0xe6, 0x7e, 0x80),
            (0xdb, 0xbc, 0x7f),
            (0xd6, 0x99, 0xb6),
            (0xe6, 0x98, 0x75),
            (0x7f, 0xbb, 0xb3),
        ),
        named(
            "ayu-dark",
            (0x0b, 0x0e, 0x14),
            (0xbf, 0xbd, 0xb6),
            (0x56, 0x5b, 0x66),
            (0xaa, 0xd9, 0x4c),
            (0xd9, 0x57, 0x57),
            (0xff, 0xb4, 0x54),
            (0xd2, 0xa6, 0xff),
            (0xff, 0x8f, 0x40),
            (0x59, 0xc2, 0xff),
        ),
        named(
            "one-dark",
            (0x28, 0x2c, 0x34),
            (0xab, 0xb2, 0xbf),
            (0x5c, 0x63, 0x70),
            (0x98, 0xc3, 0x79),
            (0xe0, 0x6c, 0x75),
            (0xe5, 0xc0, 0x7b),
            (0xc6, 0x78, 0xdd),
            (0xd1, 0x9a, 0x66),
            (0x61, 0xaf, 0xef),
        ),
        named(
            "solarized-dark",
            (0x00, 0x2b, 0x36),
            (0x83, 0x94, 0x96),
            (0x58, 0x6e, 0x75),
            (0x85, 0x99, 0x00),
            (0xdc, 0x32, 0x2f),
            (0xb5, 0x89, 0x00),
            (0xd3, 0x36, 0x82),
            (0xcb, 0x4b, 0x16),
            (0x26, 0x8b, 0xd2),
        ),
        named(
            "solarized-light",
            (0xfd, 0xf6, 0xe3),
            (0x65, 0x7b, 0x83),
            (0x93, 0xa1, 0xa1),
            (0x85, 0x99, 0x00),
            (0xdc, 0x32, 0x2f),
            (0xb5, 0x89, 0x00),
            (0xd3, 0x36, 0x82),
            (0xcb, 0x4b, 0x16),
            (0x26, 0x8b, 0xd2),
        ),
        named(
            "github-dark",
            (0x0d, 0x11, 0x17),
            (0xe6, 0xed, 0xf3),
            (0x84, 0x8d, 0x97),
            (0x3f, 0xb9, 0x50),
            (0xf8, 0x51, 0x49),
            (0xd2, 0x99, 0x22),
            (0xa3, 0x71, 0xf7),
            (0xdb, 0x6d, 0x28),
            (0x44, 0x93, 0xf8),
        ),
        named(
            "monokai",
            (0x27, 0x28, 0x22),
            (0xf8, 0xf8, 0xf2),
            (0x75, 0x71, 0x5e),
            (0xa6, 0xe2, 0x2e),
            (0xf9, 0x26, 0x72),
            (0xe6, 0xdb, 0x74),
            (0xae, 0x81, 0xff),
            (0xfd, 0x97, 0x1f),
            (0x66, 0xd9, 0xef),
        ),
        named(
            "flexoki-dark",
            (0x10, 0x0f, 0x0f),
            (0xce, 0xcd, 0xc3),
            (0x87, 0x85, 0x80),
            (0x87, 0x9a, 0x39),
            (0xd1, 0x4d, 0x41),
            (0xd0, 0xa2, 0x15),
            (0xa0, 0x2f, 0x6f),
            (0xda, 0x70, 0x2c),
            (0x43, 0x85, 0xbe),
        ),
        named(
            "oxocarbon",
            (0x16, 0x16, 0x16),
            (0xf2, 0xf4, 0xf8),
            (0x52, 0x52, 0x52),
            (0x42, 0xbe, 0x65),
            (0xee, 0x53, 0x96),
            (0xff, 0x7e, 0xb6),
            (0xbe, 0x95, 0xff),
            (0x3d, 0xdb, 0xd9),
            (0x78, 0xa9, 0xff),
        ),
        // Wes Bos Cobalt2: https://github.com/wesbos/cobalt2-vscode
        named(
            "cobalt2",
            (0x19, 0x35, 0x49),
            (0xff, 0xff, 0xff),
            (0x5a, 0x7b, 0x92),
            (0x3a, 0xd9, 0x00),
            (0xff, 0x62, 0x8c),
            (0xff, 0xc6, 0x00),
            (0xff, 0x00, 0x88),
            (0xff, 0x9d, 0x00),
            (0x00, 0x88, 0xff),
        ),
    ];
    sort_brave_first(&mut themes);
    themes
}

/// `brave` first, every other name alphabetical. Used for both the compiled-in set and the list
/// after user JSON is merged in.
fn sort_brave_first(themes: &mut [Theme]) {
    themes.sort_by(
        |a, b| match (a.name.as_str() == BRAVE, b.name.as_str() == BRAVE) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn named(
    name: &'static str,
    background: Rgb,
    text: Rgb,
    muted: Rgb,
    ok: Rgb,
    fail: Rgb,
    running: Rgb,
    accent: Rgb,
    note: Rgb,
    primary: Rgb,
) -> Theme {
    Theme::builtin(
        name,
        Palette {
            background: rgb(background),
            text: rgb(text),
            muted: rgb(muted),
            ok: rgb(ok),
            fail: rgb(fail),
            running: rgb(running),
            accent: rgb(accent),
            note: rgb(note),
            primary: rgb(primary),
            paints_background: true,
        },
    )
}

/// Directory holding user theme JSON files, inside `~/.bravebot`.
pub fn user_themes_directory() -> Option<PathBuf> {
    crate::store::directory().map(|dir| dir.join("themes"))
}

fn load_user_themes() -> Vec<Theme> {
    let Some(dir) = user_themes_directory() else {
        return Vec::new();
    };
    load_user_themes_from(&dir)
}

/// Load themes from a directory. Separate so tests need no home directory.
pub fn load_user_themes_from(dir: &Path) -> Vec<Theme> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut themes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if stem.is_empty() || stem == BRAVE {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(theme) = parse_user_theme(stem, &contents) {
            themes.push(theme);
        }
    }
    themes.sort_by(|a, b| a.name.cmp(&b.name));
    themes
}

#[derive(Debug, Deserialize)]
struct UserThemeFile {
    #[serde(default)]
    defs: std::collections::BTreeMap<String, String>,
    background: Option<String>,
    text: Option<String>,
    muted: Option<String>,
    ok: Option<String>,
    fail: Option<String>,
    running: Option<String>,
    accent: Option<String>,
    note: Option<String>,
    primary: Option<String>,
}

/// Parse one user theme JSON. Missing keys inherit from `brave`. A broken file yields `None`.
pub fn parse_user_theme(name: &str, contents: &str) -> Option<Theme> {
    let file: UserThemeFile = serde_json::from_str(contents).ok()?;
    let base = brave_palette(LIGHT.load(Ordering::Relaxed));
    let resolve = |value: Option<&String>, fallback: Color| -> Option<Color> {
        match value {
            None => Some(fallback),
            Some(raw) => resolve_colour(raw, &file.defs),
        }
    };
    let background = resolve(file.background.as_ref(), base.background)?;
    let text = resolve(file.text.as_ref(), base.text)?;
    let muted = resolve(file.muted.as_ref(), base.muted)?;
    let ok = resolve(file.ok.as_ref(), base.ok)?;
    let fail = resolve(file.fail.as_ref(), base.fail)?;
    let running = resolve(file.running.as_ref(), base.running)?;
    let accent = resolve(file.accent.as_ref(), base.accent)?;
    let note = resolve(file.note.as_ref(), base.note)?;
    let primary = resolve(file.primary.as_ref(), base.primary)?;
    let paints_background = !matches!(background, Color::Reset);
    Some(Theme {
        name: name.to_string(),
        palette: Palette {
            background,
            text,
            muted,
            ok,
            fail,
            running,
            accent,
            note,
            primary,
            paints_background,
        },
    })
}

fn resolve_colour(value: &str, defs: &std::collections::BTreeMap<String, String>) -> Option<Color> {
    let value = value.trim();
    if value == "none" {
        return Some(Color::Reset);
    }
    if let Some(hex) = value.strip_prefix('#') {
        return channel6(hex).map(rgb);
    }
    if let Some(def) = defs.get(value) {
        // One level only: a def that names another def is refused rather than chasing a cycle.
        if def == "none" {
            return Some(Color::Reset);
        }
        return def.strip_prefix('#').and_then(channel6).map(rgb);
    }
    None
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
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    use std::time::{Duration, Instant};

    write!(out, "\x1b]11;?\x07").ok()?;
    out.flush().ok()?;

    let stdin = std::io::stdin();
    let deadline = Instant::now() + Duration::from_millis(80);
    let mut buf = Vec::new();
    let mut tmp = [0u8; 64];

    while Instant::now() < deadline {
        let remain = deadline.saturating_duration_since(Instant::now());
        let timeout = Timespec::try_from(remain).unwrap_or(Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        });
        let mut fds = [PollFd::new(&stdin, PollFlags::IN)];
        // poll/read on the tty itself, because crossterm drops OSC replies and a blocking
        // read would hang on a terminal that never answers.
        if poll(&mut fds, Some(&timeout)).ok()? == 0 {
            break;
        }
        let n = rustix::io::read(&stdin, tmp.as_mut_slice()).ok()?;
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
        apply_brave();
        assert!(
            matches!(note(), Color::Rgb(..)),
            "a note took a named colour, which the terminal chooses: {:?}",
            note()
        );
        assert!(
            matches!(NOTE, Color::Rgb(..)),
            "a note took a named colour, which the terminal chooses: {NOTE:?}"
        );
    }

    /// Cyan is the slot that used to carry brand primary, and terminals disagree about what it
    /// looks like. Pinning the kind keeps a named colour from returning under a different spelling.
    #[test]
    fn brand_primary_is_a_shade_and_not_a_slot_a_terminal_repaints() {
        apply_brave();
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

    /// Under `brave`, finished stays a named slot so it reads against the person's terminal.
    #[test]
    fn brave_keeps_named_slots_for_the_terminals_own_meanings() {
        apply_brave();
        assert_eq!(ok(), Color::Green);
        assert_eq!(fail(), Color::Red);
        assert_eq!(running(), Color::Yellow);
        assert_eq!(muted(), Color::DarkGray);
        assert!(!paints_background());
    }

    /// A named theme mixes every role, including the background, so a remapped ANSI slot cannot
    /// collapse finished into failed.
    #[test]
    fn a_named_theme_paints_its_own_background_and_inks() {
        let theme = find("nord").expect("nord is built in");
        apply(&theme);
        assert!(paints_background());
        assert!(matches!(background(), Color::Rgb(..)));
        assert!(matches!(ok(), Color::Rgb(..)));
        assert_ne!(ok(), Color::Green);
        apply_brave();
    }

    #[test]
    fn the_old_system_name_still_finds_brave() {
        let theme = find("system").expect("alias");
        assert_eq!(theme.name, BRAVE);
        assert!(!theme.palette.paints_background);
    }

    #[test]
    fn none_in_json_inherits_the_terminal_default() {
        let theme = parse_user_theme(
            "plain",
            "{\"background\": \"none\", \"text\": \"none\", \"ok\": \"#00ff00\"}",
        )
        .expect("valid");
        assert_eq!(theme.palette.background, Color::Reset);
        assert_eq!(theme.palette.text, Color::Reset);
        assert_eq!(theme.palette.ok, Color::Rgb(0, 255, 0));
        assert!(!theme.palette.paints_background);
        // Missing keys inherit system.
        assert_eq!(theme.palette.fail, Color::Red);
    }

    #[test]
    fn defs_are_resolved_once() {
        let theme = parse_user_theme(
            "mine",
            concat!(
                "{",
                "\"defs\": { \"base\": \"#112233\", \"ink\": \"#aabbcc\" },",
                "\"background\": \"base\",",
                "\"text\": \"ink\"",
                "}"
            ),
        )
        .expect("valid");
        assert_eq!(theme.palette.background, Color::Rgb(0x11, 0x22, 0x33));
        assert_eq!(theme.palette.text, Color::Rgb(0xaa, 0xbb, 0xcc));
    }

    #[test]
    fn a_broken_json_file_is_not_a_theme() {
        assert!(parse_user_theme("x", "{ not json").is_none());
        assert!(parse_user_theme("x", "{\"ok\": \"not-a-colour\"}").is_none());
    }

    #[test]
    fn user_themes_come_from_a_directory_of_json_files() {
        let root = std::env::temp_dir().join(format!("bravebot-themes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("dir");
        std::fs::write(root.join("mine.json"), "{\"primary\": \"#123456\"}").expect("write");
        std::fs::write(root.join("broken.json"), "{").expect("write");
        std::fs::write(root.join("readme.txt"), "ignore").expect("write");

        let themes = load_user_themes_from(&root);
        assert_eq!(themes.len(), 1);
        assert_eq!(themes[0].name, "mine");
        assert_eq!(themes[0].palette.primary, Color::Rgb(0x12, 0x34, 0x56));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Twenty-one named schemes plus `brave`, with `brave` first and the rest alphabetical.
    #[test]
    fn the_built_in_set_is_brave_and_twenty_one_named() {
        let names: Vec<_> = builtins().into_iter().map(|t| t.name).collect();
        assert_eq!(names[0], BRAVE);
        assert_eq!(names.len(), 22);
        assert!(names.contains(&"nord".to_string()));
        assert!(names.contains(&"catppuccin-mocha".to_string()));
        assert!(names.contains(&"cobalt2".to_string()));
        let rest = &names[1..];
        let mut sorted = rest.to_vec();
        sorted.sort();
        assert_eq!(rest, sorted.as_slice(), "named themes are not alphabetical");
    }
}
