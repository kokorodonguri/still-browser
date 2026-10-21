#!/usr/bin/env python3
"""Capture a normally launched Firefox window, without WebDriver's UI markings.

Run under xvfb-run or on a desktop. Commands on stdin: capture NAME, click X Y,
key KEYSYM, quit. Captures preserve the actual pixels shown by X11.
"""
import argparse
from pathlib import Path
import subprocess
import tempfile
import time

import mss
from mss.tools import to_png
from Xlib import X, XK, display
from Xlib.ext import xtest

ROOT = Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser()
parser.add_argument("--profile", type=Path)
args = parser.parse_args()
temporary = tempfile.TemporaryDirectory(prefix="still-native-ui-") if not args.profile else None
profile = args.profile or Path(temporary.name) / "profile"
output = ROOT / "artifacts" / "linux-checks"
output.mkdir(parents=True, exist_ok=True)
browser = subprocess.Popen([str(ROOT / "linux" / "still"), "--profile", str(profile)], stdout=(output / "native-launch.log").open("w"), stderr=subprocess.STDOUT)
x11 = display.Display()
window = None
deadline = time.monotonic() + 60
while time.monotonic() < deadline:
    for candidate in x11.screen().root.query_tree().children:
        if candidate.get_wm_class() and any("still" in s or "Firefox" in s or "firefox" in s for s in candidate.get_wm_class()):
            if candidate.get_attributes().map_state == X.IsViewable:
                window = candidate
                break
    if window:
        break
    if browser.poll() is not None:
        raise SystemExit("Firefox stopped before displaying a window. See native-launch.log.")
    time.sleep(.2)
if not window:
    browser.terminate()
    raise SystemExit("Firefox did not display a window within 60 seconds.")
window.set_input_focus(X.RevertToParent, X.CurrentTime)
x11.sync()
print("Firefox mapped. Commands: capture NAME, click X Y, key KEYSYM, quit", flush=True)
try:
    with mss.MSS() as screen:
        for line in iter(input, "quit"):
            command, *values = line.split()
            if command == "capture" and len(values) == 1:
                name = Path(values[0]).name
                if not name.endswith(".png"):
                    name += ".png"
                bounds = window.get_geometry()
                position = window.translate_coords(x11.screen().root, 0, 0)
                pixels = screen.grab({"left": -position.x, "top": -position.y, "width": bounds.width, "height": bounds.height})
                to_png(pixels.rgb, pixels.size, output=str(output / name))
                print("Saved", output / name, flush=True)
            elif command == "click" and len(values) == 2:
                xtest.fake_input(x11, X.MotionNotify, x=int(values[0]), y=int(values[1]))
                xtest.fake_input(x11, X.ButtonPress, 1)
                xtest.fake_input(x11, X.ButtonRelease, 1)
                x11.sync()
            elif command == "key" and len(values) == 1:
                code = x11.keysym_to_keycode(XK.string_to_keysym(values[0]))
                xtest.fake_input(x11, X.KeyPress, code)
                xtest.fake_input(x11, X.KeyRelease, code)
                x11.sync()
            else:
                print("Unknown command", flush=True)
except EOFError:
    pass
finally:
    browser.terminate()
    browser.wait(timeout=30)
    x11.close()
    if temporary:
        temporary.cleanup()
