use super::{
    layout::{canonical, normalize, sha256},
    loader,
    playback::{orbit, overlap_fraction},
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
fn mapped_output_ownership_survives_failure_until_explicit_mode_off() {
    let fixture = Fixture::new();
    let shared = std::rc::Rc::new(std::cell::RefCell::new(Some(
        super::load(&fixture.path, false).unwrap(),
    )));
    assert!(super::owns_output(&shared, "Virtual-0"));
    assert!(!super::owns_output(&shared, "Unmapped"));
    shared
        .borrow_mut()
        .as_mut()
        .unwrap()
        .fail("injected failure".into());
    assert!(super::owns_output(&shared, "Virtual-0"));
    assert!(!super::owns_output(&shared, "Unmapped"));
    shared.borrow_mut().take();
    assert!(!super::owns_output(&shared, "Virtual-0"));
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

#[test]
fn overlap_matches_published_producer_extreme_geometry() {
    // Values from producer c17f4e8, not the native implementation.
    let unit = f64::from_bits(1);
    let root_two = 2.0_f64.sqrt();
    let cases = [
        ([0.0, 0.0], [10.0, 0.0], 5.0, 5.0, 0.0),
        ([0.0, 0.0], [0.0, 0.0], 5.0, 3.0, 1.0),
        ([0.0, 0.0], [5.0, 0.0], 5.0, 5.0, 0.3910022189557707),
        ([0.0, 0.0], [1.0, 0.0], 1.0, 2.0, 1.0),
        ([0.0, 0.0], [3.0, 0.0], 1.0, 2.0, 0.0),
        ([0.0, 0.0], [99.1, 0.0], 1.0, 100.0, 0.9812184918183816),
        ([0.0, 0.0], [100.9, 0.0], 1.0, 100.0, 0.018605751654293432),
        (
            [500.0000005, 300.0],
            [600.0, 300.0],
            1e-6,
            100.0,
            0.8044988891369677,
        ),
        ([0.0, 0.0], [100.0, 0.0], unit, 100.0, 0.5),
        ([0.0, 0.0], [100.0, 0.0], 1e-300, 100.0, 0.5),
        ([1e-18, 0.0], [100.0, 0.0], 1e-18, 100.0, 1.0),
        ([1.0, 1.0], [2.0, 2.0], 1e-18, root_two, 1.0),
        ([1.0, 1.0], [2.0, 2.0], 1e-18, root_two - f64::EPSILON, 0.0),
        ([0.0, 0.0], [root_two, 0.0], 1e-18, root_two, 0.5),
        (
            [10.0 * unit, 10.0 * unit],
            [13.0 * unit, 12.0 * unit],
            7.0 * unit,
            7.0 * unit,
            0.6757524754693924,
        ),
        (
            [0.0, 0.0],
            [unit, unit],
            2.0 * unit,
            2.0 * unit,
            0.5594043441634879,
        ),
    ];
    for (first, second, small, large, expected) in cases {
        let actual = overlap_fraction(first, second, small, large);
        assert!(actual.is_finite() && (0.0..=1.0).contains(&actual));
        assert!(
            (actual - expected).abs() < 2e-13,
            "{first:?} {second:?} {small} {large}: {actual} vs {expected}"
        );
        assert_eq!(actual, overlap_fraction(second, first, large, small));
    }
    for radius in [unit, 1e-200, 1.0, 1e200] {
        let actual = overlap_fraction([0.0, 0.0], [radius, 0.0], radius, radius);
        assert!((actual - 0.3910022189557707).abs() < 2e-13);
    }
    for radius in [f64::MIN_POSITIVE - unit, f64::MIN_POSITIVE] {
        let actual = overlap_fraction(
            [0.0, 0.0],
            [radius * 3.0 / 7.0, radius * 2.0 / 7.0],
            radius,
            radius,
        );
        assert!((actual - 0.6757524754693924).abs() < 2e-13);
    }
    assert!(overlap_fraction([0.0, 0.0], [2.0 - f64::EPSILON, 0.0], 1.0, 1.0) > 0.0);
}

#[test]
fn loader_accepts_safe_tiny_circles_and_rejects_excess_overlap() {
    let unit = f64::from_bits(1);
    let root_two = 2.0_f64.sqrt();
    for (first, second, small, large, safe) in [
        (
            [10.0 * unit, 10.0 * unit],
            [13.0 * unit, 12.0 * unit],
            7.0 * unit,
            7.0 * unit,
            false,
        ),
        (
            [10.0 * unit, 10.0 * unit],
            [11.0 * unit, 11.0 * unit],
            2.0 * unit,
            2.0 * unit,
            true,
        ),
        ([1.0, 1.0], [2.0, 2.0], 1e-18, root_two, false),
        (
            [root_two, 4.0],
            [2.0 * root_two, 4.0],
            1e-18,
            root_two,
            true,
        ),
        ([10.0, 10.0], [20.0, 10.0], unit, 10.0, true),
        ([10.00000005, 10.0], [20.0, 10.0], 1e-7, 10.0, false),
        // Producer fractions .6500000005 and .650000002 straddle the 1e-9 tolerance.
        ([10.0, 10.0], [10.557067947976236, 10.0], 1.0, 1.0, true),
        ([10.0, 10.0], [10.557067945522956, 10.0], 1.0, 1.0, false),
    ] {
        let mut fixture = Fixture::new();
        fixture.arrangements["orbit"]["amplitude_pixels"] = json!(0);
        fixture.arrangements["circle_radii"]["cyan"] = json!(small);
        fixture.arrangements["circle_radii"]["violet"] = json!(large);
        for (index, row) in fixture.arrangements["arrangements"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            // Static tiny pair, three distinct maps. Loading samples every directed path.
            row["centers"]["cyan"] = json!(first);
            row["centers"]["violet"] = json!(second);
            row["centers"]["magenta"] = json!([1000 + index, 300]);
            row["centers"]["ice"] = json!([1300, 300]);
        }
        fixture.write();
        let result = super::load(&fixture.path, false);
        if safe {
            result.unwrap();
        } else {
            assert_eq!(
                result.err().unwrap().to_string(),
                "Circle overlap exceeds bound"
            );
        }
    }
}

#[test]
fn loader_preserves_positive_radius_domain_and_producer_amplitude_range() {
    let mut fixture = Fixture::new();
    for amplitude in [0.0, 12.0, 60.0] {
        fixture.arrangements["orbit"]["amplitude_pixels"] = json!(amplitude);
        fixture.write();
        super::load(&fixture.path, false).unwrap();
    }
    for amplitude in [-1.0, 60.000000001] {
        fixture.arrangements["orbit"]["amplitude_pixels"] = json!(amplitude);
        fixture.write();
        assert!(super::load(&fixture.path, false).is_err());
    }
    fixture.arrangements["orbit"]["amplitude_pixels"] = json!(0);
    for radius in [0.0, -f64::from_bits(1), 32.000000001] {
        fixture.arrangements["circle_radii"]["cyan"] = json!(radius);
        fixture.write();
        assert!(super::load(&fixture.path, false).is_err());
    }
    for limit in [0.65000001, 0.64999999] {
        fixture.arrangements["circle_radii"]["cyan"] = json!(22.0);
        fixture.arrangements["constraints"]["max_overlap_fraction"] = json!(limit);
        fixture.write();
        assert!(super::load(&fixture.path, false).is_err());
    }
}

#[test]
fn orbit_normalizes_tiny_nonzero_movement_before_multiplying_offset() {
    let unit = f64::from_bits(1);
    let from = [[0.0, 0.0], [0.0, 0.0]];
    let to = [[unit, 0.0], [0.0, unit]];
    assert_eq!(orbit(&from, &to, 0.0, 1.0), from);
    assert_eq!(orbit(&from, &to, 1.0, 1.0), to);
    assert_eq!(orbit(&from, &to, 0.5, 1.0), [[0.0, 1.0], [1.0, 0.0]]);

    let mut fixture = Fixture::new();
    fixture.arrangements["circle_radii"]["cyan"] = json!(unit);
    for (i, row) in fixture.arrangements["arrangements"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        row["centers"]["cyan"] = json!([(10 + i) as f64 * unit, 300.0]);
    }
    fixture.write();
    super::load(&fixture.path, false).unwrap();
}

#[test]
fn malformed_parent_delimiters_return_errors_without_panics() {
    for raw in [
        b"}".as_slice(),
        b"]",
        b"{}]",
        b"{]",
        b"{\"generation_workflow\":[NaN]}]",
        b"{\"generation_workflow\":NaN}}",
    ] {
        assert!(loader::decode_parent(raw).is_err(), "{:?}", raw);
    }
}

#[test]
fn producer_layout_metadata_limits_match_core_normalization() {
    for (metadata, accepted) in [
        (json!("x".repeat(8192)), true),
        (json!((0..1600).collect::<Vec<u32>>()), true),
        (json!("x".repeat(9000)), false),
        (json!((0..3000).collect::<Vec<u32>>()), false),
        (json!("x".repeat(70000)), false),
    ] {
        let mut fixture = Fixture::new();
        fixture.scene["plan"]["monitors"][0]["metadata"] = metadata;
        // The outer JSON budget is deliberately larger than core's inner layout budget.
        loader::decode_json(&serde_json::to_vec(&fixture.scene).unwrap()).unwrap();
        if accepted {
            let plan = normalize(&fixture.scene["plan"]).unwrap();
            let fingerprint = sha256(canonical(&plan).as_bytes());
            fixture.scene["layout_fingerprint"] = json!(fingerprint);
            fixture.arrangements["scene"]["layout_fingerprint"] = json!(fingerprint);
            fixture.outputs["layout_fingerprint"] = json!(fingerprint);
            fixture.rebind();
            super::load(&fixture.path, false).unwrap();
        } else {
            assert!(normalize(&fixture.scene["plan"]).is_err());
        }
    }
}

#[test]
fn producer_layout_budget_boundaries_match_core() {
    // Accept/reject vectors generated by the producer's `normalize_plan` in violet-void-wallpapers
    // tools/motion_scene.py at a363468, the revision recorded in fixtures `exact_integer_metadata`
    // (core.py sha256 4547ede7289eaa89c2280affe19c0671f813571ce9817c8c9c422a4a0a927d57).
    let nest = |depth: usize| (0..depth).fold(json!(0), |value, _| json!([value]));
    let key = |len: usize| {
        let mut object = serde_json::Map::new();
        object.insert("k".repeat(len), json!(0));
        Value::Object(object)
    };
    let aggregate = |len: usize| {
        let mut strings = vec![json!("x".repeat(8000)); 8];
        strings.push(json!("x".repeat(len)));
        Value::Array(strings)
    };
    let fixtures: Value = serde_json::from_str(include_str!("fixtures/layouts.json")).unwrap();
    for (case, metadata, accepted) in [
        ("depth 29", nest(29), true),
        ("depth 30", nest(30), false),
        ("key 8192", key(8192), true),
        ("key 8193", key(8193), false),
        ("nodes 1990", json!(vec![0; 1990]), true),
        ("nodes 1991", json!(vec![0; 1991]), false),
        ("aggregate 1043", aggregate(1043), true),
        ("aggregate 1044", aggregate(1044), false),
    ] {
        let mut plan = fixtures["cases"][3]["plan"].clone();
        plan["monitors"][0]["metadata"] = metadata;
        assert_eq!(normalize(&plan).is_ok(), accepted, "{case}");
    }
}

#[test]
fn group_and_surface_requests_share_callback_accounting() {
    let fixture = Fixture::new();
    let motion = std::rc::Rc::new(std::cell::RefCell::new(None));
    let surface = std::rc::Rc::new(crate::wallpaper_groups::FrameScheduler::new(
        motion.clone(),
        "Virtual-0".into(),
    ));
    let group = surface.clone();
    for _ in 0..3 {
        assert!(group.request_group());
    }
    *motion.borrow_mut() = Some(super::load(&fixture.path, false).unwrap());
    for _ in 0..20 {
        assert!(!surface.request());
        assert!(!group.request_group());
    }
    for _ in 0..2 {
        group.received();
        assert!(!surface.request());
    }
    group.received();
    assert!(surface.request());
    assert!(!group.request_group());
    assert!(!surface.request());
    motion
        .borrow_mut()
        .as_mut()
        .unwrap()
        .fail("injected fallback".into());
    assert!(!group.request_group());
    surface.received();
    assert!(surface.request());
    motion.borrow_mut().take();
    assert!(group.request_group());
    assert!(group.request_group());
}

#[test]
fn malformed_parent_package_is_rejected_after_exact_hash_binding() {
    for raw in [b"{}]".as_slice(), b"{\"generation_workflow\":[NaN]}]"] {
        let mut fixture = Fixture::new();
        fs::write(fixture.path.join("parent-manifest.json"), raw).unwrap();
        fixture.scene["parent"] = json!({"file":"parent-manifest.json","sha256":sha256(raw),"run_id":"123456789a","layout_fingerprint":fixture.scene["layout_fingerprint"]});
        fixture.rebind();
        assert!(super::load(&fixture.path, false).is_err());
    }
}
