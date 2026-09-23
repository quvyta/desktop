//! The launcher: the search, the shelves on its left and the applications on its right.
//!
//! It is a surface that rises from the left above the dock. It does not dim the screen and does
//! not take it over: the desktop stays where it is behind it, and a press outside closes it.
//!
//! The launcher keeps nothing of its own besides what is on it now — the query, the shelf and the
//! card the person is on. What lasts — the recents and the icons of the floor — belongs to the
//! desktop's own file, [`crate::desktop::Desktop`].

use std::rc::Rc;

use qframe::icons::Icons;
use qframe::prelude::*;
use qframe::widgets::{
    CardGrid, ContextItem, ContextMenu, EmptyState, IconButton, Menu, MenuGroup, MenuItem, Panel, TextInput,
};

use crate::apps::{Catalog, Category, Entry, Launch};
use crate::desktop::{Desktop, glyph_of};

/// The id of the search field, for putting the keys in it the moment the launcher opens.
pub const SEARCH: &str = "launcher-search";

/// The widest the launcher grows, in columns: wider than this and a card grid would spread the
/// applications over a whole server screen instead of gathering them in a corner.
pub const MAX_WIDTH: u16 = 72;

/// The tallest the launcher grows, in rows.
pub const MAX_HEIGHT: u16 = 16;

/// Columns the shelves take beside the applications.
const SHELF_WIDTH: u16 = 16;

/// Which applications the launcher is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shelf {
    /// The applications opened last.
    Recents,
    /// Everything that is installed.
    All,
    /// One category.
    Category(Category),
    /// The entries whose program is not on this machine.
    Installable,
}

impl Shelf {
    /// The key the menu knows the shelf by.
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Self::Recents => "recents".to_owned(),
            Self::All => "all".to_owned(),
            Self::Category(category) => category.name().to_owned(),
            Self::Installable => "installable".to_owned(),
        }
    }

    /// The shelf a key names.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "recents" => Some(Self::Recents),
            "all" => Some(Self::All),
            "installable" => Some(Self::Installable),
            other => Category::from_name(other).map(Self::Category),
        }
    }

    /// The text of the shelf's row.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Recents => t!("launcher.recents"),
            Self::All => t!("launcher.all"),
            Self::Category(category) => t!(&format!("category.{}", category.name())),
            Self::Installable => t!("launcher.installable"),
        }
    }
}

/// How an application that is not installed would be installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Way {
    /// qpac installs this package.
    Qpac(String),
    /// quvyta installs this Quvyta application.
    Quvyta(String),
}

impl Way {
    /// How an entry says it is installed; Quvyta's own applications first, since one installed
    /// through the system's packages would still be quvyta's to update.
    #[must_use]
    pub fn of(entry: &Entry) -> Option<Self> {
        entry.install.quvyta.clone().map(Self::Quvyta).or_else(|| entry.install.qpac.clone().map(Self::Qpac))
    }

    /// What the person is told to run, with the words around it.
    #[must_use]
    pub fn sentence(&self, name: &str) -> String {
        match self {
            Self::Quvyta(member) => {
                t!("launcher.install-quvyta", name = name, command = format!("quvyta show {member}").as_str())
            }
            Self::Qpac(package) => t!("launcher.install-qpac", name = name, package = package.as_str()),
        }
    }
}

/// One application as the launcher shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// The entry's id.
    pub id: String,
    /// Its name in the language on screen.
    pub name: String,
    /// Its short description, empty when it has none.
    pub comment: String,
    /// The glyph of its icon.
    pub glyph: String,
    /// Whether its program is on this machine.
    pub installed: bool,
    /// How it would be installed, when it is not.
    pub way: Option<Way>,
    /// Whether it already has an icon on the floor.
    pub on_desktop: bool,
    /// What opening it starts, for the notice that names the command.
    pub command: Option<String>,
}

/// What the launcher shows and remembers while it is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launcher {
    /// What is typed in the search field.
    pub query: String,
    /// The shelf on the left that is chosen.
    pub shelf: Shelf,
    /// The card the person is on.
    pub selected: Option<usize>,
}

impl Default for Launcher {
    fn default() -> Self {
        Self { query: String::new(), shelf: Shelf::All, selected: None }
    }
}

/// What the launcher asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// Close the launcher.
    Close,
    /// The search field changed.
    Query(String),
    /// Enter in the search field: open the first hit.
    Submit,
    /// A shelf was chosen.
    Shelf(String),
    /// A card was selected.
    Select(usize),
    /// A card was activated.
    Activate(usize),
    /// Open this application.
    Open(String),
    /// Put an icon for it on the floor.
    AddIcon(String),
    /// Take its icon off the floor.
    RemoveIcon(String),
    /// Install it.
    Install(String),
}

/// The applications a shelf holds, or the hits of a query when something is typed.
///
/// A query searches the whole catalog, whatever shelf is chosen: someone who types `ht` wants
/// htop, not htop inside the category they happened to be looking at.
#[must_use]
pub fn shown(catalog: &Catalog, desktop: &Desktop, launcher: &Launcher, language: &str, icons: &Icons) -> Vec<Shown> {
    let query = launcher.query.trim();
    let dress = |entry: &Entry, installed: bool| Shown {
        id: entry.id.clone(),
        name: entry.name.get(language).to_owned(),
        comment: entry.comment.as_ref().map(|comment| comment.get(language).to_owned()).unwrap_or_default(),
        glyph: glyph_of(entry, icons),
        installed,
        way: Way::of(entry),
        on_desktop: desktop.icons.contains(&entry.id),
        command: match &entry.launch {
            Launch::Command(words) => Some(words.join(" ")),
            Launch::Open(path) => Some(path.display().to_string()),
            Launch::Screen(_) => None,
        },
    };
    if !query.is_empty() {
        return catalog.search(query, language).iter().map(|hit| dress(hit.entry, hit.installed)).collect();
    }
    match &launcher.shelf {
        Shelf::Recents => desktop
            .recents
            .iter()
            .filter_map(|id| catalog.get(id))
            .filter(|entry| catalog.is_installed(&entry.id))
            .map(|entry| dress(entry, true))
            .collect(),
        Shelf::All => catalog.installed(language).into_iter().map(|entry| dress(entry, true)).collect(),
        Shelf::Category(category) => catalog
            .groups(language)
            .into_iter()
            .find(|group| group.category == *category)
            .map(|group| group.entries.into_iter().map(|entry| dress(entry, true)).collect())
            .unwrap_or_default(),
        Shelf::Installable => catalog.installable(language).into_iter().map(|entry| dress(entry, false)).collect(),
    }
}

/// The shelves the launcher offers: the recents first when there are any, then everything, the
/// categories that hold something, and the Installable section when the machine is missing a
/// program qdesk knows.
#[must_use]
pub fn shelves(catalog: &Catalog, desktop: &Desktop, language: &str) -> Vec<Shelf> {
    let mut shelves = Vec::new();
    if desktop.recents.iter().any(|id| catalog.is_installed(id)) {
        shelves.push(Shelf::Recents);
    }
    shelves.push(Shelf::All);
    shelves.extend(catalog.groups(language).into_iter().map(|group| Shelf::Category(group.category)));
    if !catalog.installable(language).is_empty() {
        shelves.push(Shelf::Installable);
    }
    shelves
}

/// The size the launcher takes in a body of `body` cells.
#[must_use]
pub fn size(body: Size) -> Size {
    Size::new(body.width.min(MAX_WIDTH), body.height.min(MAX_HEIGHT))
}

/// Draws the launcher for `apps`, the applications [`shown`] gave.
pub fn view(launcher: &Launcher, apps: &[Shown], shelves: &[Shelf], ui: &mut View<'_, Msg>) {
    let search = ui.env().icons().glyph("search").into_owned();
    // The dock takes the bottom row; the launcher rises in what is left of the screen.
    let screen = ui.size();
    let room = size(Size::new(screen.width, screen.height.saturating_sub(1)));
    let cards: Rc<[Shown]> = apps.into();
    let chosen = launcher.selected.and_then(|index| apps.get(index)).cloned();
    ui.add_with(Panel::new(), |ui| {
        ui.row(|ui| {
            ui.add(Text::new(search));
            ui.add(
                TextInput::new(launcher.query.clone())
                    .placeholder(t!("launcher.search"))
                    .on_change(Msg::Query)
                    .on_submit(|_| Msg::Submit),
            )
            .id(SEARCH)
            .fill_width();
            ui.add(IconButton::new("close").on_press(Msg::Close).tooltip(t!("launcher.close")));
        })
        .fill_width()
        .gap(1);
        ui.row(|ui| {
            let items = shelves.iter().map(|shelf| MenuItem::new(shelf.key(), shelf.label()));
            ui.add(
                Menu::new([MenuGroup::new("shelves", items)])
                    .selected(launcher.query.trim().is_empty().then(|| launcher.shelf.key()).as_deref())
                    .on_select(|key| Msg::Shelf(key.to_owned())),
            )
            .width(Length::Cells(SHELF_WIDTH));
            // The menu acts on the card that is selected, and every row says which application
            // that is: a right click cannot name the card under the pointer, because the cards of
            // a grid take no input of their own.
            match chosen.map(|app| menu_items(&app)).filter(|items| !items.is_empty()) {
                Some(items) => {
                    ui.add_with(ContextMenu::new(items), |ui| grid(launcher, Rc::clone(&cards), ui)).fill();
                }
                None => grid(launcher, Rc::clone(&cards), ui),
            }
        })
        .fill()
        .gap(2);
        ui.add(
            KeyHints::new()
                .hint("enter", t!("launcher.hint-open"))
                .hint("ctrl+enter", t!("launcher.hint-add"))
                .hint("esc", t!("launcher.hint-close")),
        );
    })
    .width(Length::Cells(room.width))
    .height(Length::Cells(room.height));
}

/// The rows of one application's menu. Each names the application, so the menu is never about
/// something other than what it says.
fn menu_items(app: &Shown) -> Vec<ContextItem<Msg>> {
    let name = app.name.as_str();
    let mut items = Vec::new();
    if app.installed {
        items.push(ContextItem::new(t!("launcher.menu-open", name = name), Msg::Open(app.id.clone())));
    }
    if let Some(way) = &app.way
        && !app.installed
    {
        let label = match way {
            Way::Qpac(_) => t!("launcher.menu-install-qpac", name = name),
            Way::Quvyta(_) => t!("launcher.menu-install-quvyta", name = name),
        };
        items.push(ContextItem::new(label, Msg::Install(app.id.clone())));
    }
    if app.installed {
        let (label, message) = if app.on_desktop {
            (t!("launcher.menu-remove-icon", name = name), Msg::RemoveIcon(app.id.clone()))
        } else {
            (t!("launcher.menu-add-icon", name = name), Msg::AddIcon(app.id.clone()))
        };
        items.push(ContextItem::new(label, message));
    }
    items
}

/// The applications as cards: the name, and under it what it does or how to install it.
fn grid(launcher: &Launcher, cards: Rc<[Shown]>, ui: &mut View<'_, Msg>) {
    let empty = if launcher.query.trim().is_empty() {
        EmptyState::new(t!("launcher.empty")).message(t!("launcher.empty-hint"))
    } else {
        EmptyState::new(t!("launcher.no-hits", query = launcher.query.trim())).message(t!("launcher.no-hits-hint"))
    };
    ui.add(
        CardGrid::new(cards.len())
            .card_width(18, 24)
            .card_height(2)
            .selected(launcher.selected)
            .on_select(Msg::Select)
            .on_activate(Msg::Activate)
            .empty(empty)
            .card(move |ui, index| {
                let Some(app) = cards.get(index) else { return };
                let title = format!("{} {}", app.glyph, app.name);
                // An application that is not on the machine is faint, and says how it arrives.
                let role = if app.installed { "title" } else { "faint" };
                ui.add(Text::new(title).role(role).no_wrap());
                let second = match (&app.way, app.installed) {
                    (Some(Way::Qpac(_)), false) => t!("launcher.with-qpac"),
                    (Some(Way::Quvyta(_)), false) => t!("launcher.with-quvyta"),
                    _ if app.comment.is_empty() => String::new(),
                    _ => app.comment.clone(),
                };
                if !second.is_empty() {
                    ui.add(Text::new(second).role("secondary").no_wrap());
                }
            }),
    )
    .fill();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelf_keys_round_trip() {
        let mut shelves = vec![Shelf::Recents, Shelf::All, Shelf::Installable];
        shelves.extend(Category::ALL.map(Shelf::Category));
        for shelf in shelves {
            assert_eq!(Shelf::from_key(&shelf.key()), Some(shelf.clone()), "{shelf:?}");
        }
        assert_eq!(Shelf::from_key("games"), None);
    }

    #[test]
    fn the_launcher_opens_on_everything_with_an_empty_query() {
        let launcher = Launcher::default();
        assert_eq!(launcher.shelf, Shelf::All);
        assert!(launcher.query.is_empty());
        assert_eq!(launcher.selected, None);
    }

    #[test]
    fn it_never_grows_past_the_body_it_is_in() {
        assert_eq!(size(Size::new(200, 50)), Size::new(MAX_WIDTH, MAX_HEIGHT));
        assert_eq!(size(Size::new(60, 12)), Size::new(60, 12));
    }
}
