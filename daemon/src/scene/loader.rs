use std::{
    collections::{BTreeMap, HashSet},
    ffi::CString,
    fs::File,
    io::{Cursor, Read},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path},
    rc::Rc,
};

use color_eyre::{
    eyre::{ensure, eyre},
    Result,
};
use image::RgbaImage;
use nix::libc;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::{json, value::RawValue, Number, Value};

use super::{
    layout::{canonical, normalize, sha256},
    Background, Object, Scene,
};

pub const JSON_LIMIT: usize = 1024 * 1024 + 1;
const ASSET_LIMIT: usize = 64 * 1024 * 1024;
const TOTAL_LIMIT: usize = 128 * 1024 * 1024;

/// Pin every ancestor with O_NOFOLLOW, then read bytes from the same regular descriptor.
pub struct Package(File);
impl Package {
    pub fn open(path: &Path) -> Result<Self> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let mut fd = File::open("/")?;
        for part in absolute.components() {
            match part {
                Component::RootDir => (),
                Component::Normal(name) => fd = open_child(&fd, name.as_bytes(), true)?,
                _ => return Err(eyre!("Package path contains noncanonical components")),
            }
        }
        Ok(Self(fd))
    }

    pub fn read(&self, path: &str, limit: usize) -> Result<Vec<u8>> {
        portable(path)?;
        let parts = path.split('/').collect::<Vec<_>>();
        let mut owner = self.0.try_clone()?;
        for part in &parts[..parts.len() - 1] {
            owner = open_child(&owner, part.as_bytes(), true)?;
        }
        let mut file = open_child(&owner, parts.last().unwrap().as_bytes(), false)?;
        let before = file.metadata()?;
        ensure!(
            before.is_file() && before.len() <= limit as u64,
            "Not a bounded regular file: {path}"
        );
        let mut bytes = Vec::with_capacity(before.len() as usize);
        (&mut file).take(limit as u64 + 1).read_to_end(&mut bytes)?;
        let after = file.metadata()?;
        ensure!(
            bytes.len() as u64 == before.len()
                && (
                    before.len(),
                    before.mtime(),
                    before.mtime_nsec(),
                    before.ctime(),
                    before.ctime_nsec()
                ) == (
                    after.len(),
                    after.mtime(),
                    after.mtime_nsec(),
                    after.ctime(),
                    after.ctime_nsec()
                ),
            "File changed while reading: {path}"
        );
        Ok(bytes)
    }
}

fn open_child(owner: &File, name: &[u8], directory: bool) -> Result<File> {
    let name = CString::new(name)?;
    let flags = libc::O_RDONLY
        | libc::O_CLOEXEC
        | libc::O_NONBLOCK
        | libc::O_NOFOLLOW
        | if directory { libc::O_DIRECTORY } else { 0 };
    let fd = unsafe { libc::openat(owner.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

pub fn portable(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty() && path.len() <= 240,
        "Invalid portable asset path"
    );
    for p in path.split('/') {
        ensure!(
            !p.is_empty()
                && p.len() <= 80
                && p != "."
                && p != ".."
                && p.bytes()
                    .next()
                    .is_some_and(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b)),
            "Invalid portable asset component"
        );
    }
    Ok(())
}

pub fn id(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 60
            && value.as_bytes()[0].is_ascii_lowercase()
            && value
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c)),
        "Invalid stable ID"
    );
    Ok(())
}
pub fn fields(v: &Value, required: &[&str], optional: &[&str]) -> Result<()> {
    let map = v.as_object().ok_or_else(|| eyre!("Expected JSON object"))?;
    ensure!(
        required.iter().all(|k| map.contains_key(*k))
            && map
                .keys()
                .all(|k| required.contains(&k.as_str()) || optional.contains(&k.as_str())),
        "Missing or unsupported schema fields"
    );
    Ok(())
}
pub fn number(v: &Value, low: f64, high: f64) -> Result<f64> {
    let n = v.as_f64().ok_or_else(|| eyre!("Expected finite number"))?;
    ensure!(
        n.is_finite() && (low..=high).contains(&n),
        "Number outside bounds {low}..{high}"
    );
    Ok(n)
}
pub fn size(v: &Value, limit: u32) -> Result<[u32; 2]> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 2)
        .ok_or_else(|| eyre!("Expected [width,height]"))?;
    let mut result = [0; 2];
    for (i, v) in a.iter().enumerate() {
        let n = v
            .as_u64()
            .ok_or_else(|| eyre!("Dimensions must be integers"))?;
        ensure!((1..=limit as u64).contains(&n), "Dimension outside bounds");
        result[i] = n as u32;
    }
    Ok(result)
}
pub fn rgba(v: &Value) -> Result<[u8; 4]> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 4)
        .ok_or_else(|| eyre!("Expected RGBA"))?;
    let mut out = [0; 4];
    for (i, n) in a.iter().enumerate() {
        let n = n.as_u64().ok_or_else(|| eyre!("Expected 8-bit channel"))?;
        ensure!(n <= 255, "Channel outside bounds");
        out[i] = n as u8;
    }
    Ok(out)
}
pub fn center(v: &Value, canvas: [u32; 2]) -> Result<[f64; 2]> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 2)
        .ok_or_else(|| eyre!("Expected center [x,y]"))?;
    Ok([
        number(&a[0], 0.0, canvas[0] as f64)?,
        number(&a[1], 0.0, canvas[1] as f64)?,
    ])
}
pub fn hash(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "Invalid SHA256"
    );
    Ok(())
}

// Deserialize through a budgeted visitor to reject duplicate keys before map insertion.
struct JsonSeed<'a> {
    depth: usize,
    nodes: &'a mut usize,
}
impl<'de> DeserializeSeed<'de> for JsonSeed<'_> {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        d: D,
    ) -> std::result::Result<Value, D::Error> {
        *self.nodes += 1;
        if self.depth > 32 || *self.nodes > 32768 {
            return Err(serde::de::Error::custom("JSON depth/node budget exceeded"));
        }
        // RawValue preserves the real token kind. deserialize_any with arbitrary_precision
        // represents numbers using a private map sentinel, which a literal object could spoof.
        let raw = <&RawValue>::deserialize(d)?;
        self.raw(raw.get()).map_err(serde::de::Error::custom)
    }
}
impl JsonSeed<'_> {
    fn raw(self, raw: &str) -> std::result::Result<Value, serde_json::Error> {
        use serde::Deserializer;
        let mut d = serde_json::Deserializer::from_str(raw);
        let value = match raw.as_bytes()[0] {
            b'{' => d.deserialize_map(self)?,
            b'[' => d.deserialize_seq(self)?,
            b'"' => {
                let v = String::deserialize(&mut d)?;
                if v.len() > 256 * 1024 {
                    return Err(serde::de::Error::custom("JSON string budget exceeded"));
                }
                Value::String(v)
            }
            b'-' | b'0'..=b'9' => {
                let n = if raw.bytes().any(|b| b".eE".contains(&b)) {
                    let f: f64 = raw.parse().map_err(serde::de::Error::custom)?;
                    Number::from_f64(f)
                        .ok_or_else(|| serde::de::Error::custom("Nonfinite number"))?
                } else {
                    // Python int(-0) is 0; all other integer digits remain exact.
                    serde_json::from_str::<Number>(if raw == "-0" { "0" } else { raw })?
                };
                return Ok(Value::Number(n));
            }
            b't' | b'f' => Value::Bool(bool::deserialize(&mut d)?),
            b'n' => {
                <()>::deserialize(&mut d)?;
                Value::Null
            }
            _ => return Err(serde::de::Error::custom("Invalid JSON token")),
        };
        d.end()?;
        Ok(value)
    }
}
impl<'de> Visitor<'de> for JsonSeed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("bounded JSON")
    }
    fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<Value, E> {
        Ok(json!(v))
    }
    fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Value, E> {
        Ok(json!(v))
    }
    fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Value, E> {
        Ok(json!(v))
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Value, E> {
        if !v.is_finite() {
            return Err(E::custom("Nonfinite number"));
        }
        Ok(json!(v))
    }
    fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Value, E> {
        if v.len() > 256 * 1024 {
            return Err(E::custom("JSON string budget exceeded"));
        }
        Ok(json!(v))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> std::result::Result<Value, A::Error> {
        let mut v = vec![];
        while let Some(item) = a.next_element_seed(JsonSeed {
            depth: self.depth + 1,
            nodes: self.nodes,
        })? {
            v.push(item);
        }
        Ok(Value::Array(v))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> std::result::Result<Value, A::Error> {
        let mut v = serde_json::Map::new();
        while let Some(k) = a.next_key::<String>()? {
            *self.nodes += 1;
            if *self.nodes > 32768 || k.len() > 256 * 1024 {
                return Err(serde::de::Error::custom("JSON key/node budget exceeded"));
            }
            if v.contains_key(&k) {
                return Err(serde::de::Error::custom("Duplicate JSON key"));
            }
            v.insert(
                k,
                a.next_value_seed(JsonSeed {
                    depth: self.depth + 1,
                    nodes: self.nodes,
                })?,
            );
        }
        Ok(Value::Object(v))
    }
}
pub fn decode_json(raw: &[u8]) -> Result<Value> {
    ensure!(raw.len() <= JSON_LIMIT, "JSON byte budget exceeded");
    decode_json_values(raw)
}
fn decode_json_values(raw: &[u8]) -> Result<Value> {
    let mut d = serde_json::Deserializer::from_slice(raw);
    let result = JsonSeed {
        depth: 0,
        nodes: &mut 0,
    }
    .deserialize(&mut d)?;
    d.end()?;
    ensure!(result.is_object(), "JSON root must be object");
    Ok(result)
}

pub(super) fn png(bytes: &[u8], expected: [u32; 2]) -> Result<RgbaImage> {
    ensure!(
        bytes.len() >= 33
            && bytes[..8] == *b"\x89PNG\r\n\x1a\n"
            && bytes[8..16] == *b"\0\0\0\rIHDR"
            && bytes[24..28] == [8, 6, 0, 0]
            && bytes[28] <= 1,
        "Asset must be encoded RGBA8 PNG"
    );
    let u32_at = |i| u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap());
    ensure!(
        [u32_at(16), u32_at(20)] == expected,
        "PNG declared dimensions mismatch"
    );
    let (mut offset, mut count, mut idat, mut closed, mut palette) = (8, 0, false, false, false);
    let mut compressed = Vec::new();
    while offset < bytes.len() {
        count += 1;
        ensure!(
            count <= 4096 && bytes.len() - offset >= 12,
            "PNG chunk budget/framing error"
        );
        let len = u32_at(offset) as usize;
        ensure!(len <= bytes.len() - offset - 12, "PNG truncated chunk");
        let kind = &bytes[offset + 4..offset + 8];
        let end = offset + 8 + len;
        ensure!(
            kind.iter().all(u8::is_ascii_alphabetic) && kind[2].is_ascii_uppercase(),
            "Invalid PNG chunk type"
        );
        ensure!(
            crc32fast::hash(&bytes[offset + 4..end]) == u32_at(end),
            "PNG CRC mismatch"
        );
        match kind {
            b"IHDR" => ensure!(count == 1 && len == 13, "Duplicate/misplaced PNG IHDR"),
            _ if count == 1 => return Err(eyre!("Missing PNG IHDR")),
            b"acTL" | b"fcTL" | b"fdAT" => return Err(eyre!("Animated PNG forbidden")),
            b"IDAT" => {
                ensure!(!closed, "Noncontiguous PNG IDAT");
                idat = true;
                compressed.push(&bytes[offset + 8..end]);
            }
            b"IEND" => {
                ensure!(
                    len == 0 && idat && end + 4 == bytes.len(),
                    "Invalid terminal PNG IEND"
                );
                break;
            }
            b"PLTE" => {
                ensure!(
                    !idat && !palette && (3..=768).contains(&len) && len % 3 == 0,
                    "Invalid PNG palette"
                );
                palette = true;
            }
            _ => {
                ensure!(kind[0].is_ascii_lowercase(), "Unknown critical PNG chunk");
                if idat {
                    closed = true;
                }
            }
        }
        offset = end + 4;
    }
    ensure!(
        offset < bytes.len() && &bytes[offset + 4..offset + 8] == b"IEND",
        "Missing PNG IEND"
    );
    verify_stream(&compressed, expected, bytes[28] == 1)?;
    let mut decoder =
        png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: 480_000_000 });
    decoder.ignore_checksums(false);
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    let mut reader = decoder.read_info()?;
    ensure!(
        reader.info().color_type == png::ColorType::Rgba
            && reader.info().bit_depth == png::BitDepth::Eight
            && reader.info().animation_control.is_none(),
        "PNG mode/frame mismatch"
    );
    let mut data = vec![0; expected[0] as usize * expected[1] as usize * 4];
    let info = reader.next_frame(&mut data)?;
    reader.finish()?;
    ensure!(
        info.buffer_size() == data.len(),
        "PNG decoded size mismatch"
    );
    RgbaImage::from_raw(expected[0], expected[1], data).ok_or_else(|| eyre!("Invalid decoded PNG"))
}

/// png::Reader may finish after pixels even if the zlib trailer is missing.
/// Check complete zlib/Adler integrity on retained bytes, using bounded discarded output.
fn verify_stream(chunks: &[&[u8]], size: [u32; 2], interlaced: bool) -> Result<()> {
    let (w, h) = (size[0] as u64, size[1] as u64);
    let expected = if interlaced {
        [
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ]
        .iter()
        .map(|&(x, y, dx, dy)| {
            if w <= x || h <= y {
                0
            } else {
                let pw = (w - x).div_ceil(dx);
                let ph = (h - y).div_ceil(dy);
                ph * (4 * pw + 1)
            }
        })
        .sum()
    } else {
        h * (4 * w + 1)
    };
    let mut inflater = flate2::Decompress::new(true);
    let mut output = [0_u8; 65536];
    for chunk in chunks {
        for piece in chunk.chunks(65536) {
            let mut pending = piece;
            loop {
                let (before_in, before_out) = (inflater.total_in(), inflater.total_out());
                let allowance =
                    (expected - inflater.total_out() + 1).min(output.len() as u64) as usize;
                let status = inflater.decompress(
                    pending,
                    &mut output[..allowance],
                    flate2::FlushDecompress::None,
                )?;
                let (read, written) = (
                    inflater.total_in() - before_in,
                    inflater.total_out() - before_out,
                );
                ensure!(
                    inflater.total_out() <= expected,
                    "PNG inflated bytes exceed RGBA scanline geometry"
                );
                if status == flate2::Status::StreamEnd {
                    ensure!(
                        inflater.total_out() == expected,
                        "PNG inflated scanline count mismatch"
                    );
                    return Ok(());
                }
                pending = &pending[read as usize..];
                if read == 0 && written == 0 {
                    ensure!(pending.is_empty(), "PNG stream made no progress");
                    break;
                }
                if pending.is_empty() && written < allowance as u64 {
                    break;
                }
            }
        }
    }
    Err(eyre!("PNG zlib stream incomplete or missing Adler trailer"))
}

pub fn load_scene(package: &Package) -> Result<Scene> {
    let raw = package.read("scene.json", JSON_LIMIT)?;
    let m = decode_json(&raw)?;
    fields(
        &m,
        &[
            "version",
            "role",
            "scene_id",
            "origin",
            "compositing",
            "canvas",
            "plan",
            "layout_fingerprint",
            "background",
            "objects",
            "assets",
        ],
        &["parent", "previews"],
    )?;
    ensure!(
        m["version"].as_u64() == Some(1)
            && m["role"] == "motion-scene-sidecar"
            && m["compositing"] == "straight-alpha-over-srgb8-depth-ascending",
        "Unsupported scene contract"
    );
    id(m["scene_id"]
        .as_str()
        .ok_or_else(|| eyre!("Invalid scene ID"))?)?;
    ensure!(
        matches!(
            m["origin"].as_str(),
            Some("procedural-new-art-v1" | "manual-independent-rgba")
        ),
        "Unsupported scene origin; flattened art cannot be rearranged"
    );
    let plan = normalize(&m["plan"])?;
    let fingerprint = sha256(canonical(&plan).as_bytes());
    ensure!(
        m["layout_fingerprint"] == fingerprint,
        "Scene layout fingerprint mismatch"
    );
    let canvas = size(&m["canvas"], 32768)?;
    ensure!(
        m["canvas"] == plan["master_pixels"] && canvas[0] as u64 * canvas[1] as u64 <= 80_000_000,
        "Canvas/layout mismatch"
    );
    let descriptors = m["assets"]
        .as_object()
        .ok_or_else(|| eyre!("Missing assets"))?;
    ensure!(
        (4..=8).contains(&descriptors.len()),
        "Asset count outside bounds"
    );
    let (mut bytes, mut pixels) = (raw.len(), 0_u64);
    let mut files = HashSet::new();
    let mut images = BTreeMap::new();
    for (key, descriptor) in descriptors {
        id(key)?;
        fields(descriptor, &["file", "sha256", "size"], &[])?;
        let file = descriptor["file"]
            .as_str()
            .ok_or_else(|| eyre!("Missing asset file"))?;
        ensure!(
            ![
                "scene.json",
                "manifest.json",
                "parent-manifest.json",
                "arrangements.json",
                "outputs.json"
            ]
            .contains(&file)
                && files.insert(file),
            "Reserved or duplicate asset path"
        );
        let expected = size(&descriptor["size"], 32768)?;
        pixels += expected[0] as u64 * expected[1] as u64;
        ensure!(pixels <= 120_000_000, "Decoded pixel budget exceeded");
        let data = package.read(file, ASSET_LIMIT)?;
        bytes += data.len();
        ensure!(bytes <= TOTAL_LIMIT, "Aggregate package budget exceeded");
        let expected_hash = descriptor["sha256"]
            .as_str()
            .ok_or_else(|| eyre!("Invalid PNG hash"))?;
        hash(expected_hash)?;
        ensure!(sha256(&data) == expected_hash, "PNG hash mismatch");
        images.insert(key.clone(), Rc::new(png(&data, expected)?));
    }
    let bg = &m["background"];
    let mut used = HashSet::new();
    let background = if bg.get("solid").is_some() {
        fields(bg, &["solid"], &[])?;
        Background::Solid(rgba(&bg["solid"])?)
    } else {
        fields(bg, &["asset"], &[])?;
        let asset = bg["asset"]
            .as_str()
            .ok_or_else(|| eyre!("Invalid background"))?;
        let image = images
            .get(asset)
            .ok_or_else(|| eyre!("Missing background asset"))?;
        ensure!(
            [image.width(), image.height()] == canvas,
            "Background canvas mismatch"
        );
        used.insert(asset.to_string());
        Background::Asset(asset.into())
    };
    let records = m["objects"]
        .as_array()
        .filter(|v| v.len() == 4)
        .ok_or_else(|| eyre!("Exactly four independent objects required"))?;
    let (mut ids, mut depths) = (HashSet::new(), HashSet::new());
    let mut objects = vec![];
    for obj in records {
        fields(
            obj,
            &[
                "id", "asset", "material", "color", "alpha", "glow", "depth", "size", "center",
            ],
            &[],
        )?;
        let key = obj["id"]
            .as_str()
            .ok_or_else(|| eyre!("Invalid object ID"))?;
        id(key)?;
        ensure!(ids.insert(key), "Duplicate object ID");
        let asset = obj["asset"]
            .as_str()
            .ok_or_else(|| eyre!("Missing object asset"))?;
        let image = images
            .get(asset)
            .ok_or_else(|| eyre!("Missing object PNG"))?;
        ensure!(
            used.insert(asset.to_string()) && obj["material"] == "luminous-translucent-circle",
            "Objects require independent luminous circle layers"
        );
        ensure!(
            rgba(&obj["color"])?[3] == 255,
            "Color opacity is baked separately"
        );
        number(&obj["alpha"], 0.0, 1.0)?;
        fields(&obj["glow"], &["radius", "alpha"], &[])?;
        number(&obj["glow"]["radius"], 0.0, 1024.0)?;
        number(&obj["glow"]["alpha"], 0.0, 1.0)?;
        let depth = obj["depth"]
            .as_u64()
            .filter(|n| *n <= 255)
            .ok_or_else(|| eyre!("Invalid depth"))?;
        ensure!(depths.insert(depth), "Duplicate object depth");
        let dimensions = size(&obj["size"], 4096)?;
        ensure!(
            dimensions[0] == dimensions[1] && dimensions == [image.width(), image.height()],
            "Object size must equal square PNG dimensions"
        );
        // Initial centers remain validated scene provenance; playback consumes authored poses.
        center(&obj["center"], canvas)?;
        objects.push(Object {
            id: key.into(),
            asset: asset.into(),
            size: dimensions,
            depth: depth as u8,
        });
    }
    if let Some(previews) = m.get("previews") {
        if previews.as_object().is_none_or(|v| !v.is_empty()) {
            fields(previews, &["initial", "proof", "contact"], &[])?;
            for role in ["initial", "proof", "contact"] {
                let key = previews[role]
                    .as_str()
                    .ok_or_else(|| eyre!("Invalid preview reference"))?;
                ensure!(
                    images.contains_key(key) && used.insert(key.into()),
                    "Invalid/shared preview asset"
                );
            }
            let image = &images[previews["initial"].as_str().unwrap()];
            ensure!(
                [image.width(), image.height()] == canvas,
                "Initial preview canvas mismatch"
            );
        }
    }
    ensure!(used.len() == images.len(), "Unreferenced assets");
    if let Some(parent) = m.get("parent") {
        fields(
            parent,
            &["file", "sha256", "run_id", "layout_fingerprint"],
            &[],
        )?;
        ensure!(
            parent["file"] == "parent-manifest.json",
            "Invalid parent path"
        );
        let raw = package.read("parent-manifest.json", JSON_LIMIT)?;
        bytes += raw.len();
        ensure!(bytes <= TOTAL_LIMIT, "Parent package budget exceeded");
        ensure!(
            parent["sha256"] == sha256(&raw),
            "Parent bytes hash mismatch"
        );
        let evidence = decode_parent(&raw)?;
        let parent_plan = normalize(&evidence["plan"])?;
        ensure!(
            evidence["version"].as_u64() == Some(4)
                && canonical(&parent_plan) == canonical(&plan)
                && sha256(canonical(&parent_plan).as_bytes()) == fingerprint
                && evidence["layout_fingerprint"] == fingerprint
                && parent["layout_fingerprint"] == fingerprint
                && evidence["source_size"] == plan["generation"]
                && evidence["final_input_size"] == m["canvas"]
                && evidence["run_id"] == parent["run_id"],
            "Parent provenance/layout mismatch"
        );
        let run = parent["run_id"]
            .as_str()
            .ok_or_else(|| eyre!("Invalid parent run ID"))?;
        ensure!(
            run.len() == 10
                && run
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
            "Invalid parent run ID"
        );
    }
    objects.sort_by_key(|obj| obj.depth);
    Ok(Scene {
        sha256: sha256(&raw),
        manifest: m,
        canvas,
        objects,
        images,
        background,
    })
}

/// Legacy v4 workflow NaN is opaque provenance. Accept it only within that root field.
pub(super) fn decode_parent(raw: &[u8]) -> Result<Value> {
    ensure!(raw.len() <= JSON_LIMIT, "Parent JSON byte budget exceeded");
    // Locate the value with the JSON tokenizer; strings and nested keys cannot select it.
    let mut i = 0;
    let mut depth: usize = 0;
    let mut workflow = false;
    let mut opaque_depth = None;
    let mut out = Vec::with_capacity(raw.len());
    while i < raw.len() {
        match raw[i] {
            b'"' => {
                let start = i;
                i += 1;
                while i < raw.len() {
                    if raw[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if raw[i] == b'"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                ensure!(i <= raw.len(), "Invalid parent string");
                let token = &raw[start..i];
                if depth == 1 && opaque_depth.is_none() {
                    let mut j = i;
                    while j < raw.len() && raw[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if raw.get(j) == Some(&b':') {
                        workflow =
                            serde_json::from_slice::<String>(token)? == "generation_workflow";
                    }
                }
                out.extend_from_slice(token);
            }
            b'{' | b'[' => {
                depth += 1;
                if workflow && opaque_depth.is_none() {
                    opaque_depth = Some(depth);
                    workflow = false;
                }
                out.push(raw[i]);
                i += 1;
            }
            b'}' | b']' => {
                if opaque_depth == Some(depth) {
                    opaque_depth = None;
                }
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| eyre!("Invalid parent JSON nesting"))?;
                out.push(raw[i]);
                i += 1;
            }
            b'N' if (opaque_depth.is_some() || (workflow && depth == 1))
                && raw.get(i..i + 3) == Some(b"NaN") =>
            {
                out.extend_from_slice(b"null");
                i += 3;
                workflow = false;
            }
            _ => {
                out.push(raw[i]);
                i += 1;
            }
        }
    }
    decode_json_values(&out)
}
