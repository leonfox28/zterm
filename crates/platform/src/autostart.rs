//! Opt-in login registration. Never starts, stops, or supervises a process.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::user_state::UserPaths;

const LABEL: &str = "io.github.leonfox28.zterm.daemon";
const UNIT: &str = "zterm-daemon.service";
const MARKER: &str = "Managed by zterm daemon autostart v1";
/// Service managers enter without daemonizing, before a Tokio runtime exists.
pub const FOREGROUND_ARGUMENT: &str = "--internal-daemon-foreground";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Manager {
    LaunchAgent,
    Systemd,
}

/// One account's fixed registration paths, separate from its identity state.
#[derive(Debug)]
pub struct Autostart {
    home: PathBuf,
    uid: u32,
    manager: Manager,
}

impl Autostart {
    /// Resolves supported native login integration without creating anything.
    pub fn current(paths: &UserPaths) -> Result<Self, String> {
        let owner = Self::for_cleanup(paths).ok_or_else(|| {
            "daemon autostart is supported only on macOS and systemd Linux".to_owned()
        })?;
        if owner.manager == Manager::Systemd && !Path::new("/run/systemd/system").is_dir() {
            return Err(
                "daemon autostart is unsupported: Linux requires systemd user services".into(),
            );
        }
        if owner.manager == Manager::Systemd {
            // systemd's user unit search path follows XDG_CONFIG_HOME. Do not
            // silently enable an ignored file when the login environment differs.
            if let Some(config) = std::env::var_os("XDG_CONFIG_HOME")
                && !config.is_empty()
                && Path::new(&config) != paths.home().join(".config")
            {
                return Err("daemon autostart requires the standard account-home .config systemd user directory; custom XDG_CONFIG_HOME is unsupported".into());
            }
        }
        Ok(owner)
    }

    /// Resolves owned registration for reset/uninstall, even after systemd removal.
    pub fn for_cleanup(paths: &UserPaths) -> Option<Self> {
        let manager = if cfg!(target_os = "macos") {
            Manager::LaunchAgent
        } else if cfg!(target_os = "linux") {
            Manager::Systemd
        } else {
            return None;
        };
        Some(Self {
            home: paths.home().into(),
            uid: paths.uid(),
            manager,
        })
    }

    fn directory(&self) -> PathBuf {
        self.home.join(match self.manager {
            Manager::LaunchAgent => "Library/LaunchAgents",
            Manager::Systemd => ".config/systemd/user",
        })
    }

    fn registration(&self) -> PathBuf {
        self.directory().join(match self.manager {
            Manager::LaunchAgent => format!("{LABEL}.plist"),
            Manager::Systemd => UNIT.into(),
        })
    }

    fn link(&self) -> PathBuf {
        self.directory().join("default.target.wants").join(UNIT)
    }

    /// Read-only configuration inspection; does not contact or start a daemon.
    pub fn status(&self, executable: &Path) -> Result<String, String> {
        self.directories(false)?;
        let content = self.owned_content()?;
        let enabled = match self.manager {
            Manager::LaunchAgent => content.is_some(),
            Manager::Systemd => self.link_exists()?,
        };
        if enabled && content.is_none() {
            return Err(
                "autostart registration is incomplete: enabled link has no service file".into(),
            );
        }
        if let Some(content) = content {
            if content != self.render(executable)? {
                return Err("autostart configuration differs from this executable; run zterm daemon autostart enable to repair it".into());
            }
            self.validate_executable(executable)?;
            if self.manager == Manager::LaunchAgent
                && crate::account::EffectiveAccount::current()
                    .is_ok_and(|account| account.home() == self.home)
            {
                let domain = format!("gui/{}", self.uid);
                if let Ok(output) = std::process::Command::new("/bin/launchctl")
                    .args(["print-disabled", &domain])
                    .output()
                    && output.status.success()
                    && String::from_utf8_lossy(&output.stdout).lines().any(|line| {
                        line.contains(&format!("\"{LABEL}\""))
                            && (line.contains("=> disabled") || line.contains("=> true"))
                    })
                {
                    return Err("autostart is disabled by a launchctl override; inspect launchctl print-disabled for this user".into());
                }
            }
            if !enabled {
                return Err("autostart service exists but is not enabled; run zterm daemon autostart enable or disable".into());
            }
        }
        Ok(if enabled {
            "Daemon autostart: enabled (next login).\n"
        } else {
            "Daemon autostart: disabled.\n"
        }
        .into())
    }

    /// Installs only next-login configuration; no load/start, restart policy or linger.
    pub fn enable(&self, executable: &Path) -> Result<(), String> {
        self.validate_executable(executable)?;
        let content = self.render(executable)?;
        self.directories(true)?;
        self.owned_content()?; // Never overwrite a foreign or unsafe registration.
        if self.manager == Manager::Systemd {
            self.link_exists()?;
        }
        let destination = self.registration();
        let temporary = self
            .directory()
            .join(format!(".zterm-autostart-{}.tmp", std::process::id()));
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(detail)?;
        let result = (|| {
            output.write_all(content.as_bytes()).map_err(detail)?;
            output.sync_all().map_err(detail)?;
            fs::rename(&temporary, &destination).map_err(detail)?;
            File::open(self.directory())
                .and_then(|file| file.sync_all())
                .map_err(detail)?;
            if self.manager == Manager::Systemd && !self.link_exists()? {
                std::os::unix::fs::symlink(format!("../{UNIT}"), self.link()).map_err(detail)?;
                File::open(self.directory().join("default.target.wants"))
                    .and_then(|file| file.sync_all())
                    .map_err(detail)?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }

    /// Removes owned next-login files, leaving the current process and Sessions alone.
    pub fn disable(&self) -> Result<(), String> {
        self.directories(false)?;
        let content = self.owned_content()?;
        let link = self.manager == Manager::Systemd && self.link_exists()?;
        if link {
            fs::remove_file(self.link()).map_err(detail)?;
        }
        if content.is_some() {
            fs::remove_file(self.registration()).map_err(detail)?;
        }
        // launchd may retain the current job until logout. No KeepAlive or trigger
        // is installed, so removing its file cancels future login without bootout
        // (which would kill a running daemon and its Sessions).
        Ok(())
    }

    /// Clears manager registration after the daemon and its Sessions have stopped.
    pub fn remove_after_stop(&self) -> Result<(), String> {
        self.disable()?;
        // Library fixtures never communicate with the real account's manager.
        let account = crate::account::EffectiveAccount::current().map_err(detail)?;
        if account.home() != self.home || account.uid() != self.uid {
            return Ok(());
        }
        match self.manager {
            Manager::LaunchAgent => {
                let service = format!("gui/{}/{LABEL}", self.uid);
                let observed = std::process::Command::new("/bin/launchctl")
                    .args(["print", &service])
                    .output()
                    .map_err(detail)?;
                if !observed.status.success() {
                    return Ok(());
                } // No loaded GUI job.
                let expected = format!("path = {}", self.registration().display());
                if !String::from_utf8_lossy(&observed.stdout)
                    .lines()
                    .any(|line| line.trim() == expected)
                {
                    return Err(
                        "loaded launchd label has a different origin; refusing to remove it".into(),
                    );
                }
                let result = std::process::Command::new("/bin/launchctl")
                    .args(["bootout", &service])
                    .output()
                    .map_err(detail)?;
                if !result.status.success() {
                    return Err(
                        "unable to remove stopped launchd registration; retry reset/uninstall"
                            .into(),
                    );
                }
            }
            Manager::Systemd => {
                // Reload forgets the removed unit, without stopping any process,
                // enabling linger, or activating a target. No bus means no
                // running user manager to clear; next login reads the disk state.
                if Path::new("/run/systemd/system").is_dir() {
                    let result = std::process::Command::new("systemctl")
                        .args(["--user", "daemon-reload"])
                        .output()
                        .map_err(|_| "unable to run systemctl; retry reset/uninstall".to_owned())?;
                    if !result.status.success()
                        && Path::new(&format!("/run/user/{}/bus", self.uid)).exists()
                    {
                        return Err(
                            "unable to reload systemd user configuration; retry reset/uninstall"
                                .into(),
                        );
                    }
                }
            }
        }
        Ok(())
    }

    fn render(&self, executable: &Path) -> Result<String, String> {
        let executable = path_text(executable)?;
        let home = path_text(&self.home)?;
        Ok(match self.manager {
            Manager::LaunchAgent => format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- {MARKER} -->\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>{LABEL}</string>\n<key>ProgramArguments</key><array><string>{}</string><string>{FOREGROUND_ARGUMENT}</string></array>\n<key>WorkingDirectory</key><string>{}</string>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><false/>\n</dict></plist>\n",
                xml(executable),
                xml(home)
            ),
            Manager::Systemd => format!(
                "# {MARKER}\n[Unit]\nDescription=zterm per-user daemon\n\n[Service]\nType=exec\nExecStart={} {FOREGROUND_ARGUMENT}\nWorkingDirectory={}\nRestart=no\nKillMode=process\nUMask=0077\n\n[Install]\nWantedBy=default.target\n",
                unit_quote(executable),
                unit_quote(home).replace("$$", "$")
            ),
        })
    }

    fn validate_executable(&self, executable: &Path) -> Result<(), String> {
        path_text(executable)?;
        let meta = fs::symlink_metadata(executable).map_err(detail)?;
        if !meta.is_file()
            || meta.uid() != self.uid
            || meta.mode() & 0o022 != 0
            || meta.mode() & 0o100 == 0
        {
            return Err(
                "autostart requires a user-owned executable that is not writable by other users"
                    .into(),
            );
        }
        Ok(())
    }

    fn directories(&self, create: bool) -> Result<(), String> {
        let relative = self
            .directory()
            .strip_prefix(&self.home)
            .map_err(detail)?
            .to_path_buf();
        let mut path = self.home.clone();
        check_directory(&path, self.uid)?;
        for component in relative.components() {
            path.push(component);
            if !ensure_directory(&path, self.uid, create)? {
                return Ok(());
            }
        }
        if self.manager == Manager::Systemd {
            ensure_directory(&path.join("default.target.wants"), self.uid, create)?;
        }
        Ok(())
    }

    fn owned_content(&self) -> Result<Option<String>, String> {
        let path = self.registration();
        let Some(meta) = metadata(&path)? else {
            return Ok(None);
        };
        if !meta.is_file()
            || meta.uid() != self.uid
            || meta.mode() & 0o022 != 0
            || meta.len() > 16384
        {
            return Err("unsafe autostart registration; refusing to modify it".into());
        }
        let mut content = String::new();
        OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(path)
            .map_err(detail)?
            .take(16385)
            .read_to_string(&mut content)
            .map_err(detail)?;
        if !content
            .lines()
            .any(|line| line == format!("# {MARKER}") || line == format!("<!-- {MARKER} -->"))
        {
            return Err(
                "autostart path belongs to an unrecognized configuration; refusing to modify it"
                    .into(),
            );
        }
        Ok(Some(content))
    }

    fn link_exists(&self) -> Result<bool, String> {
        let path = self.link();
        let Some(meta) = metadata(&path)? else {
            return Ok(false);
        };
        if !meta.is_symlink()
            || meta.uid() != self.uid
            || fs::read_link(path).map_err(detail)? != Path::new(&format!("../{UNIT}"))
        {
            return Err("unrecognized autostart enable link; refusing to modify it".into());
        }
        Ok(true)
    }
}

fn detail(error: impl std::fmt::Display) -> String {
    format!("autostart: {error}")
}
fn metadata(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(detail(error)),
    }
}
fn check_directory(path: &Path, uid: u32) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(detail)?;
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o022 != 0 {
        return Err("unsafe autostart directory (type, owner or permissions)".into());
    }
    Ok(())
}
fn ensure_directory(path: &Path, uid: u32, create: bool) -> Result<bool, String> {
    if metadata(path)?.is_none() {
        if !create {
            return Ok(false);
        }
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(detail(error)),
        }
    }
    check_directory(path, uid)?;
    Ok(true)
}
fn path_text(path: &Path) -> Result<&str, String> {
    let value = path
        .to_str()
        .filter(|text| !text.chars().any(char::is_control));
    if !path.is_absolute() || value.is_none() {
        return Err("autostart requires absolute UTF-8 paths without control characters".into());
    }
    Ok(value.expect("validated path text"))
}
fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn unit_quote(text: &str) -> String {
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('$', "$$")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn both_managers_are_opt_in_idempotent_and_remove_only_owned_files() {
        for manager in [Manager::LaunchAgent, Manager::Systemd] {
            let temp = tempfile::tempdir().expect("fixture");
            let owner = Autostart {
                home: temp.path().into(),
                uid: nix::unistd::Uid::effective().as_raw(),
                manager,
            };
            let executable = temp.path().join("zterm %$ & \" binary");
            fs::write(&executable, "fixture executable").expect("fixture");
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).expect("mode");
            assert!(
                owner
                    .status(&executable)
                    .expect("status")
                    .contains("disabled")
            );
            assert!(
                !owner.directory().exists(),
                "status must not create directories"
            );
            owner.disable().expect("absent disable");
            owner.enable(&executable).expect("enable");
            owner.enable(&executable).expect("repeat enable");
            assert!(
                owner
                    .status(&executable)
                    .expect("status")
                    .contains("enabled")
            );
            let content = fs::read_to_string(owner.registration()).expect("registration");
            assert!(content.contains(FOREGROUND_ARGUMENT));
            assert!(!content.contains("--internal-daemon\""));
            if manager == Manager::Systemd {
                assert!(content.contains("Restart=no"));
                assert!(content.contains("%%$$"));
                fs::remove_file(owner.link()).expect("simulate incomplete registration");
                assert!(owner.status(&executable).is_err());
                owner.enable(&executable).expect("repair");
            } else {
                assert!(content.contains("<key>KeepAlive</key><false/>"));
                assert!(content.contains("&amp; &quot;"));
            }
            fs::write(&executable, "updated binary at stable path").expect("update");
            assert!(
                owner
                    .status(&executable)
                    .expect("update preserves registration")
                    .contains("enabled")
            );
            owner.disable().expect("disable");
            owner.disable().expect("repeat disable");
            assert!(!owner.registration().exists());
            assert!(executable.exists());
            fs::write(owner.registration(), "foreign data").expect("foreign file");
            assert!(owner.enable(&executable).is_err());
            assert!(owner.disable().is_err());
            assert_eq!(
                fs::read_to_string(owner.registration()).expect("foreign file"),
                "foreign data"
            );
        }
    }

    #[test]
    fn symlinks_and_writable_directories_are_rejected() {
        let temp = tempfile::tempdir().expect("fixture");
        let owner = Autostart {
            home: temp.path().into(),
            uid: nix::unistd::Uid::effective().as_raw(),
            manager: Manager::LaunchAgent,
        };
        std::os::unix::fs::symlink(temp.path(), temp.path().join("Library")).expect("symlink");
        assert!(owner.disable().is_err());
        assert!(owner.status(Path::new("/missing")).is_err());
    }
}
