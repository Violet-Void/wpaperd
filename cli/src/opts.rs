use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Opts {
    #[clap(subcommand)]
    pub subcmd: SubCmd,
}

#[derive(clap::Subcommand)]
pub enum SubCmd {
    /// Explicitly opt into the portable native motion-scene backend
    Scene {
        #[clap(subcommand)]
        command: SceneCommand,
    },
    #[clap(visible_alias = "get")]
    GetWallpaper { monitor: String },
    #[clap(visible_alias = "get-all")]
    AllWallpapers {
        #[clap(short, long)]
        json: bool,
    },
    #[clap(visible_alias = "next")]
    NextWallpaper { monitors: Vec<String> },
    #[clap(visible_alias = "previous")]
    PreviousWallpaper { monitors: Vec<String> },
    #[clap(visible_alias = "reload")]
    ReloadWallpaper { monitors: Vec<String> },
    #[clap(visible_alias = "pause")]
    PauseWallpaper { monitors: Vec<String> },
    #[clap(visible_alias = "resume")]
    ResumeWallpaper { monitors: Vec<String> },
    #[clap(visible_alias = "toggle-pause")]
    TogglePauseWallpaper { monitors: Vec<String> },
    #[clap(visible_alias = "status")]
    GetStatus {
        #[clap(short, long)]
        json: bool,
        monitors: Vec<String>,
    },
    /// Set wallpaper to a specific image file
    #[clap(visible_alias = "set")]
    SetWallpaper {
        /// Path to the image file
        path: PathBuf,
        /// Monitors to set wallpaper on (if empty, sets on all monitors)
        monitors: Vec<String>,
    },
}

#[derive(clap::Subcommand)]
pub enum SceneCommand {
    /// Load a package containing scene.json, arrangements.json and outputs.json
    Start {
        path: PathBuf,
        #[clap(long)]
        reduced_motion: bool,
    },
    /// Move deterministically to the next of the three arrangements
    Shuffle,
    /// Return to the preceding requested arrangement (bounded history)
    Previous,
    /// Toggle freeze at the current position; thaw resumes the shared clock
    Freeze,
    /// Release scene textures and restore ordinary still wallpaper operation
    Stop,
    Status {
        #[clap(long)]
        json: bool,
    },
}
