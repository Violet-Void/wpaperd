mod layout;
mod loader;
mod playback;
#[cfg(test)]
mod tests;

use crate::display_info::DisplayInfo;
use color_eyre::Result;
use image::RgbaImage;
pub use playback::Playback;
use serde_json::Value;
use std::{cell::RefCell, collections::BTreeMap, path::Path, rc::Rc, time::Instant};

pub type SharedMotion = Rc<RefCell<Option<Playback>>>;

pub struct Object {
    pub id: String,
    pub asset: String,
    pub size: [u32; 2],
    pub depth: u8,
}
pub enum Background {
    Solid([u8; 4]),
    Asset(String),
}
pub struct Scene {
    pub sha256: String,
    pub manifest: Value,
    pub canvas: [u32; 2],
    pub objects: Vec<Object>,
    pub images: BTreeMap<String, Rc<RgbaImage>>,
    pub background: Background,
}

/// A sampled frame contains one master-space scene and explicit output crop.
pub struct Frame {
    pub scene: Rc<Scene>,
    pub centers: Vec<[f64; 2]>,
    pub crop: [u32; 4],
    pub animating: bool,
}

pub fn load(path: &Path, reduced_motion: bool) -> Result<Playback> {
    let package = loader::Package::open(path)?;
    let scene = Rc::new(loader::load_scene(&package)?);
    Playback::load(&package, scene, reduced_motion)
}

pub fn frame(shared: &SharedMotion, display: &DisplayInfo) -> Result<Option<Frame>> {
    let controller = shared.borrow();
    let Some(playback) = controller.as_ref() else {
        return Ok(None);
    };
    if playback.failure.is_some() {
        return Ok(None);
    }
    let Some(binding) = playback.outputs.iter().find(|o| o.output == display.name) else {
        return Ok(None);
    };
    binding.validate(display)?;
    let now = Instant::now();
    Ok(Some(Frame {
        scene: playback.scene.clone(),
        centers: playback.sample(now),
        crop: binding.crop,
        animating: playback.animating(now),
    }))
}
