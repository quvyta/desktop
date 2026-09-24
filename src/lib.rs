//! qdesk: a desktop inside the terminal, made first for servers reached over SSH.

pub mod app;
pub mod apps;
pub mod cli;
pub mod desktop;
pub mod dock;
pub mod files;
pub mod gadgets;
pub mod inbox;
pub mod launcher;
pub mod notice;
pub mod power;
pub mod session;
pub mod settings;
pub mod status;
pub mod wm;

pub use app::run;

/// The application's own key bindings, compiled in, layered over the framework's. The file name
/// only labels diagnostics.
#[must_use]
pub fn keymap() -> (&'static str, &'static str) {
    ("qdesk.toml", include_str!("../keymaps/qdesk.toml"))
}

/// The application's own locale files, compiled in so an installed program carries its text
/// with it. Each is `(file name, contents)`, ready for the runtime or an [`qframe::i18n::I18n`].
/// English comes first: it is the language every other file is checked against.
#[must_use]
pub fn locales() -> &'static [(&'static str, &'static str)] {
    &[
        ("en.toml", include_str!("../locales/en.toml")),
        ("tr.toml", include_str!("../locales/tr.toml")),
        ("de.toml", include_str!("../locales/de.toml")),
        ("es.toml", include_str!("../locales/es.toml")),
        ("fr.toml", include_str!("../locales/fr.toml")),
        ("ja.toml", include_str!("../locales/ja.toml")),
        ("pt-BR.toml", include_str!("../locales/pt-BR.toml")),
        ("ru.toml", include_str!("../locales/ru.toml")),
        ("zh-Hans.toml", include_str!("../locales/zh-Hans.toml")),
    ]
}

#[cfg(test)]
mod languages;
