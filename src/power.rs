//! The session actions at the foot of the launcher: lock the screen, log out, restart and power
//! off the machine.
//!
//! None of them needs root. On a machine qdesk was started on as the person's own session (the
//! `qdesk-session` of a Raspberry Pi), the system lets the active local session restart and power
//! off the machine, so `systemctl` is asked for that like any program, with no shell between. The
//! lock screen checks the password with the system's own helper, `unix_chkpwd`, which is made to
//! be run by the person whose password it checks. Where a helper is missing its action is not
//! offered: a lock that cannot check a password would be a lock that only looks like one.
//!
//! Over SSH only logging out is offered. Powering off there would turn the server off for everyone
//! on it, and a lock screen would lock only the one connection.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use qframe::prelude::*;
use qframe::runtime::{Line, Process, ProcessOutcome};

use crate::apps::Environment;

/// The program that powers off and restarts the machine.
const SYSTEMCTL: &str = "systemctl";

/// The helper that says whether a password is the person's own.
const CHKPWD: &str = "unix_chkpwd";

/// One of the session actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Cover the screen until the person's password is typed.
    Lock,
    /// Leave qdesk. Where a session service started it, the service starts it again.
    LogOut,
    /// Restart the machine.
    Restart,
    /// Power the machine off.
    PowerOff,
}

impl Action {
    /// Every action, in the order the launcher lists them: the gentlest first, so the one that
    /// ends the most stands at the far end of the row.
    pub const ALL: [Self; 4] = [Self::Lock, Self::LogOut, Self::Restart, Self::PowerOff];

    /// The key its words are looked up by, also the end of its button's id.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Lock => "lock",
            Self::LogOut => "log-out",
            Self::Restart => "restart",
            Self::PowerOff => "power-off",
        }
    }

    /// The words on its button, in the language on screen.
    #[must_use]
    pub fn label(self) -> String {
        t!(&format!("power.{}", self.key()))
    }

    /// The id of its button in the launcher.
    #[must_use]
    pub fn id(self) -> String {
        format!("power-{}", self.key())
    }
}

/// The helpers the session actions run, found once when the desktop is given its environment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tools {
    systemctl: Option<PathBuf>,
    checker: Option<Checker>,
}

impl Tools {
    /// The helpers of `env`: `systemctl` and `unix_chkpwd` on its `PATH` or in its system folders,
    /// and the person the password is checked for.
    #[must_use]
    pub fn find(env: &Environment) -> Self {
        let checker = env.find_tool(CHKPWD).zip(env.user.clone()).map(|(program, user)| Checker { program, user });
        Self { systemctl: env.find_tool(SYSTEMCTL), checker }
    }

    /// The actions offered on a terminal that is `remote` or not, in the launcher's order.
    #[must_use]
    pub fn offered(&self, remote: bool) -> Vec<Action> {
        Action::ALL
            .into_iter()
            .filter(|action| match action {
                Action::LogOut => true,
                Action::Lock => !remote && self.checker.is_some(),
                Action::Restart | Action::PowerOff => !remote && self.systemctl.is_some(),
            })
            .collect()
    }

    /// What checks the password on the lock screen, when there is anything to.
    #[must_use]
    pub fn checker(&self) -> Option<&Checker> {
        self.checker.as_ref()
    }

    /// The program and the words that carry out `action` on the machine: restarting and powering
    /// off. `None` for the actions qdesk carries out itself, and where `systemctl` is missing.
    ///
    /// `--no-ask-password` keeps `systemctl` from asking for a password on the terminal qdesk is
    /// drawn on: where the system does not allow it, it says so and ends, and the corner tells
    /// the person why.
    #[must_use]
    pub fn command(&self, action: Action) -> Option<Process> {
        let verb = match action {
            Action::Restart => "reboot",
            Action::PowerOff => "poweroff",
            Action::Lock | Action::LogOut => return None,
        };
        let program = self.systemctl.clone()?;
        Some(Process::new(program).args(["--no-ask-password", verb]).no_stdin())
    }
}

/// Runs `process` to its end and says why it failed, if it did: what it wrote to its error
/// stream, else its exit code. It is run off the render path; the machine is usually going down
/// by the time it ends.
///
/// # Errors
///
/// The reason, in the program's own words, when the program could not be started or failed.
pub fn carry_out(process: Process) -> Result<(), String> {
    let mut said = Vec::new();
    let outcome = process.run(&|| false, &mut |line| {
        if let Line::Err(text) | Line::Out(text) = line
            && !text.trim().is_empty()
        {
            said.push(text.trim().to_owned());
        }
    });
    match outcome {
        Ok(ProcessOutcome::Finished { code: Some(0) }) => Ok(()),
        Ok(ProcessOutcome::Finished { code }) => Err(said.pop().unwrap_or_else(|| match code {
            Some(code) => t!("power.exit-code", code = code),
            None => t!("power.killed"),
        })),
        Ok(ProcessOutcome::Cancelled) => Err(t!("power.killed")),
        Err(error) => Err(error.to_string()),
    }
}

/// Says whether a password is the person's own, through the system's `unix_chkpwd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checker {
    program: PathBuf,
    user: String,
}

impl Checker {
    /// The person whose password is checked.
    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }

    /// Whether `password` is the person's password. Anything that goes wrong on the way — the
    /// helper cannot be started, it is killed, it says no — is a no: the screen stays locked.
    ///
    /// The helper reads the password from a pipe, never a terminal, ended by a NUL byte; without
    /// that byte it takes the password as missing. It answers with its exit status, 0 for yes.
    /// `nullok` lets a person who has no password at all unlock with an empty one, as logging in
    /// does for them.
    #[must_use]
    pub fn check(&self, password: &str) -> bool {
        let child = Command::new(&self.program)
            .args([self.user.as_str(), "nullok"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else { return false };
        let written = child.stdin.take().is_some_and(|mut stdin| {
            let mut bytes = password.as_bytes().to_vec();
            bytes.push(0);
            let sent = stdin.write_all(&bytes).is_ok();
            // The copy is cleared before it goes; the pipe is closed when `stdin` drops.
            bytes.fill(0);
            sent
        });
        let status = child.wait();
        written && status.is_ok_and(|status| status.success())
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// A folder of the test's own under the system's temporary folder, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let number = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("qdesk-power-{}-{number}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("bin")).expect("a scratch folder");
            Self(path)
        }

        fn write(&self, relative: &str, text: String) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::write(&path, text).expect("a file");
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn env(scratch: &Scratch, user: Option<&str>) -> Environment {
        Environment {
            path: Some(OsString::from(scratch.0.join("bin"))),
            user: user.map(str::to_owned),
            ..Environment::default()
        }
    }

    /// Writes an executable shell script at `relative` running `body`.
    fn script(scratch: &Scratch, relative: &str, body: &str) -> PathBuf {
        let path = scratch.write(relative, format!("#!/bin/sh\n{body}\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("permissions");
        }
        path
    }

    #[test]
    fn a_machine_with_every_helper_offers_every_action_and_ssh_only_logging_out() {
        let scratch = Scratch::new();
        script(&scratch, "bin/systemctl", "exit 0");
        script(&scratch, "bin/unix_chkpwd", "exit 1");
        let tools = Tools::find(&env(&scratch, Some("ada")));
        assert_eq!(tools.offered(false), Action::ALL);
        assert_eq!(tools.offered(true), [Action::LogOut], "over SSH the machine is everyone's");
    }

    #[test]
    fn a_missing_helper_hides_its_actions() {
        let scratch = Scratch::new();
        assert_eq!(Tools::find(&env(&scratch, Some("ada"))).offered(false), [Action::LogOut]);
        script(&scratch, "bin/unix_chkpwd", "exit 1");
        let nobody = Tools::find(&env(&scratch, None));
        assert_eq!(nobody.offered(false), [Action::LogOut], "no lock without a person to check");
        assert_eq!(Tools::find(&env(&scratch, Some("ada"))).offered(false), [Action::Lock, Action::LogOut]);
    }

    #[test]
    fn powering_off_and_restarting_ask_systemctl_without_a_password_prompt() {
        let scratch = Scratch::new();
        let said = scratch.0.join("said");
        script(&scratch, "bin/systemctl", &format!("printf '%s\\n' \"$@\" >> '{}'", said.display()));
        let tools = Tools::find(&env(&scratch, None));
        assert!(tools.command(Action::Lock).is_none() && tools.command(Action::LogOut).is_none());
        for action in [Action::PowerOff, Action::Restart] {
            let process = tools.command(action).expect("systemctl is there");
            assert_eq!(carry_out(process), Ok(()));
        }
        let words = std::fs::read_to_string(&said).expect("systemctl was asked");
        assert_eq!(words, "--no-ask-password\npoweroff\n--no-ask-password\nreboot\n");
    }

    #[test]
    fn a_refusal_is_told_in_the_program_s_own_words() {
        let scratch = Scratch::new();
        script(&scratch, "bin/systemctl", "echo 'Access denied' >&2\nexit 1");
        let tools = Tools::find(&env(&scratch, None));
        let refused = carry_out(tools.command(Action::PowerOff).expect("systemctl is there"));
        assert_eq!(refused, Err("Access denied".to_owned()));
    }

    #[test]
    fn the_password_reaches_the_helper_on_a_pipe_ended_by_nul_with_the_user_named() {
        let scratch = Scratch::new();
        let said = scratch.0.join("said");
        // What the real helper does: it reads up to the NUL, and answers 0 only for the right one.
        script(
            &scratch,
            "bin/unix_chkpwd",
            &format!(
                "printf '%s %s\\n' \"$1\" \"$2\" > '{said}'\n[ -t 0 ] && exit 9\n\
                 got=$(od -An -c | tr -d ' \\n')\n[ \"$got\" = 'gizli\\0' ]",
                said = said.display()
            ),
        );
        let tools = Tools::find(&env(&scratch, Some("ada")));
        let checker = tools.checker().expect("a helper and a person");
        assert_eq!(checker.user(), "ada");
        assert!(checker.check("gizli"));
        assert_eq!(std::fs::read_to_string(&said).expect("the helper ran"), "ada nullok\n");
        assert!(!checker.check("yanlış"));
        assert!(!checker.check("gizl"));
    }

    #[test]
    fn a_helper_that_cannot_run_keeps_the_screen_locked() {
        let checker = Checker { program: Path::new("/nonexistent/unix_chkpwd").to_path_buf(), user: "ada".into() };
        assert!(!checker.check(""));
    }
}
