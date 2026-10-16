# Native motion scenes

## TL;DR

> [!NOTE]
> Explicit `wpaperctl scene` commands load four independent RGBA layers and three producer-authored
> arrangements into the existing Wpaperd output renderer. A bound `outputs.json` maps each upright
> master crop to a named Wayland output, with the scene presented over an opaque black matte.
> Stop releases scene resources and resumes still wallpapers.

## Use it

The scene commands require matching daemon/client builds. The Rust `wpaperd-ipc` crate is version
2.0.0 because its public enums gained variants; existing still IPC wire messages retain their form.
After installing this version, run the ordinary Wpaperd daemon with its existing still configuration.
Prepare a separate portable package containing producer `scene.json`, `arrangements.json`, all
declared PNGs, any copied parent manifest, and the consumer `outputs.json` below. Use actual verified
output names, upright physical dimensions and transforms; do not copy another desktop's topology.

```bash
wpaperctl scene start /absolute/path/to/package
wpaperctl scene status --json
wpaperctl scene shuffle
wpaperctl scene previous
wpaperctl scene freeze
wpaperctl scene freeze
wpaperctl scene stop
```

Status reports `static`, `orbit`, `frozen`, `fallback`, or `stopped`, plus scene/arrangement hashes,
the current arrangement, bounded history depth and each mapped output's condition. Freeze toggles
the current pose and resumes the remaining transition on thaw. `start --reduced-motion` changes
arrangements immediately. Neither mode runs an automatic animation at rest.

Shuffle deliberately visits the three arrangements in their declared order. Previous consumes up to
32 preceding requested arrangements. Commands during Orbit retarget from the current sampled pose;
there is no queued animation backlog. Commands while frozen retain the sampled position until thaw.
An unsafe retarget fails and leaves the preceding transition intact.
Scene requests share one pending frame callback per surface; callbacks already queued in still mode
drain before a new scene callback is requested. Repeated commands replace the target without creating
parallel callback chains. Grouped still invalidations use the same tracker before activation and
skip mapped scene outputs during playback.

Still navigation, set, reload and pause commands targeting mapped outputs require `scene stop` first.
This also applies to a failed scene in static fallback. Its still timer and any fresh image selection
remain held until mode-off; a timer already due in the activation dispatch cannot advance navigation.
Configuration duration reloads update the held remainder, including initially unset timers and an
exhausted zero remainder, without selecting an image behind the scene. A newly configured timer
retains the ordinary transition allowance. Watcher path changes defer one still selection until
explicit Stop; duration-only changes preserve a forced still detour.
Unmapped outputs retain their ordinary wallpaper operation. Still configuration, image pool, current
image links and explicit pause state retain their existing roles; scenes are never inferred from an
image filename or ordinary still command. Playback is opt-in through IPC and is not persisted across
daemon restarts. The renderer belongs to the daemon, independently of any bar.

## Output mapping

`outputs.json` is a versioned consumer sidecar with exactly these fields:

```json
{
  "version": 1,
  "role": "wpaperd-motion-outputs",
  "scene_sha256": "<SHA256 of the exact scene.json bytes>",
  "layout_fingerprint": "<producer core.py normalized layout fingerprint>",
  "outputs": [
    {
      "monitor_id": "<exact plan monitor ID>",
      "output": "<exact Wayland output name>",
      "pixels": [800, 600],
      "transform": "normal"
    }
  ]
}
```

Bind every plan monitor exactly once, using distinct output names. `pixels` equals the plan's upright
`pixels` and the configured surface's logical dimensions multiplied by its integer buffer scale.
Transforms are `normal`, `90`, `180`, `270`, `flipped`, `flipped90`, `flipped180`, `flipped270`; when the
plan declares a transform it must agree. Unsupported fractional-scale rounding or size mismatch
fails closed. Mapping selects the recomputed `crop_box`, retaining the upright master coordinate
space. Buffer transform compensation preserves its on-screen orientation; the art is not rotated a
second time in the master.

At least one mapped output must be present and configured to start. Absent outputs report `missing`;
their original crops remain reserved and existing outputs are not retiled. Returning outputs reuse
the shared controller clock and exact mapping. A size/transform mismatch never stretches or guesses.
At `scene start`, a mismatch on any present mapped output rejects the package and preserves the
current session or still. After activation, a mismatch stops animation for the whole scene,
restores still frames on all of its mapped outputs and reports `fallback`. Unmapped outputs stay in
still mode. Physical monitor placement and bezel calibration remain explicit producer-plan inputs,
not a discovery claim.

## Producer contract and composition

The consumer reads `scene.json` version 1 with role `motion-scene-sidecar`. Exactly four stable IDs
reference separate square RGBA8 PNGs. Background, palette/material declarations, object sizes,
depths, opacity and baked glow remain unchanged. The procedural origin is new art; parent linkage
preserves provenance and does not reconstruct a flattened parent's hidden pixels or intersections.

`arrangements.json` version 1 has role `motion-arrangements`, exact scene ID/byte-hash/layout binding,
`duration_ms: 1800`, `easing: cubic-in-out`, `orbit: {kind: perpendicular-sine, amplitude_pixels: ...}`,
four `circle_radii`, three distinct `{id, centers}` arrangements and these fixed constraints:
`max_overlap_fraction: 0.65`, `min_negative_space_fraction: 0.45`, `max_travel_fraction: 0.25`.
Every center map references the exact object IDs. Only their centers move. Orbit amplitude ranges
from zero through 10% of the smaller canvas axis; the procedural default uses
`round(0.06 * min(canvas), 3)`. Arrangement zero may differ from the scene's initial centers.

For raw phase `t` clamped to 0..1, eased phase `u` is `4*t^3` below one half and
`1 - (-2*t+2)^3/2` otherwise. Each position is `lerp(source, target, u)` plus its normalized
perpendicular displacement times `amplitude * sin(pi*u)`. Signs alternate in ascending depth order;
zero travel adds zero offset. Placements round `center - size/2` with ties to even.

The loader validates all resting poses and directed transitions at 61 raw phases `k/60`, matching
the producer's sampled constraints. Circle rims must stay in the canvas; lens overlap divided by
the smaller disc area is at most 0.65. The conservative negative-space lower bound is
`max(0, 1 - sum(pi*radius^2)/(width*height))`, at least 0.45. Sampled displacement from the transition
source divided by the canvas diagonal is at most 0.25. Retargets undergo the same extra checks.
This is bounded sampled validation, not a mathematical proof of continuous collision avoidance.
Overlap uses the published producer's dimensionless circular caps and small-angle series.
Entirely subnormal geometry is normalized by an exact power of two before measuring distance.
Very unequal radii resolve contacts and chord numerators with exact binary rational arithmetic,
using the already-resolved `num-rational 0.4.2` and `num-traits 0.2.19` crates. These routes retain
the positive-radius domain and fixed limits, including tiny and diagonal containment cases.

PNG source bytes are read once from pinned descriptors, verified and decoded. Textures upload the
unchanged straight RGBA channels. The wallpaper presents the producer's scene alpha-over over an
explicit opaque black matte, including translucent solid or asset backgrounds. The shader
premultiplies sampled RGB by its alpha and blends the background and four quads in ascending depth
with `ONE, ONE_MINUS_SRC_ALPHA`; the framebuffer remains opaque. A solid background clears to
`RGB * alpha` over black. This applies the matte without changing scene metadata or PNG channels.

Composition stays in 8-bit sRGB space; it does not add glow or opacity, convert to linear light or
rewrite a full master image on the CPU per frame. The reference is the producer's Pillow
`render_scene` RGBA result alpha-composited onto opaque black. GPU channel rounding and crop
filtering can differ from that reference and require visual verification. A parser regression input
uses background `[10,80,160,64]` with overlapping `[240,20,30,128]` and `[0,220,40,96]` sprites,
checking that translucent inputs remain accepted and decoded RGBA channels remain unchanged.
That parser check does not prove GPU compositing; the same case needs isolated visual comparison.

## Validation and recovery

The loader rejects symlinks in package ancestors and asset paths, traversal, nonregular/changing
files, duplicate JSON keys, unknown schema fields, nonfinite geometry, animated/non-RGBA8 PNGs,
hash mismatches and malformed PNG chunks/CRC/zlib/Adler framing. It recomputes producer v2 layout
geometry and requires identical Python-compatible canonical bytes, including numeric types and
signed zero, before comparing the fingerprint. Bounded opaque metadata integers preserve their
exact decimal digits. Literal serde private-key objects remain objects. A refreshed fingerprint
cannot certify an edited crop. Parent v4 workflow NaN remains opaque provenance only inside the
root `generation_workflow` field; geometry still requires finite JSON.

Producer limits remain 1 MiB + 1 byte per JSON, 32 JSON levels, 32768 nodes, 256 KiB strings,
8 assets, 64 MiB encoded per PNG, 128 MiB scene/parent bytes, 4096 PNG chunks, 120 million decoded
pixels, 32768 pixels per canvas axis and 80 million canvas pixels. Sprites are at most 4096 square.
The producer's core layout normalization separately bounds its input to 64 KiB, 2048 nodes,
depth 32 and 8192 UTF-8 bytes per string/key; the larger outer JSON limits do not replace these.
The two additional consumer sidecars each have the same bounded JSON limits. Stream validation
discards at most 64 KiB output per step and requires exact RGBA/Adam7 scanline geometry.

Start validates the whole package, present mappings and GPU resources before replacing a working
session. A rejected package preserves its current controller and still frame. After activation,
render or mapping failures stop scene animation, restore the existing still path and report
`fallback`; they do not schedule a scene restart loop. Explicit stop/start retries recovery. Stop
releases scene textures and decoded data, restores the prior still frame/pool and timer remainder,
and preserves an explicit user pause. Static/frozen playback requests no recurring frame callbacks.
If an EGL context cannot be made current, scene failure performs no GL draw or cleanup in another
output's context. A post-preflight activation binding failure reports an IPC error and enters
static fallback instead of returning success. Teardown releases GL handles before destroying a
successfully bound context;
on binding failure it disarms their Rust destructors and lets named EGL destruction reclaim them.
Texture construction owns its allocation before activation/upload, so failed uploads release new
textures in the bound context. Shader construction likewise owns unattached shaders through every
failure path. Error reporting consumes the original GL error without generating another GL query
error; subsequent valid operations remain usable. Stop and fallback redraw retained still textures
without selecting another image; a still load already pending at activation resumes separately.
An output with no prior still selection initializes its configured still image after mode-off.

Run repository checks without applying wallpaper:

```bash
cargo fmt --check --all
cargo check --all --locked
cargo test --all --locked
cargo build --all --locked
```

Parser/state tests include retained pure producer geometry fixtures with their source hash, exact
fingerprints including Unicode/DEL, padded layouts, equal integer/float extents and signed zero,
large integer metadata, canonical encoding forgery, malformed geometry/paths/PNGs, retargeting,
freeze/thaw, history, frame-request coalescing, injected context-switch/teardown failure and reduced
motion. They do not certify GPU fidelity, output hotplug, live
keeper performance or desktop rollout. Those require isolated runtime evidence and subsequent
authorized live acceptance. No compositor/service activation is performed by these tests.

The loader uses pinned [sha2](https://docs.rs/sha2/0.10.9/sha2/) for producer byte hashes and the
already-resolved [png](https://docs.rs/png/0.18.1/png/struct.Decoder.html), CRC and flate2 dependencies
for strict integrity and RGBA decoding. Wpaperd's existing EGL contexts, layer surfaces, frame
callbacks, IPC socket and CLI remain the native owners.


## Isolated runtime QA

With the locked debug binaries built, the retained harness runs on two private 800x600 virtual
outputs. It needs KWin's virtual backend, `dbus-run-session`, and system Python with dbus, Pillow
and PySide6.QtGui. Its private HOME/XDG directories and session bus exclude the host desktop.
The screenshot permission development switch applies only to the new private compositor.

```bash
/usr/bin/python3 tests/nested_wayland_scene.py --binaries target/debug
```

To exercise a published producer bundle for this exact two-output layout, pass
`--producer-package /absolute/path/to/bundle`. The harness copies all producer files byte-for-byte,
adds only its private `outputs.json`, and compares both panels of all three resting arrangements
against the published static exports. It also checks forced still restoration for file and directory
configurations, including the current image/link, status and ordinary Previous behavior.
Existing still detour bookkeeping is compared against a no-scene control; its separate maintenance
belongs to [wpaperd #2](https://github.com/Violet-Void/wpaperd/issues/2).

`--scenario transition-reload` runs a separate still-wallpaper check on four private outputs. It
starts a 12 s transition on every output with one `wpaperctl next`, rewrites the
`[default.transition.*]` table in place while those transitions run, and requires each output to
show its exact new solid colour in a capture that finishes within 11 s (only an applied reload
ends the transition early; the harness polls rather than assuming hotwatch's ~2 s delay) and again
after it settles. Before the fix for a transition program swap that dropped the
mode uniforms, every output settled on a flat 25% frame while `get-status` reported `running`.

```bash
/usr/bin/python3 tests/nested_wayland_scene.py --binaries target/debug --scenario transition-reload
```

Three pool scenarios check [wpaperd #2](https://github.com/Violet-Void/wpaperd/issues/2) on four
private outputs. Each one changes the configured pool by saving the config in place, then reads
`get`, `status --json` and per-output captures. Every image has a unique solid colour.

- `pause-reload` pauses two outputs and leaves two running. After an A to B pool change, every
  output shows a B image. Paused outputs keep `paused` and their held whole-second `duration_left`.
  They do not advance for longer than their period, while running outputs do. `next` and
  `previous` stay in B and do not lift the pause.
- `set-reload` follows dotfiles `wallpaper-cache-sync` activation:
  1. Pause every output.
  2. Set a unique image on each of three outputs.
  3. Reload into pool C. An unset paused control output adopts C, which signals that the
     reload was applied.
  4. Check that the forced images survive an idle load pass, the reload and later passes.
  5. Check that `next`, `next`, `previous` each change the image within C. A further
     `previous` stays at the start of the new history.
  6. Set final images, and check that they hold.
  7. Resume, and check that status is `running` again.
- `group-set-reload` puts three outputs in group 1. A `set` on a grouped output holds only until
  any output decodes an image. That load pass returns the group's shared image. The behavior
  predates #2: it also occurs on `2cde3d9`, and upstream `483225a` has the same code. The
  scenario pins it as parity. It then checks that the reload and each navigation step move the
  group, as one image, within the new pool. Distinct per-output sets inside one group depend on
  timing, so they are not asserted.

```bash
/usr/bin/python3 tests/nested_wayland_scene.py --binaries target/release --scenario pause-reload
```

On 2026-10-16, release builds ran under KWin 6.8.0 with software EGL. On `main` `c397225`, all
three scenarios passed repeatedly. On the pre-#13 baseline `2cde3d9`, each failed at pool
adoption. Paused outputs stayed on pool A, and the set and group controls never left pool B.
Grouped set parity matched on both revisions.

The 2026-10-13 software-EGL run on KWin 6.7.5 passed: both initial master crops had zero channel
error against Pillow, translucent solid/asset backgrounds had maximum channel error one, and
static/frozen idle requested zero frames. Orbit requested 218 callbacks across both outputs;
20 rapid Shuffle commands requested 222. Previous, freeze/thaw, reduced motion, rejected hash and
transform preservation, missing-output status, unmapped still preservation and explicit stop
restoration passed. Fresh daemon logs contained no errors or warnings. The private compositor
reported its existing Vulkan conformance warning. The harness prints its evidence directory
and retains captured PNGs, command receipts, logs and a result JSON there.

A subsequent run used the unchanged published producer bundle from
[`c17f4e8`](https://github.com/Violet-Void/violet-void-wallpapers/commit/c17f4e8f5a7c17cca5cfa811d398a9f197afff0a).
The exact scene SHA256 was `f6685ea9631a2de0e206caedb644e3456d0f8aa998fcd9ee151a904cf35dc819`;
the arrangement SHA256 was `e989a92f9ec7171b3b9abd7278206f8626faa6f9adcfeafdf33f83a48e08f58f`.
Both panels of all three rests matched the bound static exports within one channel value. Orbit
requested 218 callbacks and the 20-command burst 224; static/frozen idle requested zero frames.
Forced file/directory still restoration and the no-scene Previous controls passed, with no fresh
daemon errors or warnings. A one-second automatic still timer stayed held beyond its deadline in
scene mode, then resumed and advanced once after mode-off. Reloading duration through unset, ten,
and twenty seconds passed without a crash. Shortening an elapsed timer to zero during scene mode
preserved the forced still image, symlink, paused status and subsequent Previous detour. All 52
locked Rust tests passed; shader and texture fault tests cover cleanup in the bound context.
The extended private run also passed a removed-working-directory CLI error, deferred watcher path
selection after Stop, partially mapped grouped sorting (16 callbacks across both outputs over
1.2 seconds), and fully mapped grouped-scene idle with zero callbacks. Fresh daemon logs contained
no errors or warnings. Unsafe mid-flight retargets may reject
under the published constraints;
the harness checks retained target/history when that happens rather than requiring every burst
command to succeed.

These checks cover fixed normal-transform virtual outputs. Physical GPU fidelity, fractional scale,
portrait transforms, hotplug/DPMS, post-activation GPU faults, host performance, services and bar
reload remain unverified. Desktop integration and its live acceptance belong to
[dotfiles #1189](https://github.com/Violet-Void/dotfiles/issues/1189):

- [ ] Select a reviewed combined producer/consumer revision and prepare desktop integration.
- [ ] After separate authorization, verify keeper mode switches, hotplug, restart, resource use and
  rollback with the owning desktop workflow; check i3/X11 static parity separately.

The existing daemon manifest declares Rust 1.72, while its already-pinned `image 0.25.10` requires
Rust 1.88. Clippy succeeds but reports compatibility warnings against that stale declaration, plus
existing style warnings. This prototype was validated with the installed toolchain, not Rust 1.72;
build-baseline and packaging reconciliation remain with
[wpaperd #4](https://github.com/Violet-Void/wpaperd/issues/4).
