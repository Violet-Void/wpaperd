use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};
use xdg::{BaseDirectories, BaseDirectoriesError};

#[derive(Serialize, Deserialize)]
pub enum IpcMessage {
    StartScene {
        path: PathBuf,
        reduced_motion: bool,
    },
    ShuffleScene,
    PreviousScene,
    FreezeScene,
    StopScene,
    SceneStatus,
    CurrentWallpaper {
        monitor: String,
    },
    NextWallpaper {
        monitors: Vec<String>,
    },
    PreviousWallpaper {
        monitors: Vec<String>,
    },
    PauseWallpaper {
        monitors: Vec<String>,
    },
    ResumeWallpaper {
        monitors: Vec<String>,
    },
    TogglePauseWallpaper {
        monitors: Vec<String>,
    },
    AllWallpapers,
    ReloadWallpaper {
        monitors: Vec<String>,
    },
    GetStatus {
        monitors: Vec<String>,
    },
    SetWallpaper {
        path: PathBuf,
        monitors: Vec<String>,
    },
}

#[derive(Serialize, Deserialize)]
pub enum IpcResponse {
    SceneStatus(MotionStatus),
    CurrentWallpaper {
        path: PathBuf,
    },
    AllWallpapers {
        entries: Vec<(String, PathBuf)>,
    },
    DisplaysStatus {
        entries: Vec<(String, String, Option<Duration>)>,
    },
    Ok,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MotionStatus {
    pub active: bool,
    pub scene_id: Option<String>,
    pub scene_sha256: Option<String>,
    pub arrangements_sha256: Option<String>,
    pub arrangement: Option<String>,
    pub state: String,
    pub reduced_motion: bool,
    pub history_depth: usize,
    pub outputs: BTreeMap<String, String>,
    pub error: Option<String>,
}
impl Default for MotionStatus {
    fn default() -> Self {
        Self {
            active: false,
            scene_id: None,
            scene_sha256: None,
            arrangements_sha256: None,
            arrangement: None,
            state: "stopped".into(),
            reduced_motion: false,
            history_depth: 0,
            outputs: BTreeMap::new(),
            error: None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum IpcError {
    MonitorNotFound {
        monitor: String,
    },
    DrawErrors(Vec<(String, String)>),
    /// Validation error for invalid input (e.g., file not found, not an image)
    ValidationError(String),
}

pub fn socket_path() -> Result<PathBuf, BaseDirectoriesError> {
    let xdg_dirs = BaseDirectories::with_prefix("wpaperd");
    Ok(xdg_dirs.get_runtime_directory()?.join("wpaperd.sock"))
}
