use super::{
    layout::sha256,
    loader::{self, center, fields, id, number, size},
    Scene,
};
use crate::display_info::DisplayInfo;
use color_eyre::{
    eyre::{ensure, eyre},
    Result,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use smithay_client_toolkit::reexports::client::protocol::wl_output::Transform;
use std::{
    collections::{BTreeMap, HashSet},
    rc::Rc,
    time::{Duration, Instant},
};
use wpaperd_ipc::MotionStatus;

#[derive(Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct OutputBinding {
    pub monitor_id: String,
    pub output: String,
    pub pixels: [u32; 2],
    pub transform: String,
    #[serde(skip)]
    pub crop: [u32; 4],
}
impl OutputBinding {
    pub fn validate(&self, display: &DisplayInfo) -> Result<()> {
        ensure!(self.output == display.name, "Output name mismatch");
        ensure!(
            display.is_configured(),
            "Output {} is not configured",
            self.output
        );
        ensure!(
            self.pixels
                == [
                    display.scaled_width() as u32,
                    display.scaled_height() as u32
                ]
                && self.transform == transform_name(display.transform),
            "Output {} dimensions/transform differ from explicit scene mapping",
            self.output
        );
        Ok(())
    }
}
pub fn transform_name(t: Transform) -> &'static str {
    match t {
        Transform::Normal => "normal",
        Transform::_90 => "90",
        Transform::_180 => "180",
        Transform::_270 => "270",
        Transform::Flipped => "flipped",
        Transform::Flipped90 => "flipped90",
        Transform::Flipped180 => "flipped180",
        Transform::Flipped270 => "flipped270",
        _ => "unsupported",
    }
}

struct Arrangement {
    id: String,
    centers: Vec<[f64; 2]>,
}
struct Transition {
    from: Vec<[f64; 2]>,
    started: Instant,
}
pub struct Playback {
    pub scene: Rc<Scene>,
    pub outputs: Vec<OutputBinding>,
    arrangements: Vec<Arrangement>,
    radii: Vec<f64>,
    duration: Duration,
    amplitude: f64,
    index: usize,
    history: Vec<usize>,
    transition: Option<Transition>,
    frozen_at: Option<Instant>,
    reduced_motion: bool,
    pub failure: Option<String>,
    pub arrangements_sha256: String,
}
impl Playback {
    pub fn load(package: &loader::Package, scene: Rc<Scene>, reduced_motion: bool) -> Result<Self> {
        let raw = package.read("arrangements.json", loader::JSON_LIMIT)?;
        let m = loader::decode_json(&raw)?;
        fields(
            &m,
            &[
                "version",
                "role",
                "scene",
                "duration_ms",
                "easing",
                "orbit",
                "circle_radii",
                "constraints",
                "arrangements",
            ],
            &[],
        )?;
        ensure!(
            m["version"].as_u64() == Some(1)
                && m["role"] == "motion-arrangements"
                && m["duration_ms"].as_u64() == Some(1800)
                && m["easing"] == "cubic-in-out",
            "Unsupported arrangements contract"
        );
        fields(
            &m["scene"],
            &["scene_id", "sha256", "layout_fingerprint"],
            &[],
        )?;
        ensure!(
            m["scene"]["scene_id"] == scene.manifest["scene_id"]
                && m["scene"]["sha256"] == scene.sha256
                && m["scene"]["layout_fingerprint"] == scene.manifest["layout_fingerprint"],
            "Arrangements scene binding mismatch"
        );
        fields(&m["orbit"], &["kind", "amplitude_pixels"], &[])?;
        let amplitude = number(&m["orbit"]["amplitude_pixels"], 0.0, 1966.08)?;
        let expected = (0.06 * scene.canvas[0].min(scene.canvas[1]) as f64 * 1000.0)
            .round_ties_even()
            / 1000.0;
        ensure!(
            m["orbit"]["kind"] == "perpendicular-sine" && (amplitude - expected).abs() < 1e-9,
            "Unsupported orbit geometry"
        );
        fields(
            &m["constraints"],
            &[
                "max_overlap_fraction",
                "min_negative_space_fraction",
                "max_travel_fraction",
            ],
            &[],
        )?;
        ensure!(
            m["constraints"]["max_overlap_fraction"].as_f64() == Some(0.65)
                && m["constraints"]["min_negative_space_fraction"].as_f64() == Some(0.45)
                && m["constraints"]["max_travel_fraction"].as_f64() == Some(0.25),
            "Unsupported arrangement constraints"
        );
        let radius_map = m["circle_radii"]
            .as_object()
            .filter(|v| v.len() == 4)
            .ok_or_else(|| eyre!("Missing exact object radii"))?;
        let mut radii = vec![];
        for obj in &scene.objects {
            let r = number(
                radius_map
                    .get(&obj.id)
                    .ok_or_else(|| eyre!("Missing object radius"))?,
                0.0,
                obj.size[0] as f64 / 2.0,
            )?;
            ensure!(r > 0.0, "Circle radius must be positive");
            radii.push(r);
        }
        let entries = m["arrangements"]
            .as_array()
            .filter(|v| v.len() == 3)
            .ok_or_else(|| eyre!("Exactly three arrangements required"))?;
        let mut names = HashSet::new();
        let mut arrangements = vec![];
        for a in entries {
            fields(a, &["id", "centers"], &[])?;
            let name = a["id"]
                .as_str()
                .ok_or_else(|| eyre!("Invalid arrangement ID"))?;
            id(name)?;
            ensure!(names.insert(name), "Duplicate arrangement ID");
            let centers = a["centers"]
                .as_object()
                .filter(|v| v.len() == 4)
                .ok_or_else(|| eyre!("Arrangement must reference every stable object"))?;
            let centers = scene
                .objects
                .iter()
                .map(|o| {
                    center(
                        centers
                            .get(&o.id)
                            .ok_or_else(|| eyre!("Missing arrangement object"))?,
                        scene.canvas,
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            arrangements.push(Arrangement {
                id: name.into(),
                centers,
            });
        }
        ensure!(
            arrangements.iter().enumerate().all(|(i, a)| arrangements
                .iter()
                .skip(i + 1)
                .all(|b| a.centers != b.centers)),
            "Arrangements must have distinct centers"
        );
        let raw_outputs = package.read("outputs.json", loader::JSON_LIMIT)?;
        let mapping = loader::decode_json(&raw_outputs)?;
        fields(
            &mapping,
            &[
                "version",
                "role",
                "scene_sha256",
                "layout_fingerprint",
                "outputs",
            ],
            &[],
        )?;
        ensure!(
            mapping["version"].as_u64() == Some(1)
                && mapping["role"] == "wpaperd-motion-outputs"
                && mapping["scene_sha256"] == scene.sha256
                && mapping["layout_fingerprint"] == scene.manifest["layout_fingerprint"],
            "Output mapping binding mismatch"
        );
        let mut outputs: Vec<OutputBinding> = serde_json::from_value(mapping["outputs"].clone())?;
        let monitors = scene.manifest["plan"]["monitors"].as_array().unwrap();
        ensure!(
            outputs.len() == monitors.len(),
            "Mapping must bind every plan monitor explicitly"
        );
        let (mut ids, mut live_names) = (HashSet::new(), HashSet::new());
        for o in &mut outputs {
            ensure!(
                !o.output.is_empty()
                    && o.output.len() <= 240
                    && !o.output.chars().any(char::is_control)
                    && live_names.insert(o.output.clone())
                    && ids.insert(o.monitor_id.clone()),
                "Duplicate/invalid output mapping"
            );
            let monitor = monitors
                .iter()
                .find(|m| m["id"] == o.monitor_id)
                .ok_or_else(|| eyre!("Unknown plan monitor"))?;
            ensure!(
                size(&monitor["pixels"], 16384)? == o.pixels,
                "Mapping pixels must equal plan upright raster"
            );
            ensure!(
                [
                    "normal",
                    "90",
                    "180",
                    "270",
                    "flipped",
                    "flipped90",
                    "flipped180",
                    "flipped270"
                ]
                .contains(&o.transform.as_str()),
                "Unsupported output transform"
            );
            if let Some(t) = monitor.get("transform") {
                ensure!(
                    t == &Value::String(o.transform.clone()),
                    "Mapping transform differs from plan"
                );
            }
            let crop = monitor["crop_box"].as_array().unwrap();
            for (i, v) in crop.iter().enumerate() {
                o.crop[i] = v.as_u64().ok_or_else(|| eyre!("Invalid normalized crop"))? as u32;
            }
        }
        let result = Self {
            scene,
            outputs,
            arrangements,
            radii,
            duration: Duration::from_millis(1800),
            amplitude,
            index: 0,
            history: vec![],
            transition: None,
            frozen_at: None,
            reduced_motion,
            failure: None,
            arrangements_sha256: sha256(&raw),
        };
        for a in &result.arrangements {
            result.validate_positions(&a.centers)?;
        }
        for a in &result.arrangements {
            for b in &result.arrangements {
                result.validate_transition(&a.centers, &b.centers)?;
            }
        }
        Ok(result)
    }
    fn validate_positions(&self, centers: &[[f64; 2]]) -> Result<()> {
        let [w, h] = self.scene.canvas;
        let mut covered = 0.0;
        for (i, (c, r)) in centers.iter().zip(&self.radii).enumerate() {
            ensure!(
                c[0].is_finite()
                    && c[1].is_finite()
                    && c[0] + 1e-9 >= *r
                    && c[1] + 1e-9 >= *r
                    && c[0] + r <= w as f64 + 1e-9
                    && c[1] + r <= h as f64 + 1e-9,
                "Circle rim outside canvas"
            );
            covered += std::f64::consts::PI * r * r;
            for (b, s) in centers.iter().zip(&self.radii).skip(i + 1) {
                let distance = (c[0] - b[0]).hypot(c[1] - b[1]);
                let area = overlap(*r, *s, distance);
                ensure!(
                    area / (std::f64::consts::PI * r.min(*s).powi(2)) <= 0.65 + 1e-9,
                    "Circle overlap exceeds bound"
                );
            }
        }
        // Conservative union bound: overlap can only increase negative space.
        ensure!(
            1.0 - covered / (w as f64 * h as f64) >= 0.45 - 1e-9,
            "Negative space lower bound insufficient"
        );
        Ok(())
    }
    fn validate_transition(&self, from: &[[f64; 2]], to: &[[f64; 2]]) -> Result<()> {
        let diagonal = (self.scene.canvas[0] as f64).hypot(self.scene.canvas[1] as f64);
        for step in 0..=60 {
            let positions = orbit(from, to, step as f64 / 60.0, self.amplitude);
            self.validate_positions(&positions)?;
            for (a, b) in from.iter().zip(positions) {
                ensure!(
                    (b[0] - a[0]).hypot(b[1] - a[1]) / diagonal <= 0.25 + 1e-9,
                    "Sampled travel exceeds bound"
                );
            }
        }
        Ok(())
    }
    pub fn sample(&self, now: Instant) -> Vec<[f64; 2]> {
        let target = &self.arrangements[self.index].centers;
        let Some(t) = &self.transition else {
            return target.clone();
        };
        let time = self.frozen_at.unwrap_or(now);
        let p =
            time.saturating_duration_since(t.started).as_secs_f64() / self.duration.as_secs_f64();
        orbit(&t.from, target, p, self.amplitude)
    }
    pub fn animating(&self, now: Instant) -> bool {
        self.failure.is_none()
            && self.frozen_at.is_none()
            && self
                .transition
                .as_ref()
                .is_some_and(|t| now.saturating_duration_since(t.started) < self.duration)
    }
    fn retarget(&mut self, index: usize, now: Instant) -> Result<()> {
        ensure!(
            self.failure.is_none(),
            "Scene is in fallback; stop/start to recover"
        );
        let from = self.sample(now);
        self.validate_transition(&from, &self.arrangements[index].centers)?;
        self.index = index;
        self.transition = if self.reduced_motion && self.frozen_at.is_none() {
            None
        } else {
            Some(Transition { from, started: now })
        };
        if self.frozen_at.is_some() {
            self.frozen_at = Some(now);
        }
        Ok(())
    }
    pub fn shuffle(&mut self, now: Instant) -> Result<()> {
        // Deliberate deterministic traversal; no unbounded command queue or random retries.
        let old = self.index;
        self.retarget((old + 1) % self.arrangements.len(), now)?;
        if self.history.len() == 32 {
            self.history.remove(0);
        }
        self.history.push(old);
        Ok(())
    }
    pub fn previous(&mut self, now: Instant) -> Result<()> {
        let Some(&index) = self.history.last() else {
            return Ok(());
        };
        self.retarget(index, now)?;
        self.history.pop();
        Ok(())
    }
    pub fn freeze(&mut self, now: Instant) {
        if let Some(at) = self.frozen_at.take() {
            if self.reduced_motion {
                self.transition = None;
            } else if let Some(t) = self.transition.as_mut() {
                t.started += now.saturating_duration_since(at);
            }
        } else {
            self.frozen_at = Some(now);
        }
    }
    pub fn fail(&mut self, message: String) {
        self.failure = Some(message);
        self.frozen_at = Some(Instant::now());
    }
    pub fn status(&self, displays: &[&DisplayInfo]) -> MotionStatus {
        let now = Instant::now();
        let mut entries = BTreeMap::new();
        for o in &self.outputs {
            let status = match displays.iter().find(|d| d.name == o.output) {
                None => "missing",
                Some(d) if o.validate(d).is_err() => "mismatch",
                Some(_) => "mapped",
            };
            entries.insert(o.output.clone(), status.to_string());
        }
        MotionStatus {
            active: true,
            scene_id: Some(self.scene.manifest["scene_id"].as_str().unwrap().into()),
            scene_sha256: Some(self.scene.sha256.clone()),
            arrangements_sha256: Some(self.arrangements_sha256.clone()),
            arrangement: Some(self.arrangements[self.index].id.clone()),
            state: if self.failure.is_some() {
                "fallback"
            } else if self.frozen_at.is_some() {
                "frozen"
            } else if self.animating(now) {
                "orbit"
            } else {
                "static"
            }
            .into(),
            reduced_motion: self.reduced_motion,
            history_depth: self.history.len(),
            outputs: entries,
            error: self.failure.clone(),
        }
    }
}
fn overlap(r: f64, s: f64, d: f64) -> f64 {
    if d >= r + s {
        0.0
    } else if d <= (r - s).abs() {
        std::f64::consts::PI * r.min(s).powi(2)
    } else {
        r * r
            * ((d * d + r * r - s * s) / (2.0 * d * r))
                .clamp(-1.0, 1.0)
                .acos()
            + s * s
                * ((d * d + s * s - r * r) / (2.0 * d * s))
                    .clamp(-1.0, 1.0)
                    .acos()
            - 0.5
                * ((-d + r + s) * (d + r - s) * (d - r + s) * (d + r + s))
                    .max(0.0)
                    .sqrt()
    }
}
pub fn orbit(from: &[[f64; 2]], to: &[[f64; 2]], t: f64, amplitude: f64) -> Vec<[f64; 2]> {
    let t = t.clamp(0.0, 1.0);
    if t == 0.0 {
        return from.to_vec();
    }
    if t == 1.0 {
        return to.to_vec();
    }
    let u = if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    };
    from.iter()
        .zip(to)
        .enumerate()
        .map(|(i, (a, b))| {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let d = dx.hypot(dy);
            let offset = if d == 0.0 {
                0.0
            } else {
                amplitude * (std::f64::consts::PI * u).sin() * if i % 2 == 0 { 1.0 } else { -1.0 }
                    / d
            };
            [a[0] + dx * u - dy * offset, a[1] + dy * u + dx * offset]
        })
        .collect()
}
