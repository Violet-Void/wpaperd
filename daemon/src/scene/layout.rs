//! Bounded port of the producer core.py v2 normalization and Python JSON fingerprint.
//! Never trust persisted crop boxes merely because their fingerprint was refreshed.
use color_eyre::{eyre::ensure, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::loader::{number, size};

pub fn sha256(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

/// Python json.dumps(..., ensure_ascii=True, sort_keys=True, separators=(",", ":")).
/// Preserve integer/float identity, Python exponent thresholds and UTF-16 escapes.
pub fn canonical(value: &Value) -> String {
    canonical_text(value, true)
}
fn canonical_text(value: &Value, ascii: bool) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(v) => v.to_string(),
        Value::Number(n) if n.is_f64() => python_float(n.as_f64().unwrap()),
        Value::Number(n) => n.to_string(),
        Value::String(v) => {
            if !ascii {
                return serde_json::to_string(v).unwrap();
            }
            let mut result = String::new();
            for c in serde_json::to_string(v).unwrap().chars() {
                if (c as u32) < 127 {
                    result.push(c);
                } else {
                    for code in c.encode_utf16(&mut [0; 2]).iter() {
                        result.push_str(&format!("\\u{code:04x}"));
                    }
                }
            }
            result
        }
        Value::Array(v) => format!(
            "[{}]",
            v.iter()
                .map(|v| canonical_text(v, ascii))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(v) => {
            let mut items = v.iter().collect::<Vec<_>>();
            items.sort_by_key(|(key, _)| *key);
            format!(
                "{{{}}}",
                items
                    .into_iter()
                    .map(|(k, v)| format!(
                        "{}:{}",
                        canonical_text(&Value::String(k.clone()), ascii),
                        canonical_text(v, ascii)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn python_float(v: f64) -> String {
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    let raw = serde_json::to_string(&v).unwrap();
    let (sign, raw) = if let Some(raw) = raw.strip_prefix('-') {
        ("-", raw)
    } else {
        ("", raw.as_str())
    };
    let (mantissa, power) = raw
        .split_once('e')
        .map(|(m, e)| (m, e.parse::<i32>().unwrap()))
        .unwrap_or((raw, 0));
    let decimal = mantissa.find('.').unwrap_or(mantissa.len()) as i32;
    let mut digits = mantissa.replace('.', "");
    let leading = digits.bytes().take_while(|b| *b == b'0').count();
    digits.drain(..leading);
    let exponent = power + decimal - leading as i32 - 1;
    while digits.ends_with('0') {
        digits.pop();
    }
    if !(-4..16).contains(&exponent) {
        let tail = if digits.len() > 1 {
            format!(".{}", &digits[1..])
        } else {
            String::new()
        };
        return format!(
            "{sign}{}{tail}e{}{e:02}",
            &digits[..1],
            if exponent < 0 { "-" } else { "+" },
            e = exponent.abs()
        );
    }
    let point = exponent + 1;
    let body = if point <= 0 {
        format!("0.{}{digits}", "0".repeat((-point) as usize))
    } else if point as usize >= digits.len() {
        format!("{digits}{}.0", "0".repeat(point as usize - digits.len()))
    } else {
        format!(
            "{}.{}",
            &digits[..point as usize],
            &digits[point as usize..]
        )
    };
    format!("{sign}{body}")
}

fn round(v: f64) -> i64 {
    v.round_ties_even() as i64
}
fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
fn isqrt(v: u64) -> u64 {
    let mut n = (v as f64).sqrt() as u64;
    while (n + 1) * (n + 1) <= v {
        n += 1;
    }
    while n * n > v {
        n -= 1;
    }
    n
}

pub fn normalize(plan: &Value) -> Result<Value> {
    layout_budget(
        &json!({"name":plan["name"],"source":plan["source"],"coordinate_space":plan["coordinate_space"],"monitors":plan["monitors"]}),
    )?;
    ensure!(
        plan["version"].as_u64() == Some(2),
        "Unsupported layout plan"
    );
    let generation = size(&plan["generation"], 8192)?;
    let target = (generation[0] as u64 * generation[1] as u64) as f64;
    ensure!(
        (250_000.0..=4_000_000.0).contains(&target),
        "Generation area outside bounds"
    );
    let margin = number(&plan["safe_margin"], 0.0, 0.3)?;
    ensure!(
        matches!(
            plan["coordinate_space"].as_str(),
            Some("logical" | "physical_mm")
        ),
        "Unsupported coordinate space"
    );
    ensure!(
        plan["name"].is_string() && plan["source"].is_string(),
        "Layout name/source required"
    );
    let mut monitors = plan["monitors"]
        .as_array()
        .cloned()
        .ok_or_else(|| color_eyre::eyre::eyre!("Missing layout monitors"))?;
    ensure!(
        (1..=6).contains(&monitors.len()),
        "Expected one to six monitors"
    );
    let mut names = std::collections::HashSet::new();
    for m in &mut monitors {
        let id = m["id"].as_str().unwrap_or("");
        ensure!(
            !id.is_empty()
                && id.len() <= 60
                && id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c)),
            "Invalid monitor ID"
        );
        let folded = id.to_ascii_lowercase();
        ensure!(
            !["manifest", "master", "native", "proof"].contains(&folded.as_str())
                && names.insert(folded),
            "Duplicate/reserved monitor ID"
        );
        let pixels = size(&m["pixels"], 16384)?;
        ensure!(pixels.iter().all(|n| *n >= 64), "Monitor raster too small");
        let logical = m["logical"]
            .as_array()
            .filter(|v| v.len() == 2)
            .ok_or_else(|| color_eyre::eyre::eyre!("Missing monitor logical size"))?;
        let lw = number(&logical[0], 1.0, 100000.0)?;
        let lh = number(&logical[1], 1.0, 100000.0)?;
        number(&m["x"], -100000.0, 100000.0)?;
        number(&m["y"], -100000.0, 100000.0)?;
        let rx = pixels[0] as f64 / lw;
        let ry = pixels[1] as f64 / lh;
        ensure!(
            (rx / ry - 1.0).abs() <= 0.002,
            "Monitor rectangle/raster aspect mismatch"
        );
        m.as_object_mut()
            .ok_or_else(|| color_eyre::eyre::eyre!("Invalid monitor"))?
            .insert("density".into(), json!((rx + ry) / 2.0));
    }
    monitors.sort_by(|a, b| {
        a["x"]
            .as_f64()
            .unwrap()
            .partial_cmp(&b["x"].as_f64().unwrap())
            .unwrap()
            .then(
                a["y"]
                    .as_f64()
                    .unwrap()
                    .partial_cmp(&b["y"].as_f64().unwrap())
                    .unwrap(),
            )
            .then(a["id"].as_str().cmp(&b["id"].as_str()))
    });
    let val = |m: &Value, k: &str| m[k].as_f64().unwrap();
    let lw = |m: &Value| m["logical"][0].as_f64().unwrap();
    let lh = |m: &Value| m["logical"][1].as_f64().unwrap();
    for (i, a) in monitors.iter().enumerate() {
        for b in monitors.iter().skip(i + 1) {
            ensure!(
                !((val(a, "x") + lw(a)).min(val(b, "x") + lw(b)) > val(a, "x").max(val(b, "x"))
                    && (val(a, "y") + lh(a)).min(val(b, "y") + lh(b))
                        > val(a, "y").max(val(b, "y"))),
                "Overlapping monitor rectangles"
            );
        }
    }
    let minx = monitors
        .iter()
        .map(|m| val(m, "x"))
        .reduce(|a, b| if b < a { b } else { a })
        .unwrap();
    let miny = monitors
        .iter()
        .map(|m| val(m, "y"))
        .reduce(|a, b| if b < a { b } else { a })
        .unwrap();
    let w = monitors
        .iter()
        .map(|m| val(m, "x") + lw(m))
        .reduce(|a, b| if b > a { b } else { a })
        .unwrap()
        - minx;
    let h = monitors
        .iter()
        .map(|m| val(m, "y") + lh(m))
        .reduce(|a, b| if b > a { b } else { a })
        .unwrap()
        - miny;
    let density = monitors
        .iter()
        .map(|m| val(m, "density"))
        .reduce(f64::max)
        .unwrap();
    let mut mw = round(w * density) as u64;
    let mut mh = round(h * density) as u64;
    ensure!(
        mw > 0 && mh > 0 && mw.max(mh) <= 32768 && mw * mh <= 80_000_000,
        "Master canvas exceeds bounds"
    );
    for m in &mut monitors {
        let x = val(m, "x");
        let y = val(m, "y");
        let (width, height) = (lw(m), lh(m));
        let b = [
            round((x - minx) * density),
            round((y - miny) * density),
            round((x + width - minx) * density),
            round((y + height - miny) * density),
        ];
        ensure!(b[2] > b[0] && b[3] > b[1], "Empty crop");
        let bw = b[2] - b[0];
        let bh = b[3] - b[1];
        m["crop_box"] = json!(b);
        m["normalized_box"] = json!([
            (x - minx) / w,
            (y - miny) / h,
            (x + width - minx) / w,
            (y + height - miny) / h
        ]);
        m["safe_box"] = json!([
            b[0] + round(bw as f64 * margin),
            b[1] + round(bh as f64 * margin),
            b[2] - round(bw as f64 * margin),
            b[3] - round(bh as f64 * margin)
        ]);
    }
    let common = gcd(mw, mh);
    let (n, d) = (mw / common, mh / common);
    let (sn, sd) = (16 / gcd(n, 16), 16 / gcd(d, 16));
    let step = sn * sd / gcd(sn, sd);
    let ppm = n * d;
    let mut min_area = isqrt(250_000_u64.div_ceil(ppm));
    if min_area * min_area * ppm < 250_000 {
        min_area += 1;
    }
    let minimum = 64_u64
        .div_ceil(n)
        .max(64_u64.div_ceil(d))
        .max(min_area)
        .max(1);
    let maximum = (8192 / n).min(8192 / d).min(isqrt(4_000_000 / ppm));
    let first = minimum.div_ceil(step) * step;
    let last = maximum / step * step;
    let (gw, gh, padding) = if first <= last {
        let ideal = (target / ppm as f64).sqrt();
        let lower = first.max((ideal / step as f64).floor() as u64 * step);
        let mut candidates = vec![lower, last.min(lower + step)];
        candidates.retain(|c| first <= *c && *c <= last);
        candidates.sort();
        candidates.dedup();
        let scores = candidates
            .iter()
            .map(|c| ((ppm * c * c) as f64 / target).ln().abs())
            .collect::<Vec<_>>();
        let closest = scores.iter().copied().reduce(f64::min).unwrap();
        let c = *candidates
            .iter()
            .zip(scores)
            .filter(|(_, s)| {
                (*s - closest).abs() <= 1e-12_f64.max(1e-12 * s.abs().max(closest.abs()))
            })
            .map(|(c, _)| c)
            .min()
            .unwrap();
        (n * c, d * c, None)
    } else {
        let mut candidates = Vec::new();
        for scale in 1..=512_u64 {
            let gw = 64.max(mw.div_ceil(16 * scale) * 16);
            let gh = 64.max(mh.div_ceil(16 * scale) * 16);
            let (pw, ph) = (gw * scale, gh * scale);
            if gw > 8192
                || gh > 8192
                || !(250_000..=4_000_000).contains(&(gw * gh))
                || pw.max(ph) > 32768
                || pw * ph > 80_000_000
            {
                continue;
            }
            candidates.push((pw * ph - mw * mh, pw, ph, gw, gh, scale));
        }
        candidates.sort();
        let best = *candidates
            .first()
            .ok_or_else(|| color_eyre::eyre::eyre!("Unsupported padded canvas"))?;
        let chosen = candidates
            .iter()
            .filter(|c| (c.0, c.1, c.2) == (best.0, best.1, best.2))
            .min_by(|a, b| {
                ((a.3 * a.4) as f64 / target)
                    .ln()
                    .abs()
                    .total_cmp(&((b.3 * b.4) as f64 / target).ln().abs())
                    .then((a.3, a.4).cmp(&(b.3, b.4)))
            })
            .unwrap();
        let padding = [0, 0, best.1 - mw, best.2 - mh];
        let content = [mw, mh];
        (mw, mh) = (best.1, best.2);
        for m in &mut monitors {
            let b = m["crop_box"].as_array().unwrap();
            m["normalized_box"] = json!([
                b[0].as_f64().unwrap() / mw as f64,
                b[1].as_f64().unwrap() / mh as f64,
                b[2].as_f64().unwrap() / mw as f64,
                b[3].as_f64().unwrap() / mh as f64
            ]);
        }
        (chosen.3, chosen.4, Some((content, padding)))
    };
    let preserve_number = |v: f64, inputs: Vec<&Value>| {
        if inputs.iter().all(|v| !v.is_f64()) {
            json!(v as i64)
        } else {
            json!(v)
        }
    };
    let origin_x = monitors.iter().find(|m| val(m, "x") == minx).unwrap()["x"].clone();
    let origin_y = monitors.iter().find(|m| val(m, "y") == miny).unwrap()["y"].clone();
    let maxxm = monitors
        .iter()
        .reduce(|a, b| {
            if val(b, "x") + lw(b) > val(a, "x") + lw(a) {
                b
            } else {
                a
            }
        })
        .unwrap();
    let maxym = monitors
        .iter()
        .reduce(|a, b| {
            if val(b, "y") + lh(b) > val(a, "y") + lh(a) {
                b
            } else {
                a
            }
        })
        .unwrap();
    let logical_w = preserve_number(w, vec![&maxxm["x"], &maxxm["logical"][0], &origin_x]);
    let logical_h = preserve_number(h, vec![&maxym["y"], &maxym["logical"][1], &origin_y]);
    let mut normalized = json!({"version":2,"name":plan["name"],"source":plan["source"],"coordinate_space":plan["coordinate_space"],"origin":[origin_x,origin_y],"logical_canvas":[logical_w,logical_h],"density":density,"master_pixels":[mw,mh],"generation":[gw,gh],"safe_margin":plan["safe_margin"],"monitors":monitors,"note":"Raster sizes are upright/oriented. No additional rotation is applied. Logical alignment is not physical bezel/DPI calibration."});
    if let Some((content, padding)) = padding {
        normalized["content_pixels"] = json!(content);
        normalized["canvas_padding"] = json!(padding);
    }
    ensure!(
        canonical(&normalized) == canonical(plan),
        "Layout differs from recomputed core.py normalized geometry"
    );
    Ok(normalized)
}

fn layout_budget(raw: &Value) -> Result<()> {
    let mut pending = vec![(raw, 0)];
    let (mut nodes, mut payload) = (0, 0);
    while let Some((value, depth)) = pending.pop() {
        nodes += 1;
        ensure!(
            nodes <= 2048 && depth <= 32,
            "Layout node/depth budget exceeded"
        );
        let charge = match value {
            Value::Null => 4,
            Value::Bool(_) => 5,
            Value::Number(_) => canonical(value).len(),
            Value::String(v) => {
                ensure!(v.len() <= 8192, "Layout string budget exceeded");
                v.len()
            }
            Value::Array(v) => {
                pending.extend(v.iter().map(|v| (v, depth + 1)));
                2
            }
            Value::Object(v) => {
                for (k, v) in v {
                    ensure!(k.len() <= 8192, "Layout key budget exceeded");
                    payload += k.len();
                    pending.push((v, depth + 1));
                }
                2
            }
        };
        payload += charge;
        ensure!(payload <= 64 * 1024, "Layout byte budget exceeded");
    }
    ensure!(
        canonical_text(raw, false).len() <= 64 * 1024,
        "Layout encoded byte budget exceeded"
    );
    Ok(())
}
