//! Installing an application from the launcher: qpac and quvyta do the installing, each in a
//! window of its own, and qdesk installs a missing installer with cargo when the person asks.

use qframe::prelude::*;
use qframe::runtime::Confirm;
use qframe::widgets::Toast;

use super::{Desk, FLOOR, Msg, Target};
use crate::apps::{Category, Entry, Launch, find_program, is_executable};
use crate::launcher::Way;

impl Desk {
    /// Installs an application: qpac and quvyta do the installing, each in a window of its own, and
    /// the corner says which package or member is meant.
    ///
    /// When the installer itself is not on this machine the person is asked whether qdesk should
    /// install it with cargo ([`install_here`](Self::install_here)); nothing starts before the
    /// answer. Without cargo either, the sentence is the whole of what can be done: it says what
    /// is missing, and a button onto a program that is not there would do nothing.
    pub(super) fn install(&mut self, target: &Target) -> Command<Msg> {
        let Some(way) = target.way.clone() else {
            return Command::toast(Toast::warning(t!("notice.install-unknown", name = target.name.as_str())));
        };
        let installer = way.installer();
        let Some(entry) = self.catalog.get(installer).filter(|entry| self.catalog.is_installed(&entry.id)).cloned()
        else {
            if self.cargo().is_none() {
                let missing = t!("launcher.no-cargo", installer = installer);
                return Command::toast(
                    Toast::info(t!("notice.install", name = target.name.as_str()))
                        .body(format!("{} {missing}", way.sentence(&target.name))),
                );
            }
            let command = way.install_here();
            let message = match &way {
                Way::Quvyta(_) => {
                    t!("launcher.here-quvyta", name = target.name.as_str(), command = command.as_str())
                }
                Way::Qpac(package) => t!(
                    "launcher.here-qpac",
                    name = target.name.as_str(),
                    package = package.as_str(),
                    command = command.as_str()
                ),
            };
            let question =
                Confirm::new(t!("launcher.here-title", installer = installer), Msg::InstallHere(target.clone()))
                    .message(message)
                    .confirm_label(t!("launcher.here-button"));
            return Command::confirm(question);
        };
        let told = Command::toast(
            Toast::info(t!("notice.install", name = target.name.as_str())).body(way.sentence(&target.name)),
        );
        // The installer is started by the path the catalog found it at, on the same `PATH` the
        // entries' programs are looked for in.
        let mut words = way.opening();
        if let Some(found) = self.found(installer) {
            words[0] = found;
        }
        let entry = match &way {
            // quvyta opens the member's own page, which is where its install button is.
            Way::Quvyta(_) => Entry { launch: Launch::Command(words), ..entry },
            // qpac is opened as it is: qdesk does not invent a command line for another
            // application.
            Way::Qpac(_) => match entry.launch {
                Launch::Command(_) => Entry { launch: Launch::Command(words), ..entry },
                Launch::Open(_) | Launch::Screen(_) => entry,
            },
        };
        let opened = self.open_entry(&entry, false);
        Command::batch([told, opened, self.body_focus()])
    }

    /// Where `program` is on the `PATH` of the environment, as text; `None` when it is not there
    /// or its path is not text.
    pub(super) fn found(&self, program: &str) -> Option<String> {
        find_program(program, self.apps.path.as_deref(), is_executable)?.to_str().map(str::to_owned)
    }

    /// Where cargo is, when this machine has it.
    pub(super) fn cargo(&self) -> Option<String> {
        self.found("cargo")
    }

    /// Installs the installer of `target` with cargo in a window of its own, where the person sees
    /// it work and can stop it, and then starts the installer on the application.
    ///
    /// The words go to the shell as arguments, never written into its script, so a member's name
    /// is only ever a name. The window has the environment's `PATH`, the one cargo and the
    /// installer were looked for in.
    pub(super) fn install_here(&mut self, target: &Target) -> Command<Msg> {
        let Some(way) = target.way.clone() else { return Command::none() };
        let Some(cargo) = self.cargo() else { return Command::none() };
        let installer = way.installer();
        let mut words = vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            r#"cargo="$1" crate="$2"; shift 2; "$cargo" install --locked "$crate" && exec "$@""#.to_owned(),
            "sh".to_owned(),
            cargo,
            way.installer_crate().to_owned(),
        ];
        words.extend(way.opening());
        let name = t!("launcher.here-window", installer = installer);
        let mut entry = Self::made_entry(installer, &name, "terminal", Category::System, Launch::Command(words));
        if let Some(path) = self.apps.path.as_deref().and_then(|path| path.to_str()) {
            entry.env.push(("PATH".to_owned(), path.to_owned()));
        }
        let closing = if self.launcher.take().is_some() { Command::focus(FLOOR) } else { Command::none() };
        let opened = self.open_entry(&entry, true);
        Command::batch([closing, opened, self.body_focus()])
    }
}
