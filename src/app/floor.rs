//! The floor: its icons and their menus, the entries of the Desktop folder
//! standing among them, the dialog that names a folder there, and the line a narrow floor shows.

use std::collections::BTreeSet;
use std::path::PathBuf;

use qframe::prelude::*;
use qframe::widgets::{
    ContextItem, ContextMenu, Field, FileChange, FileManagerMsg, FileManagerState, Form, Modal, NameFor, TextInput,
    Toast, Tooltip,
};

use super::{Desk, FLOOR, Msg, NAME_FIELD, NARROW_HEIGHT, NARROW_WIDTH, Target};
use crate::desktop::order::file_id;
use crate::desktop::{self, Cell, Desktop, Floor, IconCell, grid};
use crate::gadgets::Gadget;
use crate::wallpapers;

/// The width of that dialog, in cells: the framework's file manager asks with the same.
const NAMING_WIDTH: u16 = 48;

/// One icon as the floor draws it: what it is called and drawn with, what opening it does, and
/// the rows of its menu.
struct FloorIcon {
    id: String,
    name: String,
    glyph: String,
    open: Msg,
    menu: Vec<ContextItem<Msg>>,
}

impl Desk {
    /// What the floor's actions mean.
    pub(super) fn on_floor(&mut self, action: desktop::Action) -> Command<Msg> {
        let ids = self.floor_ids();
        // Any touch of the floor puts the launcher away, as a press outside it does.
        let closing = if self.launcher.take().is_some() { Command::focus(FLOOR) } else { Command::none() };
        match action {
            desktop::Action::Select { index, add } => {
                let Some(id) = ids.get(index) else { return closing };
                if add && self.selection.iter().any(|kept| kept == id) {
                    self.selection.retain(|kept| kept != id);
                } else if add {
                    self.selection.push(id.clone());
                } else {
                    self.selection = vec![id.clone()];
                }
                self.cursor = Some(index);
            }
            desktop::Action::Extend(index) => {
                // The range runs from the cursor, which stays where it is: a second shift click
                // draws the range again from the same icon, as in a list of files.
                let from = self.cursor.filter(|from| *from < ids.len()).unwrap_or(index);
                let (low, high) = (from.min(index), from.max(index));
                self.selection = ids.get(low..=high).map(<[String]>::to_vec).unwrap_or_default();
                if self.cursor.is_none() {
                    self.cursor = Some(index);
                }
            }
            desktop::Action::Band(indices) => {
                self.selection = indices.iter().filter_map(|index| ids.get(*index).cloned()).collect();
                self.cursor = indices.first().copied();
            }
            desktop::Action::Clear => self.selection.clear(),
            desktop::Action::Cursor(index) => self.cursor = Some(index),
            // Opening is the view's business: it makes the message with the name in it.
            desktop::Action::Open(index) => self.cursor = Some(index),
            desktop::Action::PlaceGadget { index, at } => {
                if let Some(gadget) = self.desktop.widgets.get_mut(index)
                    && gadget.place != at
                {
                    gadget.place = at;
                    return Command::batch([closing, self.save()]);
                }
            }
            desktop::Action::Place { index, moves, layout } => {
                let drawn: Vec<(String, Option<Cell>)> =
                    ids.into_iter().zip(layout.into_iter().chain(std::iter::repeat(None))).collect();
                if self.desktop.place_all(&drawn, &moves) {
                    // The order of the icons does not change, so the cursor, which counts in it,
                    // stays on the icon that moved.
                    self.cursor = Some(index);
                    return Command::batch([closing, self.save()]);
                }
            }
        }
        closing
    }

    /// Reads the Desktop folder and follows it, so what another program puts in it comes to the
    /// floor by itself. A screen test's desktop follows it too, each wait bounded by its patience
    /// (see [`watch_within`](Self::watch_within)).
    pub(super) fn read_folder(&mut self) -> Command<Msg> {
        let patience = self.patience;
        let Some(folder) = self.folder.take() else { return Command::none() };
        let folder = self.folder.insert(match patience {
            Some(bound) => folder.following_within(bound),
            None => folder.following(true),
        });
        folder.load(Msg::DesktopFolder)
    }

    /// Hands a message to the manager of the Desktop folder, keeping the floor in step with it: a
    /// renamed entry keeps its place, the places of entries gone from the folder are dropped, and
    /// the keys go back to the floor when the dialog asking for a name closes.
    pub(super) fn on_folder(&mut self, message: FileManagerMsg) -> Command<Msg> {
        let Some(folder) = &mut self.folder else { return Command::none() };
        let mut renamed = false;
        if let FileManagerMsg::Done(results) = &message {
            for (_, result) in results {
                // Only an entry of the folder itself stands on the floor; one moved inside a
                // folder of it leaves, and its place goes when the folder is read again.
                if let Ok(FileChange::Moved(from, to)) = result
                    && !from.contains('/')
                    && !to.contains('/')
                {
                    let (from, to) = (file_id(from), file_id(to));
                    self.desktop.rename_place(&from, &to);
                    for selected in &mut self.selection {
                        if *selected == from {
                            selected.clone_from(&to);
                        }
                    }
                    renamed = true;
                }
            }
        }
        let root_read = matches!(&message, FileManagerMsg::Listed(key, Ok(_)) if key.is_empty());
        let asking = folder.naming().is_some();
        let answer = folder.update(message, Msg::DesktopFolder);
        let asked = asking && folder.naming().is_none();
        if root_read && let Some(entries) = folder.children("") {
            let names: BTreeSet<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
            renamed |= self.desktop.keep_files(|name| names.contains(name));
        }
        let saved = if renamed { self.save() } else { Command::none() };
        let back = if asked { Command::focus(FLOOR) } else { Command::none() };
        Command::batch([answer, saved, back])
    }

    /// Opens the dialog asking for a name in the Desktop folder, with the keys in its field.
    pub(super) fn ask_name(&mut self, message: FileManagerMsg) -> Command<Msg> {
        let Some(folder) = &mut self.folder else { return Command::none() };
        let asked = folder.update(message, Msg::DesktopFolder);
        Command::batch([asked, Command::focus(NAME_FIELD)])
    }

    /// Whether the screen is too narrow for icons: the windows and the dock take it all.
    pub(super) fn narrow(size: Size) -> bool {
        size.width < NARROW_WIDTH || size.height < NARROW_HEIGHT
    }

    /// The icons on the floor, each with its own menu, and the menu of the empty floor.
    ///
    /// The applications come first, in their order, and the entries of the Desktop folder after
    /// them, each standing in its own place when it has one.
    pub(super) fn floor(&self, ui: &mut View<'_, Msg>) {
        let language = ui.env().i18n().active().to_owned();
        let icons = ui.env().icons();
        let mut shown: Vec<FloorIcon> = self
            .icons()
            .into_iter()
            .map(|entry| {
                let target = Target::of(entry, &language);
                FloorIcon {
                    id: target.id.clone(),
                    name: target.name.clone(),
                    glyph: desktop::glyph_of(entry, icons),
                    open: Msg::Open(target.clone()),
                    menu: Self::icon_items(&target, &self.desktop),
                }
            })
            .collect();
        if let Some(folder) = &self.folder {
            for entry in self.folder_entries() {
                let path = folder.root().join(&entry.name);
                let open = if entry.folder { Msg::FilesHere(path.clone()) } else { Msg::OpenFile(path.clone()) };
                shown.push(FloorIcon {
                    id: file_id(&entry.name),
                    name: entry.name.clone(),
                    glyph: icons.glyph(desktop::kind_icon(&entry.name, entry.folder, entry.executable)).into_owned(),
                    menu: Self::entry_items(&entry.name, &open, (!entry.folder).then_some(&path)),
                    open,
                });
            }
        }
        let names: Vec<String> = shown.iter().map(|icon| icon.name.clone()).collect();
        let places: Vec<Option<Cell>> = shown.iter().map(|icon| self.desktop.places.get(&icon.id).copied()).collect();
        let selected: Vec<usize> = shown
            .iter()
            .enumerate()
            .filter(|(_, icon)| self.selection.contains(&icon.id))
            .map(|(index, _)| index)
            .collect();
        let cursor = self.cursor;
        // In desktop mode the arrows pick a window, so the floor lets them through; while a name
        // is asked for, the keys are the dialog's.
        let naming = self.folder.as_ref().is_some_and(|folder| folder.naming().is_some());
        let keys = self.launcher.is_none() && self.keys.is_none() && !naming && !self.wallpaper.picking();
        let opening: Vec<Msg> = shown.iter().map(|icon| icon.open.clone()).collect();
        // Over a picture every icon stands on a tile of the theme's card tone, so its name reads
        // whatever the picture is under it.
        let backed = self.wallpaper_drawn(ui.env());
        let hidden = Self::narrow(ui.size());
        let hint = hidden.then(|| Self::narrow_hint(ui));
        ui.add_with(ContextMenu::new(self.floor_items(&language)), |ui| {
            if let Some(line) = hint {
                // A narrow screen keeps its room for the windows; the launcher still reaches
                // every application, and one line says how. The welcome, while it is up,
                // stands over that line and says the same.
                Self::narrow_hint_view(line, ui);
                return;
            }
            let floor = Floor::new(names, move |action| match action {
                desktop::Action::Open(index) => opening.get(index).cloned().unwrap_or(Msg::Ignore),
                other => Msg::Floor(other),
            })
            .places(places)
            .gadgets(self.desktop.widgets.iter().map(Gadget::spot).collect())
            .selected(selected)
            .cursor(cursor)
            .keys(keys);
            ui.add_with(floor, |ui| {
                for (index, icon) in shown.into_iter().enumerate() {
                    let cell = IconCell::new(icon.glyph, icon.name.clone())
                        .selected(self.selection.contains(&icon.id))
                        .cursor(cursor == Some(index))
                        .backed(backed);
                    let menu = ContextMenu::new(icon.menu);
                    let shortened = grid::shown_name(&icon.name) != icon.name;
                    if shortened {
                        // The cell shows what fits; the whole name is one hover away.
                        ui.add_with(Tooltip::new(icon.name), |ui| {
                            ui.add_with(menu, |ui| {
                                ui.add(cell);
                            });
                        });
                    } else {
                        ui.add_with(menu, |ui| {
                            ui.add(cell);
                        });
                    }
                }
                self.gadget_nodes(ui);
            })
            .id(FLOOR)
            .fill();
        })
        .fill();
    }

    /// The dialog asking for the name of a new folder of the Desktop, or a new name for one of its
    /// entries, while one is asked for. It is the framework's file manager's own dialog in words
    /// and in checks — the same title, field and buttons, and the manager says what is wrong with a
    /// name as it is typed — standing over the desktop instead of inside a window.
    pub(super) fn naming_view(folder: &FileManagerState, ui: &mut View<'_, Msg>) {
        let Some(naming) = folder.naming() else { return };
        let (title, confirm) = match &naming.purpose {
            NameFor::Rename(key) => {
                (t!("quvyta.file-manager.rename-title", name = key.as_str()), t!("quvyta.file-manager.rename-do"))
            }
            _ => (t!("quvyta.file-manager.new-folder-title"), t!("quvyta.file-manager.create")),
        };
        let close = Msg::DesktopFolder(FileManagerMsg::CloseNaming);
        let submit = Msg::DesktopFolder(FileManagerMsg::Submit);
        let dialog = Modal::new()
            .title(title)
            .width(NAMING_WIDTH)
            .on_close(close.clone())
            .action(Button::new(t!("quvyta.file-manager.cancel")).on_press(close))
            .action(Button::new(confirm).variant("primary").on_press(submit.clone()));
        let problem = folder.naming_problem().map(|problem| problem.message());
        let value = naming.value.clone();
        ui.add_with(dialog, |ui| {
            Form::new().show(ui, |fields| {
                let label = t!("quvyta.file-manager.name-label");
                fields.field(Field::new(label).error(problem.clone()), |ui| {
                    let input = TextInput::new(value)
                        .invalid(problem.is_some())
                        .on_change(|value| Msg::DesktopFolder(FileManagerMsg::Name(value)))
                        .on_submit(move |_| submit.clone());
                    ui.add(input).id(NAME_FIELD).fill_width();
                });
            });
        });
    }

    /// The one line a floor too narrow for icons shows in their place: where the
    /// applications went. The whole line names the keys too; where it does not fit, the shorter
    /// one names only the dock's button, so nothing is cut half way.
    pub(super) fn narrow_hint(ui: &View<'_, Msg>) -> String {
        let icons = ui.env().icons();
        let launcher = icons.glyph(desktop::quvyta_icon(icons)).into_owned();
        let whole = t!("floor.narrow-hint", icon = launcher.as_str());
        // A cell of floor on either side keeps the line off the screen's edges.
        if qframe::text::width(&whole) + 2 <= ui.size().width {
            whole
        } else {
            t!("floor.narrow-hint-short", icon = launcher.as_str())
        }
    }

    /// The narrow floor's line, quiet and in the middle of the floor.
    pub(super) fn narrow_hint_view(line: String, ui: &mut View<'_, Msg>) {
        ui.column(|ui| {
            ui.spacer();
            ui.add(Text::new(line).role("secondary").align(Align::Center).no_wrap()).fill_width();
            ui.spacer();
        })
        .fill();
    }

    /// The rows of an icon's menu.
    pub(super) fn icon_items(target: &Target, desktop: &Desktop) -> Vec<ContextItem<Msg>> {
        let mut items = vec![
            ContextItem::new(t!("icon.open"), Msg::Open(target.clone())),
            ContextItem::new(t!("icon.open-new"), Msg::OpenNew(target.clone())),
        ];
        if desktop.icons.contains(&target.id) {
            items.push(ContextItem::new(t!("icon.remove"), Msg::RemoveIcon(target.clone())));
        }
        items.push(ContextItem::gap());
        items.push(ContextItem::new(t!("icon.properties"), Msg::Properties(target.clone())));
        items
    }

    /// The rows of the menu of an entry of the Desktop folder called `name`, which `open` opens;
    /// `file` is where it is when it is a file, and a picture among them can be the wallpaper.
    pub(super) fn entry_items(name: &str, open: &Msg, file: Option<&PathBuf>) -> Vec<ContextItem<Msg>> {
        let mut items = vec![
            ContextItem::new(t!("icon.open"), open.clone()),
            ContextItem::new(t!("icon.rename"), Msg::RenameEntry(name.to_owned())),
        ];
        if let Some(file) = file.filter(|_| wallpapers::is_picture(name)) {
            items.push(ContextItem::new(t!("files.set-wallpaper"), Msg::SetWallpaper(file.clone())));
        }
        items
    }

    /// The rows of the empty floor's menu.
    pub(super) fn floor_items(&self, language: &str) -> Vec<ContextItem<Msg>> {
        let entry = |id: &str| self.catalog.get(id).map(|entry| Target::of(entry, language));
        let mut items = Vec::new();
        if let Some(terminal) = entry("terminal") {
            items.push(ContextItem::new(t!("floor.new-terminal"), Msg::Open(terminal)));
        }
        if self.folder.is_some() {
            items.push(ContextItem::new(t!("floor.new-folder"), Msg::NewFolder));
        }
        items.push(ContextItem::new(t!("floor.add-application"), Msg::OpenLauncher));
        items.extend(self.gadget_floor_items());
        let mut arranged: Vec<(String, String)> =
            self.icons().iter().map(|entry| (entry.name.get(language).to_lowercase(), entry.id.clone())).collect();
        arranged.sort();
        items.push(ContextItem::new(
            t!("floor.arrange"),
            Msg::Arrange(arranged.into_iter().map(|(_, id)| id).collect()),
        ));
        if let Some(settings) = entry("settings") {
            items.push(ContextItem::gap());
            items.push(ContextItem::new(t!("floor.settings"), Msg::Open(settings)));
        }
        items
    }

    /// Puts the icon of `target` on the floor.
    pub(super) fn add_icon(&mut self, target: &Target) -> Command<Msg> {
        self.desktop.add_icon(&target.id);
        self.save()
    }

    /// Takes the icon of `target` off the floor.
    pub(super) fn remove_icon(&mut self, target: &Target) -> Command<Msg> {
        self.desktop.remove_icon(&target.id);
        self.selection.retain(|id| *id != target.id);
        self.save()
    }

    /// What an icon's Properties says: its id, the file it was read from and its command.
    pub(super) fn properties(target: &Target) -> Command<Msg> {
        let file =
            target.file.as_ref().map(|file| file.display().to_string()).unwrap_or_else(|| t!("properties.built-in"));
        let command = target.command.clone().unwrap_or_else(|| t!("properties.no-command"));
        Command::toast(Toast::info(target.name.clone()).body(t!(
            "properties.body",
            id = target.id.as_str(),
            file = file.as_str(),
            command = command.as_str()
        )))
    }

    /// Arranges the icons in the `order` the floor's menu gives, each flowing again from the start.
    pub(super) fn arrange(&mut self, order: &[String]) -> Command<Msg> {
        // The ids the floor did not draw keep their places after the arranged ones.
        let mut icons = order.to_vec();
        icons.extend(self.desktop.icons.iter().filter(|id| !order.contains(id)).cloned());
        self.desktop.set_icons(icons);
        // Every icon flows again, in that order.
        self.desktop.clear_places();
        self.cursor = None;
        self.save()
    }
}
