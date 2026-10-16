mod opts;

use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Component, PathBuf},
    time::Duration,
};

use clap::Parser;
use serde::Serialize;
use wpaperd_ipc::{socket_path, IpcError, IpcMessage, IpcResponse};

use crate::opts::{Opts, SceneCommand, SubCmd};

fn unquote(s: String) -> String {
    if s.starts_with('"') && s.ends_with('"') {
        s.trim_start_matches('"').trim_end_matches('"').to_string()
    } else {
        s
    }
}

/// Folds `.` and `..` lexically so the daemon receives only the normal components its package
/// loader accepts. A `..` never folds across a symlink: the kernel would resolve it differently,
/// and folding would skip the loader's no-follow check on that component. `fs::canonicalize` is
/// not used because it resolves the symlinks the loader rejects.
fn lexical_scene_path(path: PathBuf) -> Result<PathBuf, String> {
    let mut folded = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match folded.components().next_back() {
                Some(Component::Normal(_)) => {
                    let metadata = std::fs::symlink_metadata(&folded)
                        .map_err(|error| format!("{}: {error}", folded.display()))?;
                    if metadata.file_type().is_symlink() {
                        return Err(format!(
                            "{} is a symlink followed by `..`",
                            folded.display()
                        ));
                    }
                    folded.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                Some(Component::ParentDir | Component::CurDir) | None => {
                    folded.push(Component::ParentDir)
                }
            },
            other => folded.push(other),
        }
    }
    Ok(folded)
}

fn main() {
    let args = Opts::parse();

    let mut json_resp = false;

    let msg = match args.subcmd {
        SubCmd::Scene { command } => match command {
            SceneCommand::Start {
                path,
                reduced_motion,
            } => IpcMessage::StartScene {
                path: lexical_scene_path(if path.is_absolute() {
                    path
                } else {
                    std::env::current_dir()
                        .unwrap_or_else(|error| {
                            eprintln!("Cannot resolve relative scene path: {error}");
                            std::process::exit(1);
                        })
                        .join(path)
                })
                .unwrap_or_else(|error| {
                    eprintln!("Cannot resolve scene path: {error}");
                    std::process::exit(1);
                }),
                reduced_motion,
            },
            SceneCommand::Shuffle => IpcMessage::ShuffleScene,
            SceneCommand::Previous => IpcMessage::PreviousScene,
            SceneCommand::Freeze => IpcMessage::FreezeScene,
            SceneCommand::Stop => IpcMessage::StopScene,
            SceneCommand::Status { json } => {
                json_resp = json;
                IpcMessage::SceneStatus
            }
        },
        SubCmd::GetWallpaper { monitor } => IpcMessage::CurrentWallpaper {
            monitor: unquote(monitor),
        },
        SubCmd::AllWallpapers { json } => {
            json_resp = json;
            IpcMessage::AllWallpapers
        }
        SubCmd::NextWallpaper { monitors } => IpcMessage::NextWallpaper {
            monitors: monitors.into_iter().map(unquote).collect(),
        },
        SubCmd::PreviousWallpaper { monitors } => IpcMessage::PreviousWallpaper {
            monitors: monitors.into_iter().map(unquote).collect(),
        },
        SubCmd::ReloadWallpaper { monitors } => IpcMessage::ReloadWallpaper {
            monitors: monitors.into_iter().map(unquote).collect(),
        },
        SubCmd::PauseWallpaper { monitors } => IpcMessage::PauseWallpaper {
            monitors: monitors.into_iter().map(unquote).collect(),
        },
        SubCmd::ResumeWallpaper { monitors } => IpcMessage::ResumeWallpaper {
            monitors: monitors.into_iter().map(unquote).collect(),
        },
        SubCmd::TogglePauseWallpaper { monitors } => IpcMessage::TogglePauseWallpaper {
            monitors: monitors.into_iter().map(unquote).collect(),
        },
        SubCmd::GetStatus { json, monitors } => {
            json_resp = json;
            IpcMessage::GetStatus {
                monitors: monitors.into_iter().map(unquote).collect(),
            }
        }
        SubCmd::SetWallpaper { path, monitors } => {
            let path = path.canonicalize().unwrap_or_else(|e| {
                eprintln!("Invalid path '{}': {e}", path.display());
                std::process::exit(1);
            });
            IpcMessage::SetWallpaper {
                path,
                monitors: monitors.into_iter().map(unquote).collect(),
            }
        }
    };

    let socket = socket_path().unwrap();
    let mut conn = UnixStream::connect(&socket).unwrap_or_else(|e| {
        eprintln!(
            "Cannot connect to wpaperd at {}: {e}. Is wpaperd running?",
            socket.display()
        );
        std::process::exit(1);
    });
    conn.write_all(&serde_json::to_vec(&msg).unwrap()).unwrap();
    let mut buf = String::new();
    conn.read_to_string(&mut buf).unwrap();
    if buf.is_empty() {
        // An older or upstream daemon drops requests it cannot parse, such as `scene`.
        eprintln!(
            "wpaperd closed the connection without a reply. The running daemon is probably a \
             different build that does not support this request; restart it from the same \
             install as this wpaperctl."
        );
        std::process::exit(1);
    }
    let res: Result<IpcResponse, IpcError> = serde_json::from_str(&buf).unwrap_or_else(|e| {
        eprintln!("wpaperd returned an invalid reply: {e}");
        std::process::exit(1);
    });
    match res {
        Ok(resp) => match resp {
            IpcResponse::SceneStatus(status) => {
                if json_resp {
                    println!("{}", serde_json::to_string(&status).unwrap());
                } else {
                    println!(
                        "{}{}{}",
                        status.state,
                        status
                            .arrangement
                            .map(|a| format!(" ({a})"))
                            .unwrap_or_default(),
                        status.error.map(|e| format!(": {e}")).unwrap_or_default()
                    );
                    for (output, state) in status.outputs {
                        println!("{output}: {state}");
                    }
                }
            }
            IpcResponse::CurrentWallpaper { path } => println!("{}", path.to_string_lossy()),
            IpcResponse::AllWallpapers { entries: paths } => {
                if json_resp {
                    #[derive(Serialize)]
                    struct Item {
                        display: String,
                        path: PathBuf,
                    }
                    let val = paths
                        .into_iter()
                        .map(|(name, path)| Item {
                            display: name,
                            path,
                        })
                        .collect::<Vec<_>>();
                    println!(
                        "{}",
                        serde_json::to_string(&val).expect("json encoding to work")
                    );
                } else {
                    for (monitor, path) in paths {
                        println!("{monitor}: {}", path.to_string_lossy());
                    }
                }
            }
            IpcResponse::DisplaysStatus { entries } => {
                /// Clean up the duration for human readability
                /// remove the milliseconds and the leading 0s
                fn clean_duration(duration: Duration) -> Duration {
                    let duration = duration.as_secs();
                    Duration::from_secs(if duration < 60 {
                        duration
                    } else if duration < 60 * 60 {
                        // if the duration is in minutes, remove the seconds
                        duration - duration % 60
                        // duration is in hours, remove the minutes and seconds
                    } else {
                        duration - duration % (60 * 60)
                    })
                }
                if json_resp {
                    #[derive(Serialize)]
                    struct Item {
                        display: String,
                        status: String,
                        #[serde(rename = "duration_left", with = "humantime_serde")]
                        duration_left: Option<Duration>,
                    }
                    let val = entries
                        .into_iter()
                        .map(|(display, status, duration_left)| Item {
                            display,
                            status,
                            duration_left: duration_left.map(clean_duration),
                        })
                        .collect::<Vec<_>>();
                    println!(
                        "{}",
                        serde_json::to_string(&val).expect("json encoding to work")
                    );
                } else {
                    for (monitor, status, duration_left) in entries {
                        println!(
                            "{monitor}: {status}{}",
                            if let Some(d) = duration_left {
                                format!(" ({} left)", humantime::format_duration(clean_duration(d)))
                            } else {
                                "".to_string()
                            }
                        );
                    }
                }
            }
            IpcResponse::Ok => (),
        },
        Err(err) => {
            match err {
                IpcError::MonitorNotFound { monitor } => {
                    eprintln!("monitor {monitor} could not be found")
                }
                IpcError::DrawErrors(errors) => {
                    for (monitor, err) in errors {
                        eprintln!("Wallpaper could not be drawn for monitor {monitor}: {err}")
                    }
                }
                IpcError::ValidationError(msg) => {
                    eprintln!("{msg}")
                }
            }
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::lexical_scene_path;
    use std::path::PathBuf;

    fn temp_base(label: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "wpaperctl-scene-path-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&base).unwrap();
        base
    }

    #[test]
    fn scene_paths_fold_dot_components() {
        for (input, expected) in [("/a/./b", "/a/b"), ("/..", "/")] {
            assert_eq!(
                lexical_scene_path(PathBuf::from(input)),
                Ok(PathBuf::from(expected))
            );
        }
        // Each popped component is lstat'd, so `/a/b/../c` folds only when `/a/b` exists.
        assert!(lexical_scene_path(PathBuf::from("/a/b/../c")).is_err());
        let base = temp_base("fold");
        std::fs::create_dir_all(base.join("a/b")).unwrap();
        let folded = lexical_scene_path(base.join("a/b/../c"));
        std::fs::remove_dir_all(&base).unwrap();
        assert_eq!(folded, Ok(base.join("a/c")));
    }

    #[test]
    fn scene_paths_do_not_fold_parent_across_symlinks() {
        let base = temp_base("symlink");
        std::fs::create_dir(base.join("real")).unwrap();
        std::os::unix::fs::symlink(base.join("real"), base.join("link")).unwrap();
        let through_link = lexical_scene_path(base.join("link/../real"));
        let through_dir = lexical_scene_path(base.join("real/../real"));
        std::fs::remove_dir_all(&base).unwrap();
        assert!(through_link.is_err());
        assert_eq!(through_dir, Ok(base.join("real")));
    }
}
