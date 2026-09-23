//! The colour of the floor: the few tones a person can choose, and the ground that paints one.
//!
//! Every tone comes from the theme's own colours, so it belongs to whichever theme is in force and
//! changes with it: a fixed blue would fight a warm theme, a tone made of the theme's canvas does
//! not. Only the floor takes it. Windows, the dock and the surfaces over them keep the theme's
//! tones, and a floor left at the theme's canvas is not painted at all, so a desktop nobody changed
//! sends not one byte more over SSH.

use qframe::color::Rgb;
use qframe::geometry::{Rect, Size};
use qframe::theme::Theme;
use qframe::widget::{MeasureCx, PaintCx, View, Widget};

/// How far the deep tone goes from the canvas towards black.
const DEEP: f32 = 0.45;
/// How far the mist tone goes from the canvas towards the theme's text colour.
const MIST: f32 = 0.07;
/// How much of the theme's accent the accent tone mixes into the canvas.
const ACCENT: f32 = 0.14;

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

/// The ground of the floor in a tone that is not the theme's: a layer that only paints, under
/// the icons, and takes neither the pointer nor the keys.
struct Ground(FloorColor);

impl<Msg: 'static> Widget<Msg> for Ground {
    fn measure(&self, _cx: &mut MeasureCx<'_>, available: Size) -> Size {
        available
    }

    fn paint(&self, cx: &mut PaintCx<'_>, area: Rect) {
        let tone = self.0.mixed(cx.color("canvas"), cx.color("text"), cx.color("accent"));
        cx.clear(area, tone);
    }
}

/// Lays the floor's ground in `tone` into the stack `ui` is building, under what comes after it.
/// The theme's own tone lays nothing: the screen is that colour already.
pub fn ground<Msg: 'static>(tone: FloorColor, ui: &mut View<'_, Msg>) {
    if tone != FloorColor::Theme {
        ui.add(Ground(tone)).fill();
    }
}
