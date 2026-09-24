//! The colour of the floor and its quiet pattern: the few tones and styles a person can choose,
//! and the ground that paints them.
//!
//! Every tone comes from the theme's own colours, so it belongs to whichever theme is in force and
//! changes with it: a fixed blue would fight a warm theme, a tone made of the theme's canvas does
//! not. Only the floor takes it. Windows, the dock and the surfaces over them keep the theme's
//! tones, and a plain floor left at the theme's canvas is not painted at all, so a desktop nobody
//! changed sends not one byte more over SSH.
//!
//! The pattern is made of cell colours and one small mark, never of lines: a gradient is one
//! background colour a row, and the dots sit on the corners of the icon grid, in the empty row
//! under each cell, so they never touch an icon or its name.

use qframe::color::{ColorDepth, Rgb};
use qframe::geometry::{Rect, Size};
use qframe::icons::GlyphMode;
use qframe::style::CellStyle;
use qframe::theme::Theme;
use qframe::widget::{MeasureCx, PaintCx, View, Widget};

use crate::desktop::grid::{CELL_HEIGHT, CELL_WIDTH};

/// How far the deep tone goes from the canvas towards black.
const DEEP: f32 = 0.45;
/// How far the mist tone goes from the canvas towards the theme's text colour.
const MIST: f32 = 0.07;
/// How much of the theme's accent the accent tone mixes into the canvas.
const ACCENT: f32 = 0.14;
/// The most of the theme's accent the gradient mixes into the bottom row of the floor. Less is
/// used when the icon names would read worse than [`NAMES_READ`] on it.
const GLOW: f32 = 0.12;
/// How far a dot goes from the ground under it towards the theme's text colour: seen when looked
/// for, not while reading.
const DOT: f32 = 0.16;
/// The contrast the icon names (`dim`) keep against every row of the floor.
pub const NAMES_READ: f64 = 4.5;
/// The contrast the icons themselves (`text`) keep against every row of the floor.
pub const GLYPHS_READ: f64 = 7.0;
/// The dot in full glyphs: a middle dot, one cell wide everywhere.
const DOT_GLYPH: &str = "\u{b7}";
/// The dot where only ASCII is drawn.
const DOT_ASCII: &str = ".";

/// The colour of the floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FloorColor {
    /// The theme's own canvas.
    #[default]
    Theme,
    /// The canvas taken towards black: the icons stand out a little more.
    Deep,
    /// The canvas taken a little towards the text colour: a soft grey of the theme.
    Mist,
    /// A little of the theme's accent in the canvas.
    Accent,
}

impl FloorColor {
    /// Every tone, in the order the chooser lists them.
    pub const ALL: [Self; 4] = [Self::Theme, Self::Deep, Self::Mist, Self::Accent];

    /// The name written in the settings file, also the key its label is looked up by.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Theme => "theme",
            Self::Deep => "deep",
            Self::Mist => "mist",
            Self::Accent => "accent",
        }
    }

    /// The tone a name means, if it is one of ours.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tone| tone.name() == name)
    }

    /// The colour this tone comes to over a theme whose canvas, text and accent are these.
    #[must_use]
    pub fn mixed(self, canvas: Rgb, text: Rgb, accent: Rgb) -> Rgb {
        match self {
            Self::Theme => canvas,
            Self::Deep => canvas.mix(Rgb::new(0, 0, 0), DEEP),
            Self::Mist => canvas.mix(text, MIST),
            Self::Accent => canvas.mix(accent, ACCENT),
        }
    }

    /// The colour this tone comes to in `theme`; `None` for a theme without a canvas.
    #[must_use]
    pub fn in_theme(self, theme: &Theme) -> Option<Rgb> {
        let canvas = theme.color("canvas")?;
        let text = theme.color("text").unwrap_or(canvas);
        let accent = theme.color("accent").unwrap_or(text);
        Some(self.mixed(canvas, text, accent))
    }
}

/// The pattern laid over the floor's colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FloorStyle {
    /// One colour, as the floor has always been.
    #[default]
    Plain,
    /// The tone at the top, warming row by row towards a little of the accent at the bottom.
    Gradient,
    /// A faint dot on every corner of the icon grid.
    Dots,
    /// The gradient with the dots over it.
    GradientDots,
}

impl FloorStyle {
    /// Every style, in the order the chooser lists them.
    pub const ALL: [Self; 4] = [Self::Plain, Self::Gradient, Self::Dots, Self::GradientDots];

    /// The name written in the settings file, also the key its label is looked up by.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Gradient => "gradient",
            Self::Dots => "dots",
            Self::GradientDots => "gradient-dots",
        }
    }

    /// The style a name means, if it is one of ours.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|style| style.name() == name)
    }

    /// Whether the floor warms from top to bottom.
    #[must_use]
    pub fn gradient(self) -> bool {
        matches!(self, Self::Gradient | Self::GradientDots)
    }

    /// Whether the corners of the icon grid carry a dot.
    #[must_use]
    pub fn dots(self) -> bool {
        matches!(self, Self::Dots | Self::GradientDots)
    }
}

/// The theme colours the floor is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// The screen's ground.
    pub canvas: Rgb,
    /// The icons, and the colour a dot leans towards.
    pub text: Rgb,
    /// The icon names.
    pub dim: Rgb,
    /// The one accent, the colour the gradient warms towards.
    pub accent: Rgb,
}

impl Palette {
    /// The colours of `theme`; `None` for a theme without a canvas.
    #[must_use]
    pub fn of(theme: &Theme) -> Option<Self> {
        let canvas = theme.color("canvas")?;
        let text = theme.color("text").unwrap_or(canvas);
        let dim = theme.color("dim").unwrap_or(text);
        let accent = theme.color("accent").unwrap_or(text);
        Some(Self { canvas, text, dim, accent })
    }
}

/// The colours of a floor of a given tone and style, row by row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shades {
    top: Rgb,
    bottom: Rgb,
    text: Rgb,
    dots: bool,
}

impl Shades {
    /// The floor `tone` and `style` make over `palette`, drawn at `depth` colours.
    ///
    /// Sixteen colours draw no gradient and no dots: the nearest lit colour there is bright black,
    /// which would turn half the floor grey at once, and the quietest dot that still shows is
    /// several times louder than in full colour, a grid of marks rather than a texture.
    #[must_use]
    pub fn new(tone: FloorColor, style: FloorStyle, palette: Palette, depth: ColorDepth) -> Self {
        let rich = depth != ColorDepth::Ansi16;
        let top = tone.mixed(palette.canvas, palette.text, palette.accent);
        let bottom = if style.gradient() && rich { top.mix(palette.accent, glow(top, palette)) } else { top };
        Self { top, bottom, text: palette.text, dots: style.dots() && rich }
    }

    /// The ground of row `row` of a floor `rows` high: the top row is the tone, the bottom row the
    /// tone with the glow in it, and the rows between them go in even steps.
    #[must_use]
    pub fn row(&self, row: u16, rows: u16) -> Rgb {
        if rows <= 1 || self.top == self.bottom {
            return self.top;
        }
        let share = f32::from(row.min(rows - 1)) / f32::from(rows - 1);
        self.top.mix(self.bottom, share)
    }

    /// The colour of a dot on `ground`.
    #[must_use]
    pub fn dot(&self, ground: Rgb) -> Rgb {
        ground.mix(self.text, DOT)
    }

    /// Whether these shades are drawn with dots.
    #[must_use]
    pub fn dots(&self) -> bool {
        self.dots
    }

    /// Whether the floor is one colour from top to bottom.
    #[must_use]
    pub fn flat(&self) -> bool {
        self.top == self.bottom
    }
}

/// How much of the accent the bottom row takes over `top`: [`GLOW`], or less where the icon names
/// or the icons would read worse than the floor promises, down to none. Worked out rather than
/// hoped for, so a theme a person wrote keeps its names readable too.
fn glow(top: Rgb, palette: Palette) -> f32 {
    let steps = 24_u8;
    (0..=steps)
        .rev()
        .map(|step| GLOW * f32::from(step) / f32::from(steps))
        .find(|amount| {
            let ground = top.mix(palette.accent, *amount);
            palette.dim.contrast_ratio(ground) >= NAMES_READ && palette.text.contrast_ratio(ground) >= GLYPHS_READ
        })
        .unwrap_or(0.0)
}

/// Whether the cell `x`, `y` of the floor, counted from its top left, carries a dot: the corner
/// of an icon cell, in the empty row under the icon and its name.
#[must_use]
pub fn is_dot(x: u16, y: u16) -> bool {
    x.is_multiple_of(CELL_WIDTH) && y % CELL_HEIGHT == CELL_HEIGHT - 1
}

/// The ground of the floor in a tone or a style that is not the theme's plain canvas: a layer
/// that only paints, under the icons, and takes neither the pointer nor the keys.
struct Ground(FloorColor, FloorStyle);

impl<Msg: 'static> Widget<Msg> for Ground {
    fn measure(&self, _cx: &mut MeasureCx<'_>, available: Size) -> Size {
        available
    }

    fn paint(&self, cx: &mut PaintCx<'_>, area: Rect) {
        let palette = Palette {
            canvas: cx.color("canvas"),
            text: cx.color("text"),
            dim: cx.color("dim"),
            accent: cx.color("accent"),
        };
        let shades = Shades::new(self.0, self.1, palette, cx.env().depth());
        if !shades.dots() && shades.flat() {
            cx.clear(area, shades.row(0, area.height));
            return;
        }
        let mark = if cx.env().icons().mode() == GlyphMode::Ascii { DOT_ASCII } else { DOT_GLYPH };
        let blank = " ".repeat(usize::from(area.width));
        for row in 0..area.height {
            let y = area.y + i32::from(row);
            let ground = shades.row(row, area.height);
            if !shades.dots() {
                cx.clear(Rect::new(area.x, y, area.width, 1), ground);
                continue;
            }
            // The blanks of a band of the grid carry the colour of the dots of that band, so a
            // dot is only its mark where a blank would be: no colour is changed for it and back.
            let band = row - row % CELL_HEIGHT + CELL_HEIGHT - 1;
            let dot = shades.dot(shades.row(band, area.height));
            let style = CellStyle::fg(dot).on(ground);
            cx.text(area.x, y, &blank, style, area.width);
            if row % CELL_HEIGHT != CELL_HEIGHT - 1 {
                continue;
            }
            for column in (0..area.width).filter(|column| is_dot(*column, row)) {
                cx.text(area.x + i32::from(column), y, mark, style, 1);
            }
        }
    }
}

/// Lays the floor's ground in `tone` and `style` into the stack `ui` is building, under what
/// comes after it. The theme's own tone, plain, lays nothing: the screen is that colour already.
pub fn ground<Msg: 'static>(tone: FloorColor, style: FloorStyle, ui: &mut View<'_, Msg>) {
    if tone != FloorColor::Theme || style != FloorStyle::Plain {
        ui.add(Ground(tone, style)).fill();
    }
}
