//! The applications the desktop and the launcher show.
//!
//! An application is an entry: a small TOML file that names a program and how its window opens.
//! Entries come from four places, highest priority first: the user's entry folder, the system's
//! entry folders, the entries built into qdesk, and the system's `.desktop` files of terminal
//! programs. The same id in two places keeps the higher one; `hidden = true` hides an id from
//! every place below.
//!
//! Loading never panics and never stops at a broken file. Every problem becomes a [`Diagnostic`]
//! with the file, line and column and a [`DiagnosticKind`]; the broken entry is left out and the
//! others load. Diagnostics carry no sentences: the interface turns their kinds into text in the
//! user's language.
//!
//! Nothing here draws or watches. [`Environment::folders`] names the folders to watch, [`load`]
//! reads them again after a change, and [`Catalog`] groups and searches the result.

mod builtin;
mod catalog;
#[cfg(test)]
mod crash_search;
mod desktop_file;
mod diagnostic;
mod entry;
mod program;
mod sources;

pub use catalog::{Catalog, Group, Hit, Rank};
pub use desktop_file::{DesktopFile, parse_desktop_file};
pub use diagnostic::{Diagnostic, DiagnosticKind, Expected, Position};
pub use entry::{Declared, Entry, Install, Launch, Screen, Source, WindowPrefs, parse_entry};
pub use program::{find_program, is_executable};
pub use sources::{Environment, Folders, Loaded, load};

/// Where an application sits in the launcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    /// System tools: monitors, settings, the terminal.
    System,
    /// Programming tools and editors.
    Development,
    /// File managers and file tools.
    Files,
    /// Network tools, mail, chat.
    Network,
    /// Documents, notes, calendars.
    Office,
    /// Music, video, pictures.
    Media,
    /// The Quvyta ecosystem's own applications.
    Quvyta,
    /// Everything else.
    Other,
}

impl Category {
    /// Every category, in the order the launcher lists them.
    pub const ALL: [Self; 8] = [
        Self::System,
        Self::Development,
        Self::Files,
        Self::Network,
        Self::Office,
        Self::Media,
        Self::Quvyta,
        Self::Other,
    ];

    /// The name entries use for the category, also the key its label is looked up by.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Development => "development",
            Self::Files => "files",
            Self::Network => "network",
            Self::Office => "office",
            Self::Media => "media",
            Self::Quvyta => "quvyta",
            Self::Other => "other",
        }
    }

    /// The category an entry names, if it is one of ours.
    ///
    /// `family` is what the Quvyta category was called before it took the ecosystem's name;
    /// entries written then still land in it.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        if name == "family" {
            return Some(Self::Quvyta);
        }
        Self::ALL.into_iter().find(|category| category.name() == name)
    }
}

/// A text with translations: a default and the same text in other languages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Localized {
    /// The text when no translation fits: English when the entry gives it.
    pub default: String,
    /// Translations as `(language code, text)`, in the order the file gives them.
    pub translations: Vec<(String, String)>,
}

impl Localized {
    /// A text without translations.
    #[must_use]
    pub fn plain(text: impl Into<String>) -> Self {
        Self { default: text.into(), translations: Vec::new() }
    }

    /// The text in `language`, a code such as `tr`, `pt-BR` or `pt_BR.UTF-8`.
    ///
    /// The exact code is tried first, then its language alone (`pt` for `pt-BR`), then English,
    /// then the default.
    #[must_use]
    pub fn get(&self, language: &str) -> &str {
        let wanted = normalize_code(language);
        let wanted_language = wanted.split('-').next().unwrap_or_default();
        let found = |matches: &dyn Fn(&str) -> bool| {
            self.translations.iter().find(|(code, _)| matches(&normalize_code(code))).map(|(_, text)| text.as_str())
        };
        found(&|code| code == wanted)
            .or_else(|| found(&|code| code.split('-').next() == Some(wanted_language)))
            .or_else(|| found(&|code| code == "en"))
            .unwrap_or(&self.default)
    }

    /// Every form of the text: the default and each translation.
    pub fn all(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.default.as_str()).chain(self.translations.iter().map(|(_, text)| text.as_str()))
    }
}

/// A language code in one spelling: lower case, `-` between parts, without an encoding or a
/// modifier, so `pt_BR.UTF-8`, `pt-br` and `pt_BR@euro` all read `pt-br`.
fn normalize_code(code: &str) -> String {
    let bare = code.split(['.', '@']).next().unwrap_or_default();
    bare.to_lowercase().replace('_', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Localized {
        Localized {
            default: "Settings".to_owned(),
            translations: vec![("en".to_owned(), "Settings".to_owned()), ("tr".to_owned(), "Ayarlar".to_owned())],
        }
    }

    #[test]
    fn category_names_round_trip() {
        for category in Category::ALL {
            assert_eq!(Category::from_name(category.name()), Some(category));
        }
        assert_eq!(Category::from_name("games"), None);
        assert_eq!(Category::from_name("System"), None);
    }

    #[test]
    fn localized_text_picks_the_language() {
        assert_eq!(names().get("tr"), "Ayarlar");
        assert_eq!(names().get("tr_TR.UTF-8"), "Ayarlar");
        assert_eq!(names().get("TR"), "Ayarlar");
    }

    #[test]
    fn localized_text_falls_back_to_english_then_default() {
        assert_eq!(names().get("de"), "Settings");
        let no_english =
            Localized { default: "Réglages".to_owned(), translations: vec![("fr".into(), "Réglages".into())] };
        assert_eq!(no_english.get("de"), "Réglages");
        assert_eq!(Localized::plain("htop").get("tr"), "htop");
    }

    #[test]
    fn localized_text_matches_the_language_of_a_regional_code() {
        let text = Localized {
            default: "Color".to_owned(),
            translations: vec![("pt_BR".into(), "Cor".into()), ("en_GB".into(), "Colour".into())],
        };
        assert_eq!(text.get("pt-BR"), "Cor");
        assert_eq!(text.get("pt"), "Cor");
        assert_eq!(text.get("en_GB"), "Colour");
        assert_eq!(text.get("en_US"), "Colour");
        assert_eq!(text.get("de"), "Color");
    }
}
