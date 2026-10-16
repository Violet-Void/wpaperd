use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use color_eyre::{
    eyre::{ensure, eyre, WrapErr},
    owo_colors::OwoColorize,
    Result, Section,
};
use dirs::home_dir;
use log::{error, warn};
use notify_debouncer_full::{
    new_debouncer,
    notify::{
        event::{AccessKind, AccessMode},
        Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    },
    DebounceEventResult, Debouncer, FileIdMap,
};
use serde::Deserialize;
use smithay_client_toolkit::reexports::calloop::ping::Ping;

use crate::{
    image_picker::ImagePicker,
    render::Transition,
    wallpaper_info::{BackgroundMode, Recursive, Sorting, WallpaperInfo},
};

use std::os::unix::fs::PermissionsExt;

#[derive(Default, Deserialize, PartialEq, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct SerializedWallpaperInfo {
    #[serde(default, deserialize_with = "tilde_expansion_deserialize")]
    pub path: Option<PathBuf>,
    #[serde(default, with = "humantime_serde")]
    pub duration: Option<Duration>,
    #[serde(rename = "apply-shadow")]
    pub apply_shadow: Option<bool>,
    pub sorting: Option<Sorting>,
    pub mode: Option<BackgroundMode>,
    #[serde(rename = "queue-size")]
    pub queue_size: Option<usize>,
    #[serde(rename = "transition-time")]
    pub transition_time: Option<u32>,

    /// Determines if we should show the transition between black and first
    /// wallpaper. `Some(false)` means we instantly cut to the first wallpaper,
    /// `Some(true)` means we fade from black to the first wallpaper.
    ///
    /// See [crate::wallpaper_info::WallpaperInfo]
    #[serde(rename = "initial-transition")]
    pub initial_transition: Option<bool>,
    pub transition: Option<Transition>,

    /// Determine the offset for the wallpaper to be drawn into the screen
    /// Must be from 0.0 to 1.0, by default is 0.0 in tile mode and 0.5 in all the others
    ///
    /// See [crate::wallpaper_info::WallpaperInfo]
    pub offset: Option<f32>,

    /// Assign these displays to a group that shows the same wallpaper
    pub group: Option<u8>,

    /// Recursively traverse the directory set as path
    /// Set as true by default
    pub recursive: Option<bool>,

    // Path to bash script.
    #[serde(default, deserialize_with = "tilde_expansion_deserialize")]
    pub exec: Option<PathBuf>,

    /// Path for a symlink pointing to the current wallpaper.
    /// Relative paths are resolved under $XDG_STATE_HOME/wpaperd/wallpapers/.
    #[serde(default, deserialize_with = "tilde_expansion_deserialize")]
    pub symlink: Option<PathBuf>,
}

impl SerializedWallpaperInfo {
    pub fn apply_and_validate(&self, default: &Self) -> Result<WallpaperInfo> {
        let mut path_inherited = false;
        let path = match (&self.path, &default.path) {
            (Some(path), None) | (Some(path), Some(_)) => path.to_path_buf(),
            (None, Some(path)) => {
                path_inherited = true;
                path.to_path_buf()
            }
            (None, None) => xdg::BaseDirectories::with_prefix("wpaperd")
                .create_data_directory("wallpapers")
                .wrap_err(
                    "Failed to create default wallpaper directory $XDG_DATA_HOME/wpaperd/wallpapers",
                )?
                .to_path_buf(),
        };
        // Ensure that a path exists
        if !path.exists() {
            return Err(eyre!(
                "Path {} for attribute {}{} must exist",
                path.to_string_lossy().italic().yellow(),
                "path".bold().italic().blue(),
                if path_inherited {
                    format!(
                        " (inherited from {} configuration)",
                        "default".magenta().bold()
                    )
                } else {
                    "".to_string()
                }
            ))
            .with_suggestion(|| {
                format!(
                    "Set attribute {} to an existing file or directory",
                    "path".bold().italic().blue(),
                )
            });
        }

        let duration = match (&self.duration, &default.duration) {
            // duration is inherited from default, but this section set path to a file, ignore
            // duration
            (None, Some(_)) if path.is_file() && !path_inherited => None,
            (Some(duration), _) | (None, Some(duration)) => Some(*duration),
            (None, None) => None,
        };
        // duration can only be set when path is a directory
        if duration.is_some() && !path.is_dir() {
            // Do no use bail! to add suggestion
            return Err(eyre!(
                "{} cannot be set when {} points to a file",
                "duration".bold().italic().blue(),
                "path".bold().italic().blue(),
            )
            .with_suggestion(|| {
                format!(
                    "Either remove {} or set {} to a directory",
                    "path".bold().italic().blue(),
                    "duration".bold().italic().blue()
                )
            }));
        }

        let sorting = match (&self.sorting, &default.sorting) {
            (None, Some(_)) if path.is_file() && !path_inherited => None,
            (Some(sorting), _) | (None, Some(sorting)) => Some(*sorting),
            (None, None) => None,
        };

        let group = match (&self.group, &default.group) {
            (None, Some(_)) if path.is_file() && !path_inherited => None,
            (Some(sorting), _) | (None, Some(sorting)) => Some(*sorting),
            (None, None) => None,
        };

        // sorting and group can only be set when path is a directory
        if (sorting.is_some() || group.is_some()) && !path.is_dir() {
            // Do no use bail! to add suggestion
            let attr = if sorting.is_some() {
                "sorting"
            } else {
                "group"
            };
            return Err(eyre!(
                "{} cannot be set when {} points to a file",
                attr.bold().italic().blue(),
                "path".bold().italic().blue(),
            )
            .with_suggestion(|| {
                format!(
                    "Either remove {} or set {} to a directory",
                    attr.bold().italic().blue(),
                    "path".bold().italic().blue(),
                )
            }));
        }

        // If there is no sorting but the group is set
        let sorting = if group.is_some() && sorting.is_none() {
            // Assign it the default one, so that we can do the group match below
            Some(Sorting::default())
        } else {
            sorting
        };
        let sorting = if let Some(group) = group {
            match sorting {
                None | Some(Sorting::Random) => Some(Sorting::GroupedRandom { group }),
                Some(Sorting::Ascending) | Some(Sorting::Descending) => {
                    return Err(eyre!(
                        "{} cannot be combined with {} other than {}",
                        "group".bold().italic().blue(),
                        "sorting".bold().italic().blue(),
                        "random".bold().italic().green(),
                    ));
                }
                Some(Sorting::GroupedRandom { group: _ }) => unreachable!(),
            }
        } else {
            sorting
        };

        let mode = match (&self.mode, &default.mode) {
            (Some(mode), _) | (None, Some(mode)) => *mode,
            (None, None) => BackgroundMode::default(),
        };
        let drawn_images_queue_size = match (&self.queue_size, &default.queue_size) {
            (Some(size), _) | (None, Some(size)) => *size,
            (None, None) => ImagePicker::DEFAULT_DRAWN_IMAGES_QUEUE_SIZE,
        };
        let initial_transition = match (&self.initial_transition, &default.initial_transition) {
            (Some(initial_transition), _) | (None, Some(initial_transition)) => *initial_transition,
            (None, None) => false,
        };

        let transition = match (&self.transition, &default.transition) {
            (Some(transition), _) | (None, Some(transition)) => transition.clone(),
            (None, None) => Transition::Fade {},
        };

        let transition_time = match (&self.transition_time, &default.transition_time) {
            (Some(transition_time), _) | (None, Some(transition_time)) => *transition_time,
            (None, None) => transition.default_transition_time(),
        };

        let offset = match (&self.offset, &default.offset) {
            (Some(offset), _) | (None, Some(offset)) => Some(*offset),
            (None, None) => None,
        };

        let recursive = match (&self.recursive, &default.recursive) {
            (Some(recursive), _) | (None, Some(recursive)) => {
                Some(std::convert::Into::<Recursive>::into(*recursive))
            }
            (None, None) => None,
        };

        let exec = match (&self.exec, &default.exec) {
            (Some(exec), _) | (None, Some(exec)) => Some(exec.to_path_buf()),
            (None, None) => None,
        };

        let symlink = match (&self.symlink, &default.symlink) {
            (Some(link), _) | (None, Some(link)) => Some(link.to_path_buf()),
            (None, None) => None,
        };

        if let Some(exec_path) = &exec {
            ensure!(
                exec_path.exists(),
                "Exec script {} must exist",
                exec_path.to_string_lossy().italic().yellow()
            );
            ensure!(
                exec_path.is_file(),
                "Exec path {} must be a file",
                exec_path.to_string_lossy().italic().yellow()
            );
            ensure!(
                std::fs::metadata(exec_path)?.permissions().mode() & 0o111 != 0,
                "Exec script {} must be executable",
                exec_path.to_string_lossy().italic().yellow()
            );
        }

        Ok(WallpaperInfo {
            path,
            duration,
            apply_shadow: false,
            sorting,
            mode,
            drawn_images_queue_size,
            transition_time,
            initial_transition,
            transition,
            offset,
            recursive,
            exec,
            symlink,
        })
    }
}

#[derive(Deserialize, Default)]
pub struct Config {
    #[serde(flatten)]
    data: HashMap<String, SerializedWallpaperInfo>,
    #[serde(skip)]
    default: SerializedWallpaperInfo,
    #[serde(skip)]
    any: SerializedWallpaperInfo,
    #[serde(skip)]
    pub path: PathBuf,
}

impl Config {
    pub fn new_from_path(path: &Path) -> Result<Self> {
        ensure!(path.exists(), "File {path:?} does not exist");
        let mut config: Self = toml::from_str(&fs::read_to_string(path)?)?;
        config
            .data
            .get("default")
            .unwrap_or(&SerializedWallpaperInfo::default())
            .clone_into(&mut config.default);
        config
            .data
            .get("any")
            .unwrap_or(&SerializedWallpaperInfo::default())
            .clone_into(&mut config.any);
        config.data.retain(|name, info| {
            // The default configuration does not follow these rules
            // We still need the default configuration here because the path needs to be cached
            if info == &config.default {
                true
            } else {
                match info.apply_and_validate(&config.default).wrap_err_with(|| {
                    format!(
                        "Failed to validate configuration for display {}",
                        name.bold().magenta()
                    )
                }) {
                    Ok(_) => true,
                    Err(err) => {
                        // We do not want to exit when error occurs, print it and go forward
                        warn!("{err:?}");
                        false
                    }
                }
            }
        });

        let groups = config
            .data
            .iter()
            .filter_map(|(name, info)| {
                // It's a bit overkill to call this function again, but the displays are on average 1 or 2
                // and the code is simple enough that it wouldn't make a difference with 10 either
                info.apply_and_validate(&config.default)
                    .map(|res| (name, res))
                    .ok()
            })
            .filter(|(_, info)| matches!(info.sorting, Some(Sorting::GroupedRandom { .. })))
            .collect::<Vec<_>>();

        // Check if all the groups share the same path
        // This check is only useful when there is more than one display
        // We display a warning related to the config issue, but we are not fixing anything here
        // The path that will be used is probably not what the user expect, but proving an "expected"
        // behaviour for a configuration issue seems overkill
        if groups.len() > 1 {
            // We want to skip displays for which we already displayed the warning
            let mut errored_list = Vec::new();
            for i in 0..groups.len() {
                let x = groups.get(i).unwrap();
                for j in (i + 1)..groups.len() {
                    if errored_list.contains(&j) {
                        continue;
                    }
                    let y = groups.get(j).unwrap();
                    if !(x.1.sorting.is_none()
                        || x.1.sorting != y.1.sorting
                        || x.1.path == y.1.path)
                    {
                        warn!(
                            "Displays {} and {} are assigned to group {} but have different paths",
                            x.0,
                            y.0,
                            match x.1.sorting {
                                Some(Sorting::GroupedRandom { group }) => group,
                                _ => unreachable!(),
                            }
                        );
                        errored_list.push(j);
                    }
                }
            }
        }

        // Remove current-image-link if it has no template variables in a section that can match
        // multiple monitors — without %PORT% or %NAME%, all matching monitors would write to
        // the same symlink path.
        config
            .data
            .iter_mut()
            .filter(|(key, info)| {
                (*key == "default" || *key == "any" || key.starts_with("re:"))
                    && info
                        .symlink
                        .as_deref()
                        .is_some_and(|p| !has_template_vars(p))
            })
            .for_each(|(key, info)| {
                error!(
                    "`symlink` in [{key}] has no template variables (%PORT% or \
                     %NAME%) but the section can match multiple monitors, ignoring it"
                );
                info.symlink = None;
            });

        config.path = path.to_path_buf();
        Ok(config)
    }

    pub fn get_info_for_output(&self, name: &str, description: &str) -> Result<WallpaperInfo> {
        use regex::Regex;
        // Actually, wayland may report an output description in different
        // formats as the compositors wants to. For example, niri reports
        // description in format `<vendor name> - <monitor name> - <port name>`
        // so, we can not handle here the all formats at once. But users
        // can use a regex to match theirs' compositors outputs.
        let mut matched = None;
        for (k, v) in self.data.iter() {
            if !k.starts_with("re:") {
                continue;
            }
            // TODO(Shvedov): Better to compile re withing creation of
            // WallpaperInfo, but adding field of type `Option<Regex>` breaks
            // PatialEq macro.
            let re = match Regex::new(&k[3..]) {
                Ok(re) => re,
                Err(err) => {
                    warn!("Invalid regex in config key {k:?}: {err}");
                    continue;
                }
            };
            if re.is_match(description) || re.is_match(name) {
                matched = Some(v);
                break;
            };
        }

        if let Some(info) = matched {
            info.apply_and_validate(&self.default)
        } else {
            self.data
                .get(clean_monitor_description(description))
                .or_else(|| self.data.get(name))
                .unwrap_or(&self.any)
                .apply_and_validate(&self.default)
        }
    }

    pub fn paths(&self) -> Vec<(PathBuf, Recursive)> {
        let mut paths: Vec<_> = self
            .data
            .values()
            .filter_map(|info| {
                info.path.as_ref().map(|p| {
                    (
                        p.to_path_buf(),
                        info.recursive
                            .or(self.default.recursive)
                            .map(Recursive::from)
                            .unwrap_or_default(),
                    )
                })
            })
            .collect();
        paths.sort_unstable();
        paths.dedup();
        paths
    }

    /// Return true if the struct changed
    pub fn update(&mut self) -> bool {
        // When the config file has been written into
        let new_config = Config::new_from_path(&self.path).wrap_err_with(|| {
            format!(
                "Failed to read the new configuration from {}",
                self.path.to_string_lossy()
            )
        });
        match new_config {
            Ok(new_config) if new_config != *self => {
                *self = new_config;
                true
            }
            Ok(_) => {
                // Do nothing, the new config is the same as the loaded one
                false
            }
            Err(err) => {
                error!("{err:?}");
                false
            }
        }
    }
}

impl PartialEq for Config {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
    }
}

/// Debounce window for config file events, matching the previous hotwatch default.
const CONFIG_DEBOUNCE: Duration = Duration::from_secs(2);

/// Watch the configuration file through its parent directories.
///
/// An inotify watch on the file itself follows the inode: it is lost when an editor or
/// Stow replaces the file by rename, and it cannot see the file being recreated. Instead,
/// non-recursively watch the directory of the configured path and the directory of its
/// resolved symlink target, and only react to events naming one of those two files.
/// The watches are re-armed after every change so that retargeted symlinks are followed.
pub struct ConfigWatcher {
    path: PathBuf,
    debouncer: Debouncer<RecommendedWatcher, FileIdMap>,
    watched_dirs: Vec<PathBuf>,
    resolved_target: Option<PathBuf>,
    targets: Arc<Mutex<Vec<PathBuf>>>,
    changed: Arc<AtomicBool>,
}

impl ConfigWatcher {
    pub fn new(path: &Path, ping: Ping) -> Result<Self> {
        Self::with_debounce(path, ping, CONFIG_DEBOUNCE)
    }

    fn with_debounce(path: &Path, ping: Ping, debounce: Duration) -> Result<Self> {
        let changed = Arc::new(AtomicBool::new(false));
        let targets = Arc::new(Mutex::new(Vec::new()));
        let handler = {
            let changed = changed.clone();
            let targets = targets.clone();
            move |result: DebounceEventResult| match result {
                Ok(events) => {
                    let targets = targets.lock().expect("config watcher mutex poisoned");
                    if events.iter().any(|event| is_config_event(event, &targets)) {
                        changed.store(true, Ordering::Release);
                        ping.ping();
                    }
                }
                Err(errors) => errors
                    .iter()
                    .for_each(|err| warn!("Error while watching the configuration: {err}")),
            }
        };
        let debouncer = new_debouncer(debounce, None, handler)
            .wrap_err("Failed to create the configuration file watcher")?;
        let mut watcher = Self {
            path: path.to_path_buf(),
            debouncer,
            watched_dirs: Vec::new(),
            resolved_target: None,
            targets,
            changed,
        };
        watcher.rearm()?;
        Ok(watcher)
    }

    /// Return true if the configuration changed since the last call.
    ///
    /// Consuming the change before the caller reads the file means a save that lands
    /// during the read is reported again instead of being lost.
    pub fn take_change(&mut self) -> bool {
        if !self.changed.swap(false, Ordering::AcqRel) {
            return false;
        }
        if let Err(err) = self.rearm() {
            error!("{err:?}");
        }
        true
    }

    fn rearm(&mut self) -> Result<()> {
        let file_name = self
            .path
            .file_name()
            .ok_or_else(|| eyre!("Configuration path {:?} has no file name", self.path))?;
        let parent = match self.path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        let parent = parent.canonicalize().wrap_err_with(|| {
            format!("Failed to resolve the configuration directory {parent:?}")
        })?;
        // Keep the last resolved target while the file (or the symlink target) is missing,
        // so that recreating it is still observed.
        if let Ok(resolved) = self.path.canonicalize() {
            self.resolved_target = Some(resolved);
        }
        let mut targets = vec![parent.join(file_name)];
        targets.extend(self.resolved_target.clone());
        targets.dedup();
        let mut dirs: Vec<PathBuf> = targets
            .iter()
            .filter_map(|target| target.parent().map(Path::to_path_buf))
            .collect();
        dirs.sort_unstable();
        dirs.dedup();

        *self.targets.lock().expect("config watcher mutex poisoned") = targets;

        for dir in &dirs {
            if self.watched_dirs.contains(dir) {
                continue;
            }
            self.debouncer
                .watcher()
                .watch(dir, RecursiveMode::NonRecursive)
                .wrap_err_with(|| format!("Failed to watch file changes in {dir:?}"))?;
            self.debouncer
                .cache()
                .add_root(dir, RecursiveMode::NonRecursive);
            self.watched_dirs.push(dir.clone());
        }
        let (keep, stale): (Vec<_>, Vec<_>) = std::mem::take(&mut self.watched_dirs)
            .into_iter()
            .partition(|dir| dirs.contains(dir));
        self.watched_dirs = keep;
        for dir in stale {
            if let Err(err) = self.debouncer.watcher().unwatch(&dir) {
                warn!("Failed to stop watching {dir:?}: {err}");
            }
            self.debouncer.cache().remove_root(&dir);
        }
        Ok(())
    }
}

/// Opening or reading the configuration must not schedule another reload.
fn is_config_event(event: &Event, targets: &[PathBuf]) -> bool {
    let relevant_kind = match event.kind {
        EventKind::Access(kind) => kind == AccessKind::Close(AccessMode::Write),
        _ => true,
    };
    relevant_kind && event.paths.iter().any(|path| targets.contains(path))
}

pub fn tilde_expansion_deserialize<'de, D>(deserializer: D) -> Result<Option<PathBuf>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let path = String::deserialize(deserializer)?;
    let path = Path::new(&path);

    Ok(Some(
        path.strip_prefix("~")
            .map_or(path.to_path_buf(), |p| home_dir().unwrap().join(p)),
    ))
}

/// Return true if `path` contains any template variable (`%PORT%` or `%NAME%`).
pub fn has_template_vars(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("%PORT%") || s.contains("%NAME%")
}

/// Clean a monitor description so that the value reported by Wayland matches the one reported by
/// Sway/Hyprland.
///
/// Wayland may report an output description that includes information about the port and port type.
/// This information is *not* reported by Sway or Hyprland so we need to strip it off so outputs are
/// matched the way users expect.
///
/// Some compositors (e.g. niri) use a ` - PORT` suffix instead of ` (PORT)`.
fn clean_monitor_description(desc: &str) -> &str {
    // Handle "(port)" suffix: "ASUS PB258 G1LMTJ002598 (DP-10)" → "ASUS PB258 G1LMTJ002598"
    let desc = desc
        .split_once(" (")
        .map(|(s, _)| s)
        .unwrap_or(desc)
        // Some monitors descriptions contain more than a single space before the port information.
        // See <https://github.com/danyspin97/wpaperd/issues/113>.
        .trim_end();
    // Handle " - PORT" suffix used by niri:
    // "Dell Inc. - DELL P2723DE - DP-4" → "Dell Inc. - DELL P2723DE"
    if let Some((before, port)) = desc.rsplit_once(" - ") {
        if is_port_name(port) {
            return before;
        }
    }
    desc
}

/// Return true if `s` looks like a Wayland output port name (e.g. `DP-1`, `HDMI-A-2`, `eDP-1`).
fn is_port_name(s: &str) -> bool {
    const PORT_PREFIXES: &[&str] = &["DP-", "HDMI-", "eDP-", "DVI-", "VGA-", "LVDS-", "DSI-"];
    PORT_PREFIXES.iter().any(|p| s.starts_with(p))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_clean_monitor_description() {
        assert_eq!(
            clean_monitor_description("Microstep MPG321UX OLED 0x01010101"),
            "Microstep MPG321UX OLED 0x01010101"
        );
        assert_eq!(
            clean_monitor_description("Ancor Communications Inc ASUS PB258 GBLMTJ022271 (DP-9)"),
            "Ancor Communications Inc ASUS PB258 GBLMTJ022271"
        );
        assert_eq!(
            clean_monitor_description("Ancor Communications Inc ASUS PB258 G1LMTJ002598 (DP-10)"),
            "Ancor Communications Inc ASUS PB258 G1LMTJ002598"
        );
        assert_eq!(
            clean_monitor_description("BOE 0x095F  (eDP-1)"),
            "BOE 0x095F"
        );
        assert_eq!(
            clean_monitor_description("GIGA-BYTE TECHNOLOGY CO. LTD. G34WQC A  (DP-8)"),
            "GIGA-BYTE TECHNOLOGY CO. LTD. G34WQC A"
        );
        // niri reports descriptions with " - PORT" suffix instead of " (PORT)"
        assert_eq!(
            clean_monitor_description("Dell Inc. - DELL P2723DE - DP-4"),
            "Dell Inc. - DELL P2723DE"
        );
        assert_eq!(
            clean_monitor_description("ASUSTek COMPUTER INC - ROG XG27AQ - DP-1"),
            "ASUSTek COMPUTER INC - ROG XG27AQ"
        );
        assert_eq!(
            clean_monitor_description("Dell Inc. - DELL P2723DE - HDMI-A-1"),
            "Dell Inc. - DELL P2723DE"
        );
    }
}

#[cfg(test)]
mod reload_tests {
    //! Real-filesystem regressions for config reloads (Violet-Void/wpaperd#3).

    use std::{
        fs::{self, OpenOptions},
        io::{Seek, SeekFrom, Write},
        os::unix::fs::symlink,
        thread::sleep,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use smithay_client_toolkit::reexports::calloop::ping::{make_ping, PingSource};

    use super::*;

    const DEBOUNCE: Duration = Duration::from_millis(100);
    const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "wpaperd-reload-{name}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct Watch {
        watcher: ConfigWatcher,
        _ping_source: PingSource,
    }

    impl Watch {
        fn start(config: &mut Config) -> Self {
            let (ping, ping_source) = make_ping().unwrap();
            let watcher = ConfigWatcher::with_debounce(&config.path, ping, DEBOUNCE).unwrap();
            Self {
                watcher,
                _ping_source: ping_source,
            }
        }

        fn take_change(&mut self) -> bool {
            self.watcher.take_change()
        }
    }

    fn content(secs: u64) -> String {
        format!("[default]\nduration = \"{secs}s\"\n")
    }

    fn duration(config: &Config) -> Option<Duration> {
        config.data.get("default").and_then(|info| info.duration)
    }

    fn write_in_place(path: &Path, content: &str) {
        // Mirrors dotfiles config_state.py: rewrite the current inode, then truncate.
        let mut file = OpenOptions::new().write(true).open(path).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        let len = file.stream_position().unwrap();
        file.set_len(len).unwrap();
        file.sync_all().unwrap();
    }

    fn replace_atomically(path: &Path, content: &str) {
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, content).unwrap();
        fs::rename(&tmp, path).unwrap();
    }

    fn wait_for_change(watch: &mut Watch, step: &str) {
        let deadline = Instant::now() + EVENT_TIMEOUT;
        while Instant::now() < deadline {
            if watch.take_change() {
                return;
            }
            sleep(Duration::from_millis(20));
        }
        panic!("no config change observed after: {step}");
    }

    /// The debouncer expires each event on its own timestamp, so one save can arrive as
    /// two batches a tick apart. A duplicate must re-read an identical file and not redraw.
    fn settle(watch: &mut Watch, config: &mut Config, step: &str) {
        sleep(DEBOUNCE);
        if watch.take_change() {
            assert!(!config.update(), "duplicate event redrew after: {step}");
        }
    }

    fn assert_quiet(watch: &mut Watch, step: &str) {
        sleep(DEBOUNCE * 5);
        assert!(
            !watch.take_change(),
            "unexpected config change after: {step}"
        );
    }

    /// Wait for the watcher, reload and check the new value is live.
    fn expect_reload(watch: &mut Watch, config: &mut Config, secs: u64, step: &str) {
        wait_for_change(watch, step);
        assert!(config.update(), "config did not change after: {step}");
        assert_eq!(duration(config), Some(Duration::from_secs(secs)), "{step}");
        settle(watch, config, step);
        assert_quiet(watch, &format!("{step} (self-trigger)"));
    }

    #[test]
    fn reloads_regular_file_across_edits_replacements_and_recreation() {
        let tmp = TempDir::new("regular");
        let path = tmp.0.join("config.toml");
        fs::write(&path, content(1)).unwrap();
        let mut config = Config::new_from_path(&path).unwrap();
        let mut watch = Watch::start(&mut config);

        write_in_place(&path, &content(2));
        expect_reload(&mut watch, &mut config, 2, "in-place edit");

        replace_atomically(&path, &content(3));
        expect_reload(&mut watch, &mut config, 3, "first atomic replace");

        replace_atomically(&path, &content(4));
        expect_reload(&mut watch, &mut config, 4, "second atomic replace");

        write_in_place(&path, &content(5));
        expect_reload(&mut watch, &mut config, 5, "edit after replace");

        // Siblings such as the dotfiles backup must not trigger reloads.
        fs::write(tmp.0.join("config.toml.backup"), content(99)).unwrap();
        assert_quiet(&mut watch, "sibling backup write");

        fs::remove_file(&path).unwrap();
        wait_for_change(&mut watch, "removal");
        assert!(!config.update(), "removal must keep the last valid config");
        assert_eq!(duration(&config), Some(Duration::from_secs(5)));
        settle(&mut watch, &mut config, "removal");

        fs::write(&path, content(6)).unwrap();
        expect_reload(&mut watch, &mut config, 6, "recreation");

        write_in_place(&path, &content(7));
        expect_reload(&mut watch, &mut config, 7, "edit after recreation");
    }

    #[test]
    fn malformed_save_keeps_last_valid_config_until_recovery() {
        let tmp = TempDir::new("malformed");
        let path = tmp.0.join("config.toml");
        fs::write(&path, content(1)).unwrap();
        let mut config = Config::new_from_path(&path).unwrap();
        let mut watch = Watch::start(&mut config);

        write_in_place(&path, "[default\nduration = ");
        wait_for_change(&mut watch, "malformed save");
        assert!(!config.update(), "malformed save must not reload");
        assert_eq!(duration(&config), Some(Duration::from_secs(1)));
        settle(&mut watch, &mut config, "malformed save");
        assert_eq!(duration(&config), Some(Duration::from_secs(1)));
        assert_quiet(&mut watch, "malformed save (self-trigger)");

        write_in_place(&path, &content(1));
        wait_for_change(&mut watch, "identical valid save");
        assert!(!config.update(), "unchanged config must not redraw");
        settle(&mut watch, &mut config, "identical valid save");

        replace_atomically(&path, &content(2));
        expect_reload(&mut watch, &mut config, 2, "valid save after malformed");
    }

    #[test]
    fn reloads_stow_symlink_target_replacement_and_retargeting() {
        let tmp = TempDir::new("stow");
        let conf = tmp.0.join("home/.config/wpaperd");
        let pkg1 = tmp.0.join("dotfiles/wpaperd/.config/wpaperd");
        let pkg2 = tmp.0.join("dotfiles/host/.config/wpaperd");
        for dir in [&conf, &pkg1, &pkg2] {
            fs::create_dir_all(dir).unwrap();
        }
        let link = conf.join("config.toml");
        let target1 = pkg1.join("config.toml");
        let target2 = pkg2.join("config.toml");
        fs::write(&target1, content(1)).unwrap();
        fs::write(&target2, content(20)).unwrap();
        symlink(
            "../../../dotfiles/wpaperd/.config/wpaperd/config.toml",
            &link,
        )
        .unwrap();

        let mut config = Config::new_from_path(&link).unwrap();
        let mut watch = Watch::start(&mut config);

        write_in_place(&target1, &content(2));
        expect_reload(&mut watch, &mut config, 2, "in-place edit of target");

        replace_atomically(&target1, &content(3));
        expect_reload(&mut watch, &mut config, 3, "first target replace");

        replace_atomically(&target1, &content(4));
        expect_reload(&mut watch, &mut config, 4, "second target replace");

        write_in_place(&target1, &content(5));
        expect_reload(&mut watch, &mut config, 5, "edit after target replace");

        // Retarget the link atomically, as `ln -sfn` / Stow restow would.
        let tmp_link = conf.join("config.toml.link-tmp");
        symlink(
            "../../../dotfiles/host/.config/wpaperd/config.toml",
            &tmp_link,
        )
        .unwrap();
        fs::rename(&tmp_link, &link).unwrap();
        expect_reload(&mut watch, &mut config, 20, "symlink retarget");

        write_in_place(&target2, &content(21));
        expect_reload(&mut watch, &mut config, 21, "edit of new target");

        // The old target is no longer the configuration.
        write_in_place(&target1, &content(6));
        assert_quiet(&mut watch, "edit of previous target");
        assert_eq!(duration(&config), Some(Duration::from_secs(21)));
    }
}
