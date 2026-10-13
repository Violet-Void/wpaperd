use super::{
    layout::{canonical, normalize, sha256},
    loader,
    playback::orbit,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

struct Fixture {
    path: PathBuf,
    scene: Value,
    arrangements: Value,
    outputs: Value,
}
impl Fixture {
    fn new() -> Self {
        static SERIAL: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "wpaperd-scene-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        let fixtures: Value = serde_json::from_str(include_str!("fixtures/layouts.json")).unwrap();
        let plan = fixtures["cases"][3]["plan"].clone();
        let fingerprint = fixtures["cases"][3]["fingerprint"].clone();
        let names = ["cyan", "violet", "magenta", "ice"];
        let centers = [[240, 240], [560, 240], [1040, 240], [1360, 240]];
        let mut assets = serde_json::Map::new();
        let mut objects = vec![];
        for (i, name) in names.iter().enumerate() {
            let data = encoded_png(64, 64, [30 + i as u8 * 40, 130, 240, 100]);
            fs::write(path.join(format!("{name}.png")), &data).unwrap();
            assets.insert(
                name.to_string(),
                json!({"file":format!("{name}.png"),"sha256":sha256(&data),"size":[64,64]}),
            );
            objects.push(json!({"id":name,"asset":name,"material":"luminous-translucent-circle","color":[30,130,240,255],"alpha":0.4,"glow":{"radius":3.0,"alpha":0.1},"depth":i,"size":[64,64],"center":centers[i]}));
        }
        let scene = json!({"version":1,"role":"motion-scene-sidecar","scene_id":"synthetic-test","origin":"manual-independent-rgba","compositing":"straight-alpha-over-srgb8-depth-ascending","canvas":[1600,600],"plan":plan,"layout_fingerprint":fingerprint,"background":{"solid":[3,2,8,255]},"objects":objects,"assets":assets,"previews":{}});
        let scene_raw = serde_json::to_vec_pretty(&scene).unwrap();
        let scene_sha = sha256(&scene_raw);
        let arrangements = json!({"version":1,"role":"motion-arrangements","scene":{"scene_id":"synthetic-test","sha256":scene_sha,"layout_fingerprint":fingerprint},"duration_ms":1800,"easing":"cubic-in-out","orbit":{"kind":"perpendicular-sine","amplitude_pixels":36.0},"circle_radii":{"cyan":22.0,"violet":22.0,"magenta":22.0,"ice":22.0},"constraints":{"max_overlap_fraction":0.65,"min_negative_space_fraction":0.45,"max_travel_fraction":0.25},"arrangements":[
            {"id":"initial","centers":{"cyan":[240,240],"violet":[560,240],"magenta":[1040,240],"ice":[1360,240]}},
            {"id":"second","centers":{"cyan":[300,240],"violet":[620,240],"magenta":[1100,240],"ice":[1420,240]}},
            {"id":"third","centers":{"cyan":[180,240],"violet":[500,240],"magenta":[980,240],"ice":[1300,240]}}
        ]});
        let outputs = json!({"version":1,"role":"wpaperd-motion-outputs","scene_sha256":scene_sha,"layout_fingerprint":fingerprint,"outputs":[{"monitor_id":"left","output":"Virtual-0","pixels":[800,600],"transform":"normal"},{"monitor_id":"right","output":"Virtual-1","pixels":[800,600],"transform":"normal"}]});
        let result = Self {
            path,
            scene,
            arrangements,
            outputs,
        };
        result.write();
        result
    }
    fn write(&self) {
        for (name, value) in [
            ("scene.json", &self.scene),
            ("arrangements.json", &self.arrangements),
            ("outputs.json", &self.outputs),
        ] {
            fs::write(
                self.path.join(name),
                serde_json::to_vec_pretty(value).unwrap(),
            )
            .unwrap();
        }
    }
    fn rebind(&mut self) {
        let hash = sha256(&serde_json::to_vec_pretty(&self.scene).unwrap());
        self.arrangements["scene"]["sha256"] = json!(hash);
        self.outputs["scene_sha256"] = json!(hash);
        self.write();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}
fn encoded_png(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
    let mut bytes = vec![];
    {
        let mut encoder = png::Encoder::new(&mut bytes, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&color.repeat(w as usize * h as usize))
            .unwrap();
    }
    bytes
}

#[test]
fn translucent_background_and_overlapping_rgba_layers_stay_unchanged() {
    let mut fixture = Fixture::new();
    fixture.scene["background"] = json!({"solid":[10,80,160,64]});
    let layers = [("cyan", [240, 20, 30, 128]), ("violet", [0, 220, 40, 96])];
    for (key, rgba) in layers {
        let data = encoded_png(64, 64, rgba);
        fs::write(fixture.path.join(format!("{key}.png")), &data).unwrap();
        fixture.scene["assets"][key]["sha256"] = json!(sha256(&data));
    }
    // These sprite rectangles and declared circle rims overlap within the producer bound.
    for (i, x) in [270, 330, 210].into_iter().enumerate() {
        fixture.arrangements["arrangements"][i]["centers"]["violet"] = json!([x, 240]);
    }
    fixture.rebind();
    let playback = super::load(&fixture.path, false).unwrap();
    assert!(matches!(
        playback.scene.background,
        super::Background::Solid([10, 80, 160, 64])
    ));
    for (key, rgba) in layers {
        assert_eq!(playback.scene.images[key].get_pixel(32, 32).0, rgba);
    }

    // A translucent full-canvas PNG background is equally valid and remains straight RGBA.
    let data = encoded_png(1600, 600, [10, 80, 160, 64]);
    fs::write(fixture.path.join("background.png"), &data).unwrap();
    fixture.scene["assets"]["background"] =
        json!({"file":"background.png","sha256":sha256(&data),"size":[1600,600]});
    fixture.scene["background"] = json!({"asset":"background"});
    fixture.rebind();
    let playback = super::load(&fixture.path, false).unwrap();
    assert_eq!(
        playback.scene.images["background"].get_pixel(500, 300).0,
        [10, 80, 160, 64]
    );
}

#[test]
fn producer_geometry_and_fingerprints_match_real_core() {
    let fixtures: Value = serde_json::from_str(include_str!("fixtures/layouts.json")).unwrap();
    for case in fixtures["cases"].as_array().unwrap() {
        let plan = normalize(&case["plan"]).unwrap();
        assert_eq!(
            sha256(canonical(&plan).as_bytes()),
            case["fingerprint"].as_str().unwrap()
        );
    }
    for case in fixtures["canonical_number_cases"].as_array().unwrap() {
        let value = loader::decode_json(case["json"].as_str().unwrap().as_bytes()).unwrap();
        assert_eq!(canonical(&value), case["expected"].as_str().unwrap());
    }
}

#[test]
fn normalization_preserves_first_equal_extent_and_signed_zero_order() {
    let fixtures: Value = serde_json::from_str(include_str!("fixtures/layouts.json")).unwrap();
    for case in fixtures["normalization_regressions"]["cases"]
        .as_array()
        .unwrap()
    {
        let plan = normalize(&case["plan"]).unwrap();
        assert_eq!(canonical(&plan), case["canonical"].as_str().unwrap());
        assert_eq!(
            sha256(canonical(&plan).as_bytes()),
            case["fingerprint"].as_str().unwrap()
        );
    }
}
#[test]
fn producer_large_integer_metadata_keeps_exact_canonical_identity() {
    let fixtures: Value = serde_json::from_str(include_str!("fixtures/layouts.json")).unwrap();
    let case = &fixtures["exact_integer_metadata"];
    let root = loader::decode_json(&serde_json::to_vec(&case["plan"]).unwrap()).unwrap();
    let plan = normalize(&root).unwrap();
    assert_eq!(canonical(&plan), case["canonical"].as_str().unwrap());
    assert_eq!(sha256(canonical(&plan).as_bytes()), case["fingerprint"]);
    assert_eq!(
        plan["monitors"][0]["opaque_integer"].to_string(),
        "1267650600228229401496703205376"
    );
}
#[test]
fn canonical_encoding_forgery_cannot_bind_normalized_or_parent_plan() {
    let fixtures: Value = serde_json::from_str(include_str!("fixtures/layouts.json")).unwrap();
    let original = &fixtures["normalization_regressions"]["cases"][2]["plan"];
    let mut forged = original.clone();
    forged["origin"][1] = json!(-0.0);
    assert_eq!(forged["origin"][1].as_f64(), original["origin"][1].as_f64());
    assert_ne!(canonical(&forged), canonical(original));
    assert!(normalize(&forged).is_err());
    forged = original.clone();
    forged["density"] = json!(1);
    assert_eq!(forged["density"].as_f64(), original["density"].as_f64());
    assert!(normalize(&forged).is_err());

    for forged_value in [json!(-0.0), json!(0.0)] {
        let mut fixture = Fixture::new();
        let mut parent_plan = fixture.scene["plan"].clone();
        parent_plan["origin"][0] = forged_value;
        let parent = json!({"version":4,"plan":parent_plan,"layout_fingerprint":fixture.scene["layout_fingerprint"],"source_size":fixture.scene["plan"]["generation"],"final_input_size":fixture.scene["canvas"],"run_id":"123456789a"});
        let bytes = serde_json::to_vec(&parent).unwrap();
        fs::write(fixture.path.join("parent-manifest.json"), &bytes).unwrap();
        fixture.scene["parent"] = json!({"file":"parent-manifest.json","sha256":sha256(&bytes),"run_id":"123456789a","layout_fingerprint":fixture.scene["layout_fingerprint"]});
        fixture.rebind();
        assert!(super::load(&fixture.path, false).is_err());
    }
}
#[test]
fn json_numbers_and_literal_private_sentinel_objects_are_distinct() {
    let parsed = loader::decode_json(br#"{"n":1267650600228229401496703205376,"z":-0,"f":1e2,"s":-0.0,"object":{"$serde_json::private::Number":"1"},"raw":{"$serde_json::private::RawValue":"1"}}"#).unwrap();
    assert_eq!(parsed["n"].to_string(), "1267650600228229401496703205376");
    assert_eq!(canonical(&parsed["z"]), "0");
    assert_eq!(canonical(&parsed["f"]), "100.0");
    assert_eq!(canonical(&parsed["s"]), "-0.0");
    assert!(parsed["object"].is_object() && parsed["raw"].is_object());
    assert!(loader::decode_json(br#"{"n":1e999}"#).is_err());
    assert!(loader::decode_json(br#"{"n":-1e999}"#).is_err());
    for key in [
        "$serde_json::private::Number",
        "$serde_json::private::RawValue",
    ] {
        let mut fixture = Fixture::new();
        fixture.scene["version"] = json!({key:"1"});
        fixture.rebind();
        assert!(super::load(&fixture.path, false).is_err());
        fixture.scene["version"] = json!(1);
        fixture.scene["plan"]["density"] = json!({key:"1.0"});
        fixture.rebind();
        assert!(super::load(&fixture.path, false).is_err());
    }
    assert!(loader::decode_json(
        format!("{{\"s\":\"{}\"}}", "a".repeat(256 * 1024 + 1)).as_bytes()
    )
    .is_err());
    assert!(
        loader::decode_json(format!("{{\"s\":[{}]}}", vec!["0"; 32768].join(",")).as_bytes())
            .is_err()
    );
}
#[test]
fn refreshed_fingerprint_does_not_certify_forged_crop() {
    let mut fixture = Fixture::new();
    fixture.scene["plan"]["monitors"][0]["crop_box"][0] = json!(1);
    fixture.scene["layout_fingerprint"] =
        json!(sha256(canonical(&fixture.scene["plan"]).as_bytes()));
    fixture.rebind();
    assert!(super::load(&fixture.path, false).is_err());
}
#[test]
fn json_rejects_duplicates_depth_nonfinite_and_unknown_fields() {
    assert!(loader::decode_json(br#"{"a":1,"a":2}"#).is_err());
    assert!(loader::decode_json(br#"{"a":NaN}"#).is_err());
    assert!(loader::decode_json(
        format!("{{\"a\":{}0{}}}", "[".repeat(33), "]".repeat(33)).as_bytes()
    )
    .is_err());
    let mut fixture = Fixture::new();
    fixture.scene["extra"] = json!(true);
    fixture.rebind();
    assert!(super::load(&fixture.path, false).is_err());
}
#[test]
fn parent_nan_is_opaque_only_inside_root_workflow() {
    assert!(loader::decode_parent(br#"{"generation_workflow":NaN}"#).is_ok());
    assert!(
        loader::decode_parent(br#"{"generation_workflow":{"a":[NaN]},"run_id":"123"}"#).is_ok()
    );
    assert!(
        loader::decode_parent(br#"{"generation_workflow":{"a":[NaN]},"geometry":NaN}"#).is_err()
    );
    assert!(loader::decode_parent(
        br#"{"generation_workflow":{"a":[NaN]},"generation_workflow":{}}"#
    )
    .is_err());
    assert!(loader::decode_parent(br#"{"a":{"generation_workflow":[NaN]}}"#).is_err());
}
#[test]
fn portable_files_hashes_and_symlink_ancestors_fail_closed() {
    for path in ["../a.png", "/a.png", "a//b.png", "a\\b.png", "a/./b.png"] {
        assert!(loader::portable(path).is_err());
    }
    let mut f = Fixture::new();
    let link = f.path.join("alias.png");
    std::os::unix::fs::symlink("cyan.png", &link).unwrap();
    f.scene["assets"]["cyan"]["file"] = json!("alias.png");
    f.rebind();
    assert!(super::load(&f.path, false).is_err());
    let alias = f.path.with_extension("alias");
    std::os::unix::fs::symlink(&f.path, &alias).unwrap();
    assert!(loader::Package::open(&alias).is_err());
    fs::remove_file(alias).unwrap();
    f.scene["assets"]["cyan"]["file"] = json!("cyan.png");
    fs::write(f.path.join("cyan.png"), b"tamper").unwrap();
    f.rebind();
    assert!(super::load(&f.path, false).is_err());
}
fn alter_idat(bytes: &[u8], mut alter: impl FnMut(&mut Vec<u8>)) -> Vec<u8> {
    let mut out = bytes[..8].to_vec();
    let mut i = 8;
    while i < bytes.len() {
        let len = u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        let kind = &bytes[i + 4..i + 8];
        let mut payload = bytes[i + 8..i + 8 + len].to_vec();
        if kind == b"IDAT" {
            alter(&mut payload);
        }
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(&payload);
        let crc = crc32fast::hash(&out[out.len() - payload.len() - 4..]);
        out.extend_from_slice(&crc.to_be_bytes());
        i += len + 12;
    }
    out
}
#[test]
fn png_requires_valid_checksums_complete_stream_and_terminal_framing() {
    let good = encoded_png(2, 2, [20, 30, 40, 50]);
    let image = loader::png(&good, [2, 2]).unwrap();
    assert_eq!(image.as_raw(), &[20, 30, 40, 50].repeat(4));
    let missing = alter_idat(&good, |data| {
        data.truncate(data.len() - 2);
    });
    assert!(
        loader::png(&missing, [2, 2]).is_err(),
        "missing Adler trailer"
    );
    let bad = alter_idat(&good, |data| {
        let n = data.len();
        data[n - 1] ^= 1;
    });
    assert!(
        loader::png(&bad, [2, 2]).is_err(),
        "invalid Adler with refreshed CRC"
    );
    let compressed = alter_idat(&good, |data| {
        data.truncate(data.len() / 2);
    });
    assert!(loader::png(&compressed, [2, 2]).is_err());
    let mut trailing = good.clone();
    trailing.push(0);
    assert!(loader::png(&trailing, [2, 2]).is_err());
    let trailing_idat = alter_idat(&good, |data| {
        data.extend_from_slice(b"PNG permitted unused compressed bytes")
    });
    assert!(
        loader::png(&trailing_idat, [2, 2]).is_ok(),
        "PNG-standard unused IDAT bytes"
    );
    let mut crc = good.clone();
    crc[29] ^= 1;
    assert!(loader::png(&crc, [2, 2]).is_err());
    assert!(loader::png(&good, [3, 2]).is_err());
}
#[test]
fn arrangements_keep_identity_and_bounded_continuous_command_history() {
    let f = Fixture::new();
    let mut p = super::load(&f.path, false).unwrap();
    let now = Instant::now();
    let initial = p.sample(now);
    p.shuffle(now).unwrap();
    assert_eq!(p.sample(now), initial);
    assert!(p.animating(now));
    let middle = now + Duration::from_millis(900);
    let before = p.sample(middle);
    p.shuffle(middle).unwrap();
    assert_eq!(p.sample(middle), before);
    let frozen = middle + Duration::from_millis(100);
    p.freeze(frozen);
    let sample = p.sample(frozen);
    assert_eq!(p.sample(frozen + Duration::from_secs(10)), sample);
    assert!(!p.animating(frozen));
    p.freeze(frozen + Duration::from_secs(10));
    assert_eq!(p.sample(frozen + Duration::from_secs(10)), sample);
    for n in 0..100 {
        p.shuffle(frozen + Duration::from_secs(20 + n)).unwrap();
    }
    assert_eq!(p.status(&[]).history_depth, 32);
    for n in 0..40 {
        p.previous(frozen + Duration::from_secs(130 + n)).unwrap();
    }
    assert_eq!(p.status(&[]).history_depth, 0);
    assert_eq!(
        p.scene
            .objects
            .iter()
            .map(|o| o.id.as_str())
            .collect::<Vec<_>>(),
        vec!["cyan", "violet", "magenta", "ice"]
    );
    p.fail("synthetic renderer failure".into());
    assert_eq!(p.status(&[]).state, "fallback");
    assert!(p.shuffle(Instant::now()).is_err());
}
#[test]
fn reduced_motion_changes_once_and_stays_idle() {
    let f = Fixture::new();
    let mut p = super::load(&f.path, true).unwrap();
    let now = Instant::now();
    p.shuffle(now).unwrap();
    assert!(!p.animating(now));
    assert_eq!(p.sample(now)[0], [300.0, 240.0]);
    assert_eq!(p.status(&[]).state, "static");
    p.freeze(now);
    let held = p.sample(now);
    p.shuffle(now).unwrap();
    assert_eq!(p.sample(now + Duration::from_secs(10)), held);
    p.freeze(now + Duration::from_secs(10));
    assert_eq!(p.sample(now + Duration::from_secs(10))[0], [180.0, 240.0]);
}
#[test]
fn mappings_and_stable_ids_are_explicit() {
    let mut f = Fixture::new();
    f.outputs["outputs"][1]["output"] = json!("Virtual-0");
    f.write();
    assert!(super::load(&f.path, false).is_err());
    f.outputs["outputs"][1]["output"] = json!("Virtual-1");
    f.outputs["outputs"][1]["pixels"] = json!([600, 800]);
    f.write();
    assert!(super::load(&f.path, false).is_err());
    f.outputs["outputs"][1]["pixels"] = json!([800, 600]);
    f.arrangements["arrangements"][1]["centers"]["other"] = json!([10, 20]);
    f.write();
    assert!(super::load(&f.path, false).is_err());
}
#[test]
fn orbit_matches_easing_endpoint_and_zero_travel_contract() {
    let from = [[0.0, 0.0], [20.0, 20.0]];
    let to = [[100.0, 0.0], [20.0, 20.0]];
    assert_eq!(orbit(&from, &to, 0.0, 30.0), from);
    assert_eq!(orbit(&from, &to, 1.0, 30.0), to);
    let mid = orbit(&from, &to, 0.5, 30.0);
    assert_eq!(mid[0], [50.0, 30.0]);
    assert_eq!(mid[1], to[1]);
}

#[test]
fn orbit_returns_authored_fractional_endpoints_without_cancellation() {
    let from = [[100.0, 100.0], [12.25, -0.0]];
    let to = [[33.49999999999999, 33.49999999999999], [12.25, -0.0]];
    for t in [-10.0, 0.0] {
        assert_eq!(orbit(&from, &to, t, 30.0), from);
    }
    for t in [1.0, 10.0] {
        let centers = orbit(&from, &to, t, 30.0);
        assert_eq!(centers, to);
        assert_eq!((centers[0][0] - 32.0).round_ties_even(), 1.0);
        assert_eq!(centers[1][1].to_bits(), (-0.0_f64).to_bits());
    }
    assert_eq!(orbit(&from, &from, 0.5, 30.0), from);
}
