#!/usr/bin/python3
"""Exercise scene IPC/rendering on two private KWin virtual outputs.

Run with /usr/bin/python3 tests/nested_wayland_scene.py --binaries target/debug.
--scenario transition-reload instead checks still transitions reloaded on four outputs.
--scenario pause-reload, set-reload or group-set-reload changes the still pool on four
outputs under explicit pause, per-output set and grouped sorting (wpaperd #2).
Needs kwin_wayland, dbus-run-session, python-dbus, Pillow and PySide6.QtGui.
Uses synthetic assets, private HOME/XDG directories and a private session bus.
Does not exercise physical hotplug, host services or desktop rollout.
"""

import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import select
import shutil
import signal
import subprocess
import tempfile
import time


NAMES = ["cyan", "violet", "magenta", "ice"]
CENTERS = [(200, 180), (220, 180), (1000, 300), (1020, 300)]
COLORS = [(0, 223, 255), (140, 85, 255), (255, 56, 190), (190, 238, 255)]
DELTAS = [[0, 0, 0, 0], [96, 106, -96, -80], [-72, -60, 70, 80]]


def write_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=True, indent=2, allow_nan=False) + "\n")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def copy_producer_package(source, root):
    """Copy a published 1600x600 bundle without changing any producer bytes."""
    folder = root / "producer-package"
    shutil.copytree(source, folder)
    for path in source.rglob("*"):
        if path.is_file():
            assert digest(path) == digest(folder / path.relative_to(source))
    scene = json.loads((folder / "scene.json").read_text())
    assert scene["canvas"] == [1600, 600]
    monitors = scene["plan"]["monitors"]
    assert len(monitors) == 2
    assert [m["crop_box"] for m in monitors] == [[0, 0, 800, 600], [800, 0, 1600, 600]]
    assert all(m["pixels"] == [800, 600] and m.get("transform", "normal") == "normal"
               for m in monitors)
    assert not (folder / "outputs.json").exists()
    write_json(folder / "outputs.json", {
        "version": 1, "role": "wpaperd-motion-outputs", "scene_sha256": digest(folder / "scene.json"),
        "layout_fingerprint": scene["layout_fingerprint"],
        "outputs": [{"monitor_id": m["id"], "output": "Virtual-" + str(i),
                     "pixels": [800, 600], "transform": "normal"}
                    for i, m in enumerate(monitors)]})
    return folder


def package(root, repo, alpha=255, asset_background=False):
    from PIL import Image, ImageDraw
    folder = root / ("package-" + str(alpha) + ("-asset" if asset_background else ""))
    folder.mkdir()
    cases = json.loads((repo / "daemon/src/scene/fixtures/layouts.json").read_text())
    case = next(c for c in cases["cases"] if c["plan"]["name"] == "nested-scene-test")
    assets, objects = {}, []
    for i, name in enumerate(NAMES):
        image = Image.new("RGBA", (64, 64))
        ImageDraw.Draw(image).ellipse((12, 12, 52, 52), fill=(*COLORS[i], 140))
        path = folder / (name + ".png")
        image.save(path)
        assets[name] = {"file": path.name, "sha256": digest(path), "size": [64, 64]}
        objects.append({"id": name, "asset": name, "material": "luminous-translucent-circle",
                        "color": [*COLORS[i], 255], "alpha": 140 / 255,
                        "glow": {"radius": 0.0, "alpha": 0.0}, "depth": i,
                        "size": [64, 64], "center": list(CENTERS[i])})
    background = {"solid": [12, 16, 24, alpha]}
    if asset_background:
        path = folder / "background.png"
        Image.new("RGBA", (1600, 600), (12, 16, 24, alpha)).save(path)
        assets["background"] = {"file": path.name, "sha256": digest(path), "size": [1600, 600]}
        background = {"asset": "background"}
    scene = {"version": 1, "role": "motion-scene-sidecar", "scene_id": "nested-test",
             "origin": "manual-independent-rgba",
             "compositing": "straight-alpha-over-srgb8-depth-ascending",
             "canvas": [1600, 600], "plan": case["plan"],
             "layout_fingerprint": case["fingerprint"], "background": background,
             "objects": objects, "assets": assets, "previews": {}}
    write_json(folder / "scene.json", scene)
    scene_hash = digest(folder / "scene.json")
    arrangements = {"version": 1, "role": "motion-arrangements",
                    "scene": {"scene_id": "nested-test", "sha256": scene_hash,
                              "layout_fingerprint": case["fingerprint"]},
                    "duration_ms": 1800, "easing": "cubic-in-out",
                    "orbit": {"kind": "perpendicular-sine", "amplitude_pixels": 36.0},
                    "circle_radii": {name: 21.0 for name in NAMES},
                    "constraints": {"max_overlap_fraction": 0.65,
                                    "min_negative_space_fraction": 0.45,
                                    "max_travel_fraction": 0.25},
                    "arrangements": [{"id": "rest-" + str(i),
                                      "centers": {name: [CENTERS[j][0] + delta[j], CENTERS[j][1]]
                                                  for j, name in enumerate(NAMES)}}
                                     for i, delta in enumerate(DELTAS)]}
    write_json(folder / "arrangements.json", arrangements)
    write_json(folder / "outputs.json", {
        "version": 1, "role": "wpaperd-motion-outputs", "scene_sha256": scene_hash,
        "layout_fingerprint": case["fingerprint"],
        "outputs": [{"monitor_id": name, "output": "Virtual-" + str(i),
                     "pixels": [800, 600], "transform": "normal"}
                    for i, name in enumerate(["left", "right"])]})
    return folder


def reference(folder, arrangement=0):
    from PIL import Image
    scene = json.loads((folder / "scene.json").read_text())
    centers = json.loads((folder / "arrangements.json").read_text())["arrangements"][arrangement]["centers"]
    bg = scene["background"]
    image = (Image.new("RGBA", (1600, 600), tuple(bg["solid"])) if "solid" in bg
             else Image.open(folder / scene["assets"][bg["asset"]]["file"]).convert("RGBA"))
    for obj in sorted(scene["objects"], key=lambda o: o["depth"]):
        sprite = Image.open(folder / scene["assets"][obj["asset"]]["file"]).convert("RGBA")
        center = centers[obj["id"]]
        image.alpha_composite(sprite, (round(center[0] - obj["size"][0] / 2),
                                      round(center[1] - obj["size"][1] / 2)))
    matte = Image.new("RGBA", image.size, (0, 0, 0, 255))
    matte.alpha_composite(image)
    return matte.convert("RGB")


def compare(actual, expected):
    from PIL import ImageChops
    error = max(high for low, high in ImageChops.difference(actual, expected).getextrema())
    assert error <= 2, "GPU/reference maximum channel error: " + str(error)
    return error


def stop_owned(process):
    if process is None or process.poll() is not None:
        return
    os.killpg(process.pid, signal.SIGTERM)
    try:
        process.wait(timeout=8)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=3)


def start_kwin(log, output_count):
    """Start private KWin with 800x600 virtual outputs Virtual-0..Virtual-N."""
    env = dict(os.environ, KWIN_SCREENSHOT_NO_PERMISSION_CHECKS="1")
    compositor = subprocess.Popen([
        "kwin_wayland", "--virtual", "--socket", "wpaperd-scene-test", "--width", "800",
        "--height", "600", "--output-count", str(output_count), "--no-lockscreen", "--no-kactivities",
        "--no-global-shortcuts"], env=env, stdout=log, stderr=subprocess.STDOUT,
        start_new_session=True)
    return compositor


def kwin_capture(root, compositor):
    """Wait for the private KWin capture service and return a per-output RGB capture."""
    import dbus
    from PIL import Image
    from PySide6.QtGui import QImage
    bus = dbus.SessionBus()
    deadline = time.monotonic() + 12
    while not bus.name_has_owner("org.kde.KWin.ScreenShot2"):
        assert compositor.poll() is None, "Private KWin exited"
        assert time.monotonic() < deadline, "Private KWin capture service unavailable"
        time.sleep(.1)
    capture = dbus.Interface(bus.get_object("org.kde.KWin.ScreenShot2", "/org/kde/KWin/ScreenShot2"),
                             "org.kde.KWin.ScreenShot2")

    def shot(label, output=0):
        read_fd, write_fd = os.pipe()
        try:
            meta = capture.CaptureScreen("Virtual-" + str(output),
                                         dbus.Dictionary({}, signature="sv"),
                                         dbus.types.UnixFd(write_fd), timeout=10)
            os.close(write_fd)
            write_fd = None
            expected = int(meta["stride"]) * int(meta["height"])
            assert 0 < expected <= 8_000_000
            data = bytearray()
            deadline = time.monotonic() + 10
            while len(data) < expected:
                remaining = deadline - time.monotonic()
                assert remaining > 0 and select.select([read_fd], [], [], remaining)[0], "Capture timeout"
                chunk = os.read(read_fd, min(65536, expected - len(data)))
                assert chunk, "Capture truncated"
                data.extend(chunk)
            image = QImage(bytes(data), int(meta["width"]), int(meta["height"]),
                           int(meta["stride"]), QImage.Format(int(meta["format"])))
            path = root / (label + ".png")
            assert image.save(str(path)), "Capture save failed"
            return Image.open(path).convert("RGB")
        finally:
            os.close(read_fd)
            if write_fd is not None:
                os.close(write_fd)

    return shot


def inside(args):
    from PIL import Image
    root = args.inside
    def interrupted(signum, frame):
        raise SystemExit("Private test interrupted")
    signal.signal(signal.SIGTERM, interrupted)
    logs = [open(root / name, "wb") for name in ("compositor.log", "daemon.log")]
    compositor = daemon = None
    result = {"passed": False, "evidence": str(root)}
    commands = open(root / "commands.log", "w")
    try:
        compositor = start_kwin(logs[0], 2)
        shot = kwin_capture(root, compositor)

        still = root / "still.png"
        Image.new("RGBA", (800, 600), (41, 49, 63, 255)).save(still)
        config = root / "config.toml"
        config.write_text('[default]\npath=' + json.dumps(str(still)) + '\nmode="stretch"\ntransition-time=0\n')
        daemon = subprocess.Popen([str(args.binaries / "wpaperd"), "--config", str(config), "--verbose"],
                                  env=dict(os.environ, WAYLAND_DEBUG="1"), stdout=logs[1],
                                  stderr=subprocess.STDOUT, start_new_session=True)
        deadline = time.monotonic() + 15
        socket = Path(os.environ["XDG_RUNTIME_DIR"]) / "wpaperd.sock"
        while not socket.exists():
            assert daemon.poll() is None, "Private daemon exited before IPC readiness"
            assert time.monotonic() < deadline, "Daemon IPC timeout"
            time.sleep(.1)

        def cli(*values, success=True):
            proc = subprocess.run([str(args.binaries / "wpaperctl"), *map(str, values)],
                                  capture_output=True, text=True, timeout=10)
            commands.write(json.dumps({"args": list(map(str, values)), "exit": proc.returncode,
                                       "stdout": proc.stdout, "stderr": proc.stderr}) + "\n")
            commands.flush()
            if success is None and proc.returncode != 0:
                assert proc.returncode == 1 and proc.stderr.strip() in {
                    "Circle overlap exceeds bound", "Circle rim outside canvas",
                    "Sampled travel exceeds bound"}, proc.stderr + proc.stdout
                return None
            assert (proc.returncode == 0) == (True if success is None else success), proc.stderr + proc.stdout
            return proc.stdout

        def frames():
            return (root / "daemon.log").read_text(errors="replace").count(".frame(")

        vanished = root / "vanished-cwd"
        vanished.mkdir()
        script = "import os,sys;os.chdir(sys.argv[1]);os.rmdir(sys.argv[1]);os.execv(sys.argv[2],[sys.argv[2],'scene','start','.'])"
        proc = subprocess.run(["/usr/bin/python3", "-c", script, str(vanished), str(args.binaries / "wpaperctl")],
                              capture_output=True, text=True, timeout=10)
        assert proc.returncode == 1 and "Cannot resolve relative scene path" in proc.stderr
        assert "panicked" not in proc.stderr
        result["removed_cwd_cli_error"] = True
        time.sleep(.5)
        baseline = shot("still-before")
        assert baseline.getpixel((200, 180)) == (41, 49, 63)
        folder = (copy_producer_package(args.producer_package, root) if args.producer_package
                  else package(root, args.repo))
        cli("scene", "start", folder)
        time.sleep(.3)
        initial = shot("initial-left")
        expected = reference(folder)
        result["initial_error"] = compare(initial, expected.crop((0, 0, 800, 600)))
        result["right_crop_error"] = compare(shot("initial-right", 1), expected.crop((800, 0, 1600, 600)))
        assert initial.getpixel((200, 180)) != (12, 16, 24)
        before = frames()
        orbit_started = time.monotonic()
        cli("scene", "shuffle")
        moving = []
        for i in range(3):
            time.sleep(.35)
            moving.append(shot("orbit-" + str(i)))
        time.sleep(1)
        settled = shot("settled")
        assert moving[0].tobytes() != moving[1].tobytes() != moving[2].tobytes()
        assert settled.tobytes() != initial.tobytes()
        result["orbit_frame_requests"] = frames() - before
        assert result["orbit_frame_requests"] >= 10
        assert result["orbit_frame_requests"] <= 240 * (time.monotonic() - orbit_started) + 10
        before = frames()
        ticks = int(Path('/proc/' + str(daemon.pid) + '/stat').read_text().split()[13])
        time.sleep(.7)
        result["idle_frame_requests"] = frames() - before
        result["idle_user_cpu_seconds"] = (int(Path('/proc/' + str(daemon.pid) + '/stat').read_text().split()[13]) - ticks) / os.sysconf("SC_CLK_TCK")
        assert result["idle_frame_requests"] == 0
        cli("scene", "previous")
        time.sleep(2)
        compare(shot("previous"), initial)
        cli("scene", "shuffle")
        time.sleep(.35)
        cli("scene", "freeze")
        time.sleep(.1)
        frozen = shot("frozen")
        before = frames()
        time.sleep(.7)
        assert shot("frozen-later").tobytes() == frozen.tobytes()
        result["frozen_frame_requests"] = frames() - before
        assert result["frozen_frame_requests"] == 0
        cli("scene", "freeze")
        time.sleep(2)
        before = frames()
        burst_started = time.monotonic()
        for _ in range(20):
            prior = json.loads(cli("scene", "status", "--json")) if args.producer_package else None
            accepted = cli("scene", "shuffle", success=None if args.producer_package else True)
            if accepted is None:
                after = json.loads(cli("scene", "status", "--json"))
                for key in ("arrangement", "history_depth", "scene_sha256", "outputs", "error"):
                    assert after[key] == prior[key], "Rejected retarget changed " + key
                result["producer_retarget_rejections"] = result.get("producer_retarget_rejections", 0) + 1
        time.sleep(2)
        result["burst_frame_requests"] = frames() - before
        assert result["burst_frame_requests"] <= 240 * (time.monotonic() - burst_started) + 10
        status = json.loads(cli("scene", "status", "--json"))
        assert status["state"] == "static" and status["history_depth"] <= 32
        assert status["outputs"] == {"Virtual-0": "mapped", "Virtual-1": "mapped"}
        result["status"] = status
        valid = shot("valid-before-rejection")
        broken = root / "broken"
        shutil.copytree(folder, broken)
        scene = json.loads((broken / "scene.json").read_text())
        scene["assets"][scene["objects"][0]["asset"]]["sha256"] = "0" * 64
        write_json(broken / "scene.json", scene)
        cli("scene", "start", broken, success=False)
        time.sleep(.2)
        assert shot("valid-after-rejection").tobytes() == valid.tobytes()
        mismatch = root / "mismatch"
        shutil.copytree(folder, mismatch)
        mapping = json.loads((mismatch / "outputs.json").read_text())
        mapping["outputs"][0]["transform"] = "180"
        write_json(mismatch / "outputs.json", mapping)
        cli("scene", "start", mismatch, success=False)
        time.sleep(.2)
        assert shot("valid-after-mismatch").tobytes() == valid.tobytes()
        cli("scene", "stop")
        time.sleep(.4)
        assert shot("still-after").tobytes() == baseline.tobytes()
        cli("scene", "start", folder, "--reduced-motion")
        time.sleep(.3)
        cli("scene", "shuffle")
        time.sleep(.2)
        before = frames()
        time.sleep(.6)
        assert frames() == before, "Reduced-motion idle requested frames"
        compare(shot("reduced-motion"), reference(folder, 1).crop((0, 0, 800, 600)))
        cli("scene", "stop")
        for asset_background in (False, True):
            translucent = package(root, args.repo, 128, asset_background)
            cli("scene", "start", translucent, "--reduced-motion")
            time.sleep(.3)
            result["translucent_" + ("asset" if asset_background else "solid") + "_error"] = compare(
                shot("translucent-" + str(asset_background)), reference(translucent).crop((0, 0, 800, 600)))
            cli("scene", "stop")
        missing = root / "missing"
        shutil.copytree(folder, missing)
        mapping = json.loads((missing / "outputs.json").read_text())
        mapping["outputs"][1]["output"] = "Absent-output"
        write_json(missing / "outputs.json", mapping)
        cli("scene", "start", missing, "--reduced-motion")
        time.sleep(.3)
        assert shot("unmapped-still", 1).tobytes() == baseline.tobytes()
        status = json.loads(cli("scene", "status", "--json"))
        assert status["outputs"] == {"Virtual-0": "mapped", "Absent-output": "missing"}
        cli("scene", "stop")
        if args.producer_package:
            exports = json.loads((folder / "exports.json").read_text())
            cli("scene", "start", folder, "--reduced-motion")
            errors = []
            for index, variant in enumerate(exports["variants"]):
                if index:
                    cli("scene", "shuffle")
                time.sleep(.2)
                for output, monitor in enumerate(json.loads((folder / "scene.json").read_text())["plan"]["monitors"]):
                    descriptor = variant["panels"][monitor["id"]]
                    expected_panel = Image.open(folder / descriptor["file"]).convert("RGBA")
                    matte = Image.new("RGBA", expected_panel.size, (0, 0, 0, 255))
                    matte.alpha_composite(expected_panel)
                    errors.append(compare(shot("producer-" + variant["id"] + "-" + str(output), output),
                                          matte.convert("RGB")))
            result["producer_static_panel_errors"] = errors
            result["producer_scene_sha256"] = digest(folder / "scene.json")
            result["producer_arrangements_sha256"] = digest(folder / "arrangements.json")
            cli("scene", "stop")
        # Verify restoration against real still selection, state, symlinks and detour navigation.
        forced = root / "forced.png"
        Image.new("RGBA", (800, 600), (96, 32, 72, 255)).save(forced)
        alternate = root / "alternate.png"
        Image.new("RGBA", (800, 600), (21, 131, 64, 255)).save(alternate)
        pool = root / "pool"
        pool.mkdir()
        for i, color in enumerate(((41, 49, 63), (64, 88, 112), (123, 91, 42))):
            Image.new("RGBA", (800, 600), (*color, 255)).save(pool / (str(i) + ".png"))
        def save_config(text):
            with config.open("r+") as stream:
                stream.write(text.ljust(4096))
                stream.flush()

        def save_directory_config(duration):
            text = ('[default]\npath=' + json.dumps(str(pool)) +
                    '\nmode="stretch"\ntransition-time=0\nsorting="ascending"\n')
            if duration is not None:
                text += 'duration=' + json.dumps(duration) + '\n'
            # Preserve the watched inode and avoid a transient truncated configuration.
            save_config(text)

        def reload_duration(duration):
            save_directory_config(duration)
            deadline = time.monotonic() + 5
            while True:
                status = json.loads(cli("status", "--json", "Virtual-0"))[0]
                if status["duration_left"] == duration:
                    return
                assert daemon.poll() is None and time.monotonic() < deadline, status
                time.sleep(.05)

        for directory in (False, True):
            if directory:
                stop_owned(daemon)
                # Only this exited test process owned the socket in the private runtime.
                socket.unlink(missing_ok=True)
                save_directory_config("1s")
                daemon = subprocess.Popen([str(args.binaries / "wpaperd"), "--config", str(config), "--verbose"],
                                          env=dict(os.environ, WAYLAND_DEBUG="1"), stdout=logs[1],
                                          stderr=subprocess.STDOUT, start_new_session=True)
                deadline = time.monotonic() + 15
                while not socket.exists():
                    assert daemon.poll() is None and time.monotonic() < deadline
                    time.sleep(.1)
                time.sleep(.5)
                timer_original = cli("get", "Virtual-0").strip()
                cli("scene", "start", folder, "--reduced-motion")
                time.sleep(2.2)
                assert cli("get", "Virtual-0").strip() == timer_original
                reload_duration(None)
                reload_duration("10s")
                reload_duration("20s")
                reload_duration("1s")
                result["scene_duration_reload_unset_and_repeated"] = True
                cli("scene", "stop")
                time.sleep(.2)
                assert cli("get", "Virtual-0").strip() == timer_original
                time.sleep(1.1)
                timer_next = cli("get", "Virtual-0").strip()
                assert timer_next == str(pool / (str((int(Path(timer_original).stem) + 1) % 3) + ".png"))
                result["automatic_timer_hold_and_resume"] = True
                save_directory_config("10s")
                time.sleep(.3)
                cli("next", "Virtual-0")
                time.sleep(2.1)
                cli("pause", "Virtual-0")
                time.sleep(.3)
            # Keep independent still-output cycling out of the detour control comparison.
            cli("pause")
            cli("set", forced, "Virtual-0")
            time.sleep(.4)
            selected = shot("detour-selected-" + str(directory))
            assert selected.getpixel((200, 180)) == (96, 32, 72)
            # Record ordinary detour behavior before testing the scene boundary. The still
            # backend has separate bookkeeping quirks; scene mode must preserve its behavior.
            cli("previous", "Virtual-0")
            time.sleep(.4)
            previous_path = cli("get", "Virtual-0").strip()
            previous_pixels = shot("detour-control-previous-" + str(directory))
            previous_state = cli("status", "--json", "Virtual-0")
            cli("set", alternate, "Virtual-0")
            time.sleep(.4)
            cli("set", forced, "Virtual-0")
            time.sleep(.4)
            assert shot("detour-reselected-" + str(directory)).tobytes() == selected.tobytes()
            state = cli("status", "--json", "Virtual-0")
            links = list((root / "home").rglob("Virtual-0"))
            assert len(links) == 1 and links[0].is_symlink()
            assert links[0].resolve() == forced
            cli("scene", "start", folder, "--reduced-motion")
            time.sleep(.2)
            if directory:
                save_directory_config("1s")
                deadline = time.monotonic() + 5
                while json.loads(cli("status", "--json", "Virtual-0"))[0]["duration_left"] != "0s":
                    assert daemon.poll() is None and time.monotonic() < deadline
                    time.sleep(.05)
                state_item = json.loads(state)
                state_item[0]["duration_left"] = "0s"
                state = json.dumps(state_item)
                previous_item = json.loads(previous_state)
                previous_item[0]["duration_left"] = "0s"
                previous_state = json.dumps(previous_item)
                result["scene_exhausted_reload_preserves_forced_detour"] = True
            cli("scene", "stop")
            time.sleep(.3)
            assert shot("detour-restored-" + str(directory)).tobytes() == selected.tobytes()
            assert cli("get", "Virtual-0").strip() == str(forced)
            assert json.loads(cli("status", "--json", "Virtual-0")) == json.loads(state)
            assert links[0].resolve() == forced
            cli("previous", "Virtual-0")
            time.sleep(.4)
            assert cli("get", "Virtual-0").strip() == previous_path
            assert shot("detour-previous-" + str(directory)).tobytes() == previous_pixels.tobytes()
            assert json.loads(cli("status", "--json", "Virtual-0")) == json.loads(previous_state)
            result["forced_" + ("directory" if directory else "file") + "_restoration"] = True
        # A watcher path update is deferred until explicit Stop, without losing the selection.
        cli("scene", "start", folder, "--reduced-motion")
        time.sleep(.2)
        scene_pixels = shot("before-config-selection")
        save_config('[default]\npath=' + json.dumps(str(alternate)) + '\nmode="stretch"\ntransition-time=0\n')
        deadline = time.monotonic() + 5
        while json.loads(cli("status", "--json", "Virtual-0"))[0]["duration_left"] is not None:
            assert daemon.poll() is None and time.monotonic() < deadline
            time.sleep(.05)
        assert shot("held-config-selection").tobytes() == scene_pixels.tobytes()
        cli("scene", "stop")
        deadline = time.monotonic() + 5
        while cli("get", "Virtual-0").strip() != str(alternate):
            assert daemon.poll() is None and time.monotonic() < deadline
            time.sleep(.05)
        time.sleep(.3)
        assert shot("restored-config-selection").getpixel((200, 180)) == (21, 131, 64)
        cli("scene", "stop")
        time.sleep(.2)
        assert cli("get", "Virtual-0").strip() == str(alternate)
        result["deferred_config_selection_after_stop"] = True

        # Restart only the owned private daemon to establish ordinary grouped sorting.
        stop_owned(daemon)
        socket.unlink(missing_ok=True)
        save_config('[default]\npath=' + json.dumps(str(pool)) +
                    '\nmode="stretch"\ntransition-time=0\ngroup=1\nduration="300ms"\n')
        daemon = subprocess.Popen([str(args.binaries / "wpaperd"), "--config", str(config), "--verbose"],
                                  env=dict(os.environ, WAYLAND_DEBUG="1"), stdout=logs[1],
                                  stderr=subprocess.STDOUT, start_new_session=True)
        deadline = time.monotonic() + 15
        while not socket.exists():
            assert daemon.poll() is None and time.monotonic() < deadline
            time.sleep(.1)
        time.sleep(.5)
        cli("scene", "start", missing, "--reduced-motion")
        time.sleep(.2)
        held = shot("group-mapped-rest")
        before = frames()
        time.sleep(1.2)
        assert shot("group-mapped-stable").tobytes() == held.tobytes()
        result["group_partial_mapping_callbacks"] = frames() - before
        assert result["group_partial_mapping_callbacks"] < 80
        cli("scene", "start", folder)
        cli("scene", "shuffle")
        time.sleep(2.2)
        before = frames()
        time.sleep(.8)
        result["group_scene_idle_callbacks"] = frames() - before
        assert result["group_scene_idle_callbacks"] == 0
        cli("scene", "stop")
        time.sleep(.6)
        assert str(pool) in cli("get", "Virtual-0")
        result["grouped_callbacks_and_restore"] = True
        assert daemon.poll() is None, "Daemon exited during scenarios"
        text = (root / "daemon.log").read_text(errors="replace")
        marker = re.search(r"\b(?:ERROR|WARN|WARNING|panicked)\b|protocol\s+error|failed\s+to\s+(?:draw|load\s+scene)",
                           text, re.IGNORECASE)
        assert marker is None, "Fresh daemon log error: " + marker.group(0)
        result["passed"] = True
    finally:
        stop_owned(daemon)
        stop_owned(compositor)
        for log in logs:
            log.close()
        commands.close()
        write_json(root / "result.json", result)
    print(json.dumps(result, indent=2))


RELOAD_OUTPUTS = 4
RELOAD_COLORS = [(230, 120, 40), (40, 200, 120), (60, 110, 240)]
RELOAD_TRANSITIONS = ["fade", "directional", "circle-open", "circle"]


def transition_reload(args):
    """Rewrite the transition table while every output runs a simultaneous transition.

    This mirrors an exec hook that edits config.toml on wallpaper changes. Every output must
    settle on its exact new solid colour instead of a flat dark frame.
    """
    from PIL import Image
    root = args.inside
    def interrupted(signum, frame):
        raise SystemExit("Private test interrupted")
    signal.signal(signal.SIGTERM, interrupted)
    logs = [open(root / name, "wb") for name in ("compositor.log", "daemon.log")]
    compositor = daemon = None
    result = {"scenario": "transition-reload", "passed": False, "evidence": str(root)}
    commands = open(root / "commands.log", "w")
    try:
        compositor = start_kwin(logs[0], RELOAD_OUTPUTS)
        shot = kwin_capture(root, compositor)
        pool = root / "pool"
        pool.mkdir()
        for i, color in enumerate(RELOAD_COLORS):
            Image.new("RGBA", (800, 600), (*color, 255)).save(pool / (str(i) + ".png"))
        config = root / "config.toml"

        def config_text(transition):
            return ('[default]\npath=' + json.dumps(str(pool)) + '\nmode="stretch"\n'
                    'sorting="ascending"\ntransition-time=12000\ninitial-transition=false\n'
                    '[default.transition.' + transition + ']\n').ljust(4096)

        config.write_text(config_text(RELOAD_TRANSITIONS[0]))
        daemon = subprocess.Popen([str(args.binaries / "wpaperd"), "--config", str(config), "--verbose"],
                                  stdout=logs[1], stderr=subprocess.STDOUT, start_new_session=True)
        deadline = time.monotonic() + 15
        socket = Path(os.environ["XDG_RUNTIME_DIR"]) / "wpaperd.sock"
        while not socket.exists():
            assert daemon.poll() is None, "Private daemon exited before IPC readiness"
            assert time.monotonic() < deadline, "Daemon IPC timeout"
            time.sleep(.1)

        def cli(*values):
            proc = subprocess.run([str(args.binaries / "wpaperctl"), *map(str, values)],
                                  capture_output=True, text=True, timeout=10)
            commands.write(json.dumps({"args": list(map(str, values)), "exit": proc.returncode,
                                       "stdout": proc.stdout, "stderr": proc.stderr}) + "\n")
            commands.flush()
            assert proc.returncode == 0, proc.stderr + proc.stdout
            return proc.stdout

        def settled(label, color):
            observed = {}
            for output in range(RELOAD_OUTPUTS):
                image = shot(label + "-" + str(output), output)
                colors = sorted(c for _, c in image.getcolors(1 << 20))
                observed["Virtual-" + str(output)] = colors[:4]
            result[label] = observed
            return all(c == [color] for c in observed.values())

        time.sleep(1)
        assert settled("initial", RELOAD_COLORS[0]), result["initial"]
        result["rounds"] = []
        for step in range(1, 4):
            # One IPC call starts a transition on every output at the same moment.
            cli("next")
            started = time.monotonic()
            time.sleep(.5)
            label = "round-" + str(step)
            color = RELOAD_COLORS[step % len(RELOAD_COLORS)]
            # The new colour must not be complete yet: the reload has to land in flight.
            in_flight = not settled(label + "-before-reload", color)
            # Rewrite the table in place; hotwatch delivers it about 2 s later, mid-transition.
            with config.open("r+") as stream:
                stream.write(config_text(RELOAD_TRANSITIONS[step]))
                stream.flush()
            # Applying the reload ends the transitions early. Before 11 s an unapplied 12 s
            # transition is still mid-way, so an exact frame proves the swap happened. Poll
            # instead of assuming the hotwatch delivery delay.
            deadline = started + 11
            while True:
                applied = settled(label + "-applied", color)
                finished = time.monotonic()
                # A match whose capture ended after the deadline may be natural completion.
                applied = applied and finished <= deadline
                if applied or finished > deadline:
                    break
                time.sleep(.25)
            result.setdefault("applied_after_seconds", []).append(round(finished - started, 2))
            time.sleep(.5)
            drawn = settled(label, color)
            result["rounds"].append({"round": step, "transition": RELOAD_TRANSITIONS[step],
                                     "in_flight": in_flight, "applied": applied, "drawn": drawn})
        status = json.loads(cli("status", "--json"))
        result["status"] = [s.get("status") for s in status]
        assert all(r["drawn"] for r in result["rounds"]), "Output left undrawn after reload"
        assert all(r["in_flight"] for r in result["rounds"]), "Transition finished before reload"
        assert all(r["applied"] for r in result["rounds"]), "Reload not applied mid-transition"
        assert daemon.poll() is None, "Daemon exited during transition reloads"
        text = (root / "daemon.log").read_text(errors="replace")
        marker = re.search(r"\b(?:ERROR|WARN|WARNING|panicked)\b|protocol\s+error|failed\s+to\s+draw",
                           text, re.IGNORECASE)
        assert marker is None, "Fresh daemon log error: " + marker.group(0)
        result["passed"] = True
    finally:
        stop_owned(daemon)
        stop_owned(compositor)
        for log in logs:
            log.close()
        commands.close()
        write_json(root / "result.json", result)
    print(json.dumps(result, indent=2))


POOL_OUTPUTS = ["Virtual-" + str(i) for i in range(4)]
# Every image has a unique solid colour, so a capture names the file each output shows.
POOL_COLORS = {"a": [(200, 40, 40), (200, 120, 40), (200, 200, 40)],
               "b": [(40, 160, 200), (40, 80, 200), (100, 40, 200)],
               "c": [(40, 200, 80), (120, 200, 120), (20, 120, 60)],
               "probe": [(250, 250, 250), (10, 10, 10), (128, 128, 128), (250, 0, 250)],
               "final": [(90, 60, 30), (30, 60, 90), (60, 90, 30), (90, 30, 60)]}


class PrivateDaemon:
    """One owned daemon on the private KWin, with logged wpaperctl and per-output captures."""

    def __init__(self, args, root, shot, log, commands):
        self.args, self.root, self.shot, self.log, self.commands = args, root, shot, log, commands
        self.config = root / "config.toml"
        self.socket = Path(os.environ["XDG_RUNTIME_DIR"]) / "wpaperd.sock"
        self.process = None
        self.colors = {}
        self.captures = 0
        for name, colors in POOL_COLORS.items():
            folder = root / name
            folder.mkdir()
            for i, color in enumerate(colors):
                path = folder / (str(i) + ".png")
                from PIL import Image
                Image.new("RGBA", (800, 600), (*color, 255)).save(path)
                self.colors[str(path)] = color

    def pool(self, name):
        return self.root / name

    def files(self, name):
        return [str(self.pool(name) / (str(i) + ".png")) for i in range(len(POOL_COLORS[name]))]

    def save(self, text):
        # In place on the watched inode, like the dotfiles config owner; padding keeps the
        # file length fixed so the watcher never parses a truncated config.
        with self.config.open("r+" if self.config.exists() else "w") as stream:
            stream.write(text.ljust(4096))
            stream.flush()

    def start(self, text):
        self.stop()
        self.save(text)
        self.process = subprocess.Popen(
            [str(self.args.binaries / "wpaperd"), "--config", str(self.config), "--verbose"],
            stdout=self.log, stderr=subprocess.STDOUT, start_new_session=True)
        deadline = time.monotonic() + 15
        while not self.socket.exists():
            assert self.process.poll() is None, "Private daemon exited before IPC readiness"
            assert time.monotonic() < deadline, "Daemon IPC timeout"
            time.sleep(.1)
        # The socket is bound before output events are dispatched; wait for every output.
        while True:
            proc = subprocess.run([str(self.args.binaries / "wpaperctl"), "status", "--json"],
                                  capture_output=True, text=True, timeout=10)
            if proc.returncode == 0 and len(json.loads(proc.stdout)) == len(POOL_OUTPUTS):
                return
            assert self.process.poll() is None and time.monotonic() < deadline, proc.stderr
            time.sleep(.1)

    def stop(self):
        stop_owned(self.process)
        # Only the exited test daemon owned this socket in the private runtime.
        self.socket.unlink(missing_ok=True)

    def cli(self, *values):
        proc = subprocess.run([str(self.args.binaries / "wpaperctl"), *map(str, values)],
                              capture_output=True, text=True, timeout=10)
        self.commands.write(json.dumps({"args": list(map(str, values)), "exit": proc.returncode,
                                        "stdout": proc.stdout, "stderr": proc.stderr}) + "\n")
        self.commands.flush()
        assert proc.returncode == 0, proc.stderr + proc.stdout
        return proc.stdout

    def current(self):
        return {name: self.cli("get", name).strip() for name in POOL_OUTPUTS}

    def status(self):
        return {s["display"]: s for s in json.loads(self.cli("status", "--json"))}

    def drawn(self, label):
        """The file each output shows, decoded from its capture; None for a mixed frame."""
        seen = {}
        self.captures += 1
        for i, name in enumerate(POOL_OUTPUTS):
            image = self.shot(label + "-" + name, i)
            colors = image.getcolors(1 << 20)
            if len(colors) != 1:
                seen[name] = None
                continue
            color = colors[0][1]
            seen[name] = next((p for p, c in self.colors.items() if c == color), "unknown" + str(color))
        return seen

    def wait(self, label, predicate, timeout=8):
        """Poll getters and captures until predicate(current, drawn) holds."""
        deadline = time.monotonic() + timeout
        while True:
            assert self.process.poll() is None, "Daemon exited during " + label
            current = self.current()
            drawn = self.drawn(label)
            if predicate(current, drawn):
                return current, drawn
            assert time.monotonic() < deadline, label + ": current=" + json.dumps(current) + \
                " drawn=" + json.dumps(drawn)
            time.sleep(.25)

    def assert_clean_log(self):
        assert self.process.poll() is None, "Daemon exited during scenario"
        text = (self.root / "daemon.log").read_text(errors="replace")
        marker = re.search(r"\b(?:ERROR|WARN|WARNING|panicked)\b|protocol\s+error|failed\s+to\s+draw",
                           text, re.IGNORECASE)
        assert marker is None, "Fresh daemon log error: " + marker.group(0)


def pool_scenario(args, body):
    """Run body(daemon, result) on four private KWin outputs and record result.json."""
    root = args.inside
    def interrupted(signum, frame):
        raise SystemExit("Private test interrupted")
    signal.signal(signal.SIGTERM, interrupted)
    logs = [open(root / name, "wb") for name in ("compositor.log", "daemon.log")]
    compositor = daemon = None
    result = {"scenario": args.scenario, "passed": False, "evidence": str(root),
              "binaries": str(args.binaries)}
    commands = open(root / "commands.log", "w")
    try:
        compositor = start_kwin(logs[0], len(POOL_OUTPUTS))
        shot = kwin_capture(root, compositor)
        daemon = PrivateDaemon(args, root, shot, logs[1], commands)
        body(daemon, result)
        daemon.assert_clean_log()
        result["passed"] = True
    except Exception as err:
        result["failure"] = type(err).__name__ + ": " + str(err)
        raise
    finally:
        if daemon is not None:
            daemon.stop()
        stop_owned(compositor)
        for log in logs:
            log.close()
        commands.close()
        write_json(root / "result.json", result)
        print(json.dumps(result, indent=2))


def pause_reload(daemon, result):
    """Explicitly paused outputs keep their pause state across a pool change (#2).

    The defined current-image replacement still applies: a paused output whose image left the
    pool shows a new-pool image. Pause status, the held timer and navigation stay paused.
    """
    a, b = daemon.files("a"), daemon.files("b")
    paused, running = POOL_OUTPUTS[:2], POOL_OUTPUTS[2:]
    def config(pool):
        return ('[default]\npath=' + json.dumps(str(daemon.pool(pool))) + '\nmode="stretch"\n'
                'sorting="ascending"\nduration="2s"\ntransition-time=0\n'
                'initial-transition=false\n' +
                # A longer period proves both an exact held remainder and no advance while held.
                ''.join('[' + n + ']\nduration="5s"\n' for n in paused))
    daemon.start(config("a"))
    daemon.wait("initial", lambda cur, drawn: all(cur[n] in a and drawn[n] == cur[n]
                                                  for n in POOL_OUTPUTS))
    # Let part of the period elapse, so a timer reset to the full period is visible.
    time.sleep(1.5)
    daemon.cli("pause", *paused)
    time.sleep(.3)
    before = daemon.status()
    result["paused_status_before"] = {n: before[n] for n in paused}
    assert all(before[n]["status"] == "paused" for n in paused), before
    held = {n: before[n]["duration_left"] for n in paused}
    assert all(held[n] not in (None, "5s") for n in paused), held

    daemon.save(config("b"))
    current, drawn = daemon.wait(
        "pool-b-adopted", lambda cur, drawn: all(cur[n] in b and drawn[n] == cur[n]
                                                 for n in POOL_OUTPUTS))
    result["adopted"] = current
    after = daemon.status()
    result["paused_status_after"] = {n: after[n] for n in paused}
    for n in paused:
        assert after[n]["status"] == "paused", after
        # Timer state is untouched: the held remainder (whole seconds) is the pre-reload value.
        assert after[n]["duration_left"] == held[n], (n, held[n], after[n])

    # Paused outputs never advance past their period; running outputs prove timers still fire.
    adopted = current
    time.sleep(5.5)
    current, drawn = daemon.wait("held-while-paused", lambda cur, drawn: all(
        drawn[n] == cur[n] for n in POOL_OUTPUTS))
    for n in paused:
        assert current[n] == adopted[n], ("paused output advanced", n, adopted[n], current[n])
    assert all(current[n] in b for n in running), current
    assert any(current[n] != adopted[n] for n in running), ("running outputs never advanced", current)
    result["running_advanced"] = {n: [adopted[n], current[n]] for n in running}

    # Navigation stays inside the new pool and does not lift an explicit pause.
    visited = []
    for action in ("next", "previous", "previous", "previous", "next"):
        prior = daemon.current()
        daemon.cli(action, *paused)
        current, _ = daemon.wait(action + "-paused", lambda cur, drawn: all(
            cur[n] != prior[n] and drawn[n] == cur[n] for n in paused))
        visited.append({n: current[n] for n in paused})
        assert all(current[n] in b for n in paused), ("navigation left pool B", action, current)
        status = daemon.status()
        assert all(status[n]["status"] == "paused" for n in paused), (action, status)
    result["paused_navigation"] = visited
    daemon.cli("resume", *paused)
    time.sleep(.3)
    status = daemon.status()
    assert all(status[n]["status"] == "running" for n in POOL_OUTPUTS), status


def set_reload(daemon, result):
    """A coordinated per-output set survives a pool reload (#2).

    Mirrors dotfiles wallpaper-cache-sync activate_selection: pause, set a unique image per
    output, reload into a new pool, navigate into it, set final images, resume. Virtual-3 stays
    paused and unset, so its adoption of the new pool signals the applied reload while every
    other output shows a forced image.
    """
    b, c = daemon.files("b"), daemon.files("c")
    probes, finals = daemon.files("probe"), daemon.files("final")
    outputs, control = POOL_OUTPUTS[:3], POOL_OUTPUTS[3]
    def config(pool):
        return ('[default]\npath=' + json.dumps(str(daemon.pool(pool))) + '\nmode="stretch"\n'
                'duration="2s"\ntransition-time=0\ninitial-transition=false\n')
    daemon.start(config("b"))
    daemon.wait("initial", lambda cur, drawn: all(
        cur[n] in b and drawn[n] == cur[n] for n in POOL_OUTPUTS))
    daemon.cli("pause", *POOL_OUTPUTS)

    def set_each(images, label):
        expected = dict(zip(outputs, images))
        for n, image in expected.items():
            daemon.cli("set", image, n)
        daemon.wait(label, lambda cur, drawn: all(
            cur[n] == expected[n] and drawn[n] == expected[n] for n in outputs))
        # Each set decode pings every output. Hold through further passes and timer periods.
        time.sleep(2.5)
        current, drawn = daemon.wait(label + "-held", lambda cur, drawn: all(
            drawn[n] == cur[n] for n in POOL_OUTPUTS))
        for n in outputs:
            assert current[n] == expected[n], (label + " replaced", n, expected[n], current[n])
        return current

    result["coordinated_set"] = set_each(probes, "coordinated-set")
    # A decode on the control pings every output: one idle load pass before the reload.
    before = daemon.current()[control]
    daemon.cli("next", control)
    current, _ = daemon.wait("control-decode", lambda cur, drawn: cur[control] != before
                             and drawn[control] == cur[control]
                             and all(cur[n] == p for n, p in zip(outputs, probes)))
    daemon.save(config("c"))
    daemon.wait("reload-applied", lambda cur, drawn: cur[control] in c
                and drawn[control] == cur[control])
    time.sleep(2.5)
    current, drawn = daemon.wait("after-reload", lambda cur, drawn: all(
        drawn[n] == cur[n] for n in POOL_OUTPUTS))
    result["after_reload"] = current
    for n, probe in zip(outputs, probes):
        assert current[n] == probe, ("forced image replaced by reload", n, probe, current[n])
    status = daemon.status()
    assert all(status[n]["status"] == "paused" for n in POOL_OUTPUTS), status

    # Leaving the detour adopts the new pool; the reset history then starts there.
    for action in ("next", "next", "previous"):
        prior = daemon.current()
        daemon.cli(action, *outputs)
        current, _ = daemon.wait(action + "-in-pool-c", lambda cur, drawn: all(
            cur[n] != prior[n] and drawn[n] == cur[n] for n in outputs))
        assert all(current[n] in c for n in outputs), (action + " left pool C", prior, current)
    # At the start of the new history, previous stays put instead of returning to B or a probe.
    daemon.cli("previous", *outputs)
    time.sleep(.6)
    later, _ = daemon.wait("history-start", lambda cur, drawn: all(
        drawn[n] == cur[n] for n in outputs))
    assert all(later[n] == current[n] for n in outputs), ("previous left the new history", current, later)
    result["after_navigation"] = current
    result["final_set"] = set_each(finals, "final-set")
    daemon.cli("resume", *POOL_OUTPUTS)
    time.sleep(.3)
    status = daemon.status()
    assert all(status[n]["status"] == "running" for n in POOL_OUTPUTS), status


def group_set_reload(daemon, result):
    """Grouped outputs keep set parity and move into a new pool together (#2).

    Virtual-0..2 share group 1; Virtual-3 is ungrouped. A set on a grouped output holds only
    until any output decodes an image: that load pass hands the output the group's shared image
    back. This is pre-existing (measured on 2cde3d9 and c397225; the same get_image_path arm is
    in upstream 483225a) and pinned here as parity; the fix for wpaperd #18 must update it. Distinct
    per-output sets inside one group are therefore timing dependent and not asserted.
    """
    b, c = daemon.files("b"), daemon.files("c")
    probes = daemon.files("probe")
    grouped, control = POOL_OUTPUTS[:3], POOL_OUTPUTS[3]
    def config(pool):
        return ('[default]\npath=' + json.dumps(str(daemon.pool(pool))) + '\nmode="stretch"\n'
                'duration="2s"\ntransition-time=0\ninitial-transition=false\n' +
                ''.join('[' + n + ']\ngroup=1\n' for n in grouped))

    def coordinated(pool):
        def check(cur, drawn):
            return (all(cur[n] in pool and drawn[n] == cur[n] for n in POOL_OUTPUTS)
                    and len({cur[n] for n in grouped}) == 1)
        return check

    daemon.start(config("b"))
    daemon.wait("initial", coordinated(b))
    daemon.cli("pause", *POOL_OUTPUTS)

    # One grouped set with no later decode holds.
    daemon.cli("set", probes[0], grouped[0])
    daemon.wait("group-set", lambda cur, drawn: cur[grouped[0]] == drawn[grouped[0]] == probes[0])
    time.sleep(2.5)
    current, _ = daemon.wait("group-set-held", lambda cur, drawn: all(
        drawn[n] == cur[n] for n in POOL_OUTPUTS))
    assert current[grouped[0]] == probes[0], ("grouped set not held", current)
    shared = current[grouped[1]]
    assert all(current[n] == shared for n in grouped[1:]), current

    # Pinned pre-existing behavior: a decode on another output returns the shared image.
    before = current[control]
    daemon.cli("next", control)
    current, _ = daemon.wait("group-set-taken-back", lambda cur, drawn: (
        cur[control] in b and cur[control] != before
        and all(cur[n] == shared and drawn[n] == shared for n in grouped)))
    result["group_set_taken_back_by_other_decode"] = True

    # The reload moves the whole group and the control into the new pool, still coordinated.
    daemon.save(config("c"))
    current, _ = daemon.wait("group-reload-applied", coordinated(c))
    result["after_reload"] = current
    time.sleep(2.5)
    later, _ = daemon.wait("group-after-reload", coordinated(c))
    assert later == current, ("paused group changed after reload", current, later)
    status = daemon.status()
    assert all(status[n]["status"] == "paused" for n in POOL_OUTPUTS), status

    for action in ("next", "next", "previous"):
        prior = current
        daemon.cli(action, *grouped)
        current, _ = daemon.wait(action + "-group-in-pool-c", lambda cur, drawn: (
            coordinated(c)(cur, drawn) and all(cur[n] != prior[n] for n in grouped)))
    result["after_navigation"] = current
    daemon.cli("resume", *POOL_OUTPUTS)
    time.sleep(.3)
    status = daemon.status()
    assert all(status[n]["status"] == "running" for n in POOL_OUTPUTS), status


POOL_SCENARIOS = {"pause-reload": pause_reload, "set-reload": set_reload,
                  "group-set-reload": group_set_reload}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binaries", type=Path, default=Path("target/debug"))
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--producer-package", type=Path,
                        help="Published bundle for the exact two-output nested layout; copied only")
    parser.add_argument("--scenario", choices=("scene", "transition-reload", *POOL_SCENARIOS),
                        default="scene",
                        help="transition-reload rewrites transitions during four-output changes; "
                             "pause-reload, set-reload and group-set-reload change the pool "
                             "under pause, set and grouped outputs")
    parser.add_argument("--inside", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    args.binaries, args.repo = args.binaries.resolve(), args.repo.resolve()
    if args.producer_package:
        args.producer_package = args.producer_package.resolve()
    if args.inside:
        if args.scenario in POOL_SCENARIOS:
            pool_scenario(args, POOL_SCENARIOS[args.scenario])
        else:
            (transition_reload if args.scenario == "transition-reload" else inside)(args)
        return
    for tool in ("kwin_wayland", "dbus-run-session"):
        assert shutil.which(tool), "Missing dependency: " + tool
    for binary in ("wpaperd", "wpaperctl"):
        assert (args.binaries / binary).is_file(), "Build the locked workspace first"
    root = Path(tempfile.mkdtemp(prefix="wpaperd-scene-"))
    for name in ("home", "runtime", "config", "data", "cache"):
        (root / name).mkdir(mode=0o700)
    env = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "HOME": str(root / "home"),
           "XDG_RUNTIME_DIR": str(root / "runtime"), "XDG_CONFIG_HOME": str(root / "config"),
           "XDG_DATA_HOME": str(root / "data"), "XDG_CACHE_HOME": str(root / "cache"),
           "WAYLAND_DISPLAY": "wpaperd-scene-test", "LIBGL_ALWAYS_SOFTWARE": "1",
           "QT_QPA_PLATFORM": "offscreen"}
    print("Private evidence: " + str(root), flush=True)
    command = ["dbus-run-session", "/usr/bin/python3", str(Path(__file__).resolve()),
               "--binaries", str(args.binaries), "--repo", str(args.repo), "--inside", str(root),
               "--scenario", args.scenario]
    if args.producer_package:
        command += ["--producer-package", str(args.producer_package)]
    process = subprocess.Popen(command, env=env, start_new_session=True)
    try:
        code = process.wait(timeout=180 if args.scenario in POOL_SCENARIOS else 90)
        assert code == 0, "Private test failed; inspect " + str(root)
    finally:
        stop_owned(process)


if __name__ == "__main__":
    main()
