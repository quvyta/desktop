//! The lock screen and the session actions at the launcher's foot: locking,
//! logging out, restarting and powering off.

use std::path::Path;
use std::time::Duration;

use qframe::prelude::*;
use qframe::runtime::{Confirm, Task};
use qframe::widgets::{BigText, TextInput, Toast};

use super::{Desk, FLOOR, Msg};
use crate::inbox::Notice;
use crate::power::{self, Action};
use crate::secret::Secret;

/// The name of the password field of the lock screen, so the keys are in it the moment the screen
/// locks and after a wrong password.
pub const LOCK_FIELD: &str = "lock-password";

/// The width of the password field of the lock screen, in cells.
const LOCK_FIELD_WIDTH: u16 = 36;

/// What the lock screen holds while it covers the desktop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Lock {
    /// What is typed in the password field: the only copy qdesk keeps.
    typed: Secret,
    /// Whether a password is being checked; another Enter waits for the answer.
    checking: bool,
    /// Whether the last password was not the person's.
    wrong: bool,
    /// Wrong passwords since the screen locked or since the last right one.
    failures: u32,
    /// Seconds left before the field takes another attempt; 0 when it takes one now.
    wait: u32,
}

/// The name of the mark a locked screen leaves in the desktop's state folder.
pub(super) const LOCK_MARK: &str = "locked";

/// The longest wait after wrong passwords, in seconds.
const LOCK_WAIT_CAP: u32 = 30;

/// How long the lock screen refuses another attempt after `failures` wrong passwords in a row, in
/// seconds: 1, 2, 4, 8 and so on, never more than [`LOCK_WAIT_CAP`].
fn lock_wait(failures: u32) -> u32 {
    let doublings = failures.saturating_sub(1);
    1u32.checked_shl(doublings).unwrap_or(u32::MAX).min(LOCK_WAIT_CAP)
}

/// Waits one second of the lock screen's wait after a wrong password. The task sleeps on the
/// runtime's clock, which a test moves by hand.
fn lock_tick() -> Command<Msg> {
    Command::task(Task::new("lock-wait", |cx| {
        if cx.sleep(Duration::from_secs(1)) { Ok(Msg::LockTick) } else { Err("stopped".to_owned()) }
    }))
}

impl Desk {
    /// A session action pressed at the launcher's foot. The launcher goes away first, as it does
    /// for an application.
    pub(super) fn on_power(&mut self, action: Action) -> Command<Msg> {
        self.launcher = None;
        // Only what the launcher offered can be asked for: a message that names another action,
        // on a machine or a connection where it is hidden, does nothing.
        if !self.tools.offered(self.remote).contains(&action) {
            return Command::focus(FLOOR);
        }
        match action {
            Action::Lock => self.lock_screen(),
            // Logging out is leaving qdesk, with the question leaving asks when programs run.
            Action::LogOut if self.programs.running() > 0 => Command::batch([Command::focus(FLOOR), self.ask_quit()]),
            Action::LogOut => Command::quit(),
            Action::Restart | Action::PowerOff => {
                let running = self.programs.running();
                let key = action.key();
                let message = if running > 0 {
                    t!(&format!("power.{key}-running"), n = running)
                } else {
                    t!(&format!("power.{key}-text"))
                };
                let question = Confirm::new(t!(&format!("power.{key}-title")), Msg::PowerConfirmed(action))
                    .message(message)
                    .confirm_label(action.label())
                    .danger();
                Command::batch([Command::focus(FLOOR), Command::confirm(question)])
            }
        }
    }

    /// Restarting or powering off, once confirmed: `systemctl` is asked off the render path, and
    /// the corner says why when the system does not agree.
    ///
    /// The only way here is the answer to the question [`on_power`](Self::on_power) asks. The
    /// action is looked at again all the same: an answer that arrives after the terminal became a
    /// remote one, or a message from anywhere else, never powers off what the launcher would not
    /// have offered.
    pub(super) fn carry_out(&self, action: Action) -> Command<Msg> {
        if !self.tools.offered(self.remote).contains(&action) {
            return Command::none();
        }
        match self.tools.command(action) {
            Some(process) => Command::perform(move || Msg::PowerDone(action, power::carry_out(process))),
            None => Command::none(),
        }
    }

    /// Covers the desktop with the lock screen and leaves the mark that keeps it locked across a
    /// restart. A mark that cannot be written leaves the lock of this run as it is: nobody is at
    /// the keyboard to be told, and the screen is locked either way.
    pub(super) fn lock_screen(&mut self) -> Command<Msg> {
        self.lock = Some(Lock::default());
        self.keys = None;
        self.help = false;
        self.more = false;
        self.inbox_open = false;
        if let Some(mark) = &self.lock_mark {
            if let Some(folder) = mark.parent() {
                let _ = std::fs::create_dir_all(folder);
            }
            let _ = std::fs::write(mark, "");
        }
        Command::focus(LOCK_FIELD)
    }

    /// Whether the desktop starts locked: the mark of a locked screen is there, left by a qdesk
    /// that ended without being opened. Only while the password can be checked, since a lock
    /// nobody can open would shut the person out of their own desktop.
    pub(super) fn starts_locked(&self) -> bool {
        self.tools.checker().is_some() && self.lock_mark.as_deref().is_some_and(Path::exists)
    }

    /// Checks what is typed on the lock screen, off the render path. The field is emptied at once:
    /// the password is not kept on screen or in the desktop while it is checked.
    pub(super) fn unlock(&mut self) -> Command<Msg> {
        let Some(lock) = &mut self.lock else { return Command::none() };
        // After a wrong password the next attempt waits: guessing is slowed by qdesk itself, not
        // only by how fast `unix_chkpwd` answers. What is typed meanwhile stays in the field.
        if lock.checking || lock.wait > 0 {
            return Command::none();
        }
        let Some(checker) = self.tools.checker().cloned() else { return Command::none() };
        lock.checking = true;
        lock.wrong = false;
        // Moved, not copied, and overwritten when the check drops it.
        let typed = lock.typed.take();
        Command::perform(move || Msg::Unlocked(checker.check(typed.as_str())))
    }

    /// The lock screen: the time, the machine and a field for the person's password, over
    /// everything else. Nothing of the desktop is drawn under it, so nothing of it can be read or
    /// pressed.
    pub(super) fn lock_view(&self, lock: &Lock, ui: &mut View<'_, Msg>) {
        let user = self.tools.checker().map(|checker| checker.user().to_owned()).unwrap_or_default();
        let clock = self.clock_text();
        let machine = self.machine_text();
        ui.column(|ui| {
            ui.spacer();
            ui.add(BigText::new(clock));
            ui.add(Text::new(machine).role("secondary").no_wrap());
            ui.spacer().height(Length::Cells(1));
            ui.row(|ui| {
                ui.add(
                    TextInput::new(lock.typed.as_str())
                        .password(true)
                        .placeholder(t!("power.lock-password", user = user.as_str()))
                        .disabled(lock.checking)
                        .invalid(lock.wrong)
                        .on_change(|typed| Msg::LockTyped(Secret::from(typed)))
                        // The field hands over its text on Enter too; the lock screen already has
                        // it, so that copy is overwritten at once.
                        .on_submit(|typed| {
                            drop(Secret::from(typed));
                            Msg::Unlock
                        }),
                )
                .id(LOCK_FIELD)
                .width(Length::Cells(LOCK_FIELD_WIDTH));
            });
            let (line, role) = if lock.checking {
                (t!("power.lock-checking"), "secondary")
            } else if lock.wait > 0 {
                (t!("power.lock-wait", n = lock.wait), "")
            } else if lock.wrong {
                (t!("power.lock-wrong"), "")
            } else {
                (t!("power.lock-hint"), "faint")
            };
            let line = Text::new(line).no_wrap();
            let line = if role.is_empty() { line.color("danger") } else { line.role(role) };
            ui.add(line);
            ui.spacer();
        })
        .align(Align::Center)
        .fill();
    }

    /// A session action the system refused: the corner says why, and the list keeps it.
    pub(super) fn power_failed(&mut self, action: Action, reason: String) -> Command<Msg> {
        let heading = t!(&format!("power.{}-failed", action.key()));
        self.inbox.add(Notice::desktop(heading.clone(), reason.clone()));
        Command::toast(Toast::warning(heading).body(reason))
    }

    /// What is typed on the lock screen, kept as its field's only copy.
    pub(super) fn lock_typed(&mut self, typed: Secret) -> Command<Msg> {
        if let Some(lock) = &mut self.lock {
            lock.typed = typed;
            lock.wrong = false;
        }
        Command::none()
    }

    /// The answer of the password check: `right` opens the desktop and takes the mark away, a
    /// wrong one makes the next attempt wait.
    pub(super) fn unlocked(&mut self, right: bool) -> Command<Msg> {
        if right {
            self.lock = None;
            if let Some(mark) = &self.lock_mark {
                let _ = std::fs::remove_file(mark);
            }
            return self.body_focus();
        }
        let Some(lock) = &mut self.lock else { return Command::none() };
        lock.checking = false;
        lock.wrong = true;
        lock.failures = lock.failures.saturating_add(1);
        lock.wait = lock_wait(lock.failures);
        Command::batch([Command::focus(LOCK_FIELD), lock_tick()])
    }

    /// One second of the wait after a wrong password has passed.
    pub(super) fn lock_ticked(&mut self) -> Command<Msg> {
        match &mut self.lock {
            Some(lock) if lock.wait > 0 => {
                lock.wait -= 1;
                if lock.wait > 0 { lock_tick() } else { Command::none() }
            }
            _ => Command::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_after_wrong_passwords_doubles_up_to_half_a_minute() {
        let waits: Vec<u32> = (1..=7).map(lock_wait).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30]);
        assert_eq!(lock_wait(u32::MAX), LOCK_WAIT_CAP, "a count that never ends still waits half a minute");
    }
}
