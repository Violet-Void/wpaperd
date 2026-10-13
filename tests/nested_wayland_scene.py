#!/usr/bin/python3
"""Exercise scene IPC/rendering on two private KWin virtual outputs.

Run with /usr/bin/python3 tests/nested_wayland_scene.py --binaries target/debug.
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
        image.alpha_composite(sprite, (round(center[0] - 32), round(center[1] - 32)))
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


def inside(args):
    import dbus
    from PIL import Image
    from PySide6.QtGui import QImage
    root = args.inside
    def interrupted(signum, frame):
        raise SystemExit("Private test interrupted")
    signal.signal(signal.SIGTERM, interrupted)
    logs = [open(root / name, "wb") for name in ("compositor.log", "daemon.log")]
    compositor = daemon = None
    result = {"passed": False, "evidence": str(root)}
    commands = open(root / "commands.log", "w")
    try:
        env = dict(os.environ, KWIN_SCREENSHOT_NO_PERMISSION_CHECKS="1")
        compositor = subprocess.Popen([
            "kwin_wayland", "--virtual", "--socket", "wpaperd-scene-test", "--width", "800",
            "--height", "600", "--output-count", "2", "--no-lockscreen", "--no-kactivities",
            "--no-global-shortcuts"], env=env, stdout=logs[0], stderr=subprocess.STDOUT,
            start_new_session=True)
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
            assert (proc.returncode == 0) == success, proc.stderr + proc.stdout
            return proc.stdout

        def frames():
            return (root / "daemon.log").read_text(errors="replace").count(".frame(")

        time.sleep(.5)
        baseline = shot("still-before")
        assert baseline.getpixel((200, 180)) == (41, 49, 63)
        folder = package(root, args.repo)
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
            cli("scene", "shuffle")
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
        scene["assets"]["cyan"]["sha256"] = "0" * 64
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binaries", type=Path, default=Path("target/debug"))
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--inside", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    args.binaries, args.repo = args.binaries.resolve(), args.repo.resolve()
    if args.inside:
        inside(args)
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
    process = subprocess.Popen(["dbus-run-session", "/usr/bin/python3", str(Path(__file__).resolve()),
                                "--binaries", str(args.binaries), "--repo", str(args.repo),
                                "--inside", str(root)], env=env, start_new_session=True)
    try:
        code = process.wait(timeout=70)
        assert code == 0, "Private test failed; inspect " + str(root)
    finally:
        stop_owned(process)


if __name__ == "__main__":
    main()
