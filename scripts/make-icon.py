#!/usr/bin/env python3
"""Generate the app icon.

Renders the "T" lettermark on a navy squircle, resizes it into the standard
.iconset sizes with `sips`, and packs `assets/icon.icns` with `iconutil`. It
also writes `assets/icon.png`, which the About dialog embeds. The vector master
lives in assets/icon.svg; keep the three in sync when the artwork changes.
"""

import os
import shutil
import struct
import subprocess
import tempfile
import zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS = os.path.join(ROOT, "assets")
ICNS = os.path.join(ASSETS, "icon.icns")

BASE = 1024
SS = 2  # supersample factor for edge smoothing

# Container (macOS squircle) --------------------------------------------------
BOX0 = 64
BOX1 = 960
RADIUS = 200
BG_TOP = (30, 72, 130)      # #1E4882
BG_BOTTOM = (26, 62, 112)   # #1A3E70

# Letterform "T" -------------------------------------------------------------
LT_TOP = 225
LT_BOTTOM = 825             # total height 600
BAR_X0, BAR_X1 = 252, 772   # crossbar width 520
BAR_Y0, BAR_Y1 = 225, 385   # crossbar thickness 160
SERIF_W = 72
SERIF_Y0, SERIF_Y1 = 199, 411
STEM_X0, STEM_X1 = 412, 612  # stem width 200
FOOT_X0, FOOT_X1 = 387, 637
FOOT_Y0, FOOT_R = 710, 16
FG_TOP = (160, 224, 255)    # #A0E0FF
FG_BOTTOM = (96, 160, 255)  # #60A0FF

SHADOW = (16, 32, 64)       # #102040


def write_png(path, width, height, rgba):
    def chunk(kind, data):
        payload = kind + data
        crc = zlib.crc32(payload) & 0xFFFFFFFF
        return struct.pack(">I", len(data)) + payload + struct.pack(">I", crc)

    rows = bytearray()
    stride = width * 4
    for y in range(height):
        rows.append(0)
        rows += rgba[y * stride : (y + 1) * stride]

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", ihdr))
        f.write(chunk(b"IDAT", zlib.compress(bytes(rows), 9)))
        f.write(chunk(b"IEND", b""))


def _clamp01(v):
    return 0.0 if v < 0.0 else 1.0 if v > 1.0 else v


def _in_rounded_rect(x, y, x0, y0, x1, y1, r):
    if x < x0 or x > x1 or y < y0 or y > y1:
        return False
    cx = x0 + r if x < x0 + r else (x1 - r if x > x1 - r else x)
    cy = y0 + r if y < y0 + r else (y1 - r if y > y1 - r else y)
    dx = x - cx
    dy = y - cy
    return dx * dx + dy * dy <= r * r


def _in_rect(x, y, x0, y0, x1, y1):
    return x0 <= x <= x1 and y0 <= y <= y1


def _in_foot(x, y):
    if not (FOOT_X0 <= x <= FOOT_X1 and FOOT_Y0 <= y <= LT_BOTTOM):
        return False
    if y > LT_BOTTOM - FOOT_R:
        if x < FOOT_X0 + FOOT_R:
            dx = x - (FOOT_X0 + FOOT_R)
            dy = y - (LT_BOTTOM - FOOT_R)
            return dx * dx + dy * dy <= FOOT_R * FOOT_R
        if x > FOOT_X1 - FOOT_R:
            dx = x - (FOOT_X1 - FOOT_R)
            dy = y - (LT_BOTTOM - FOOT_R)
            return dx * dx + dy * dy <= FOOT_R * FOOT_R
    return True


def _in_letter(x, y):
    if _in_rect(x, y, BAR_X0, BAR_Y0, BAR_X1, BAR_Y1):
        return True
    if _in_rect(x, y, BAR_X0, SERIF_Y0, BAR_X0 + SERIF_W, SERIF_Y1):
        return True
    if _in_rect(x, y, BAR_X1 - SERIF_W, SERIF_Y0, BAR_X1, SERIF_Y1):
        return True
    if _in_rect(x, y, STEM_X0, LT_TOP, STEM_X1, LT_BOTTOM):
        return True
    return _in_foot(x, y)


def render(size):
    s = size * SS
    inv = 1.0 / (SS * SS)
    span = float(BOX1 - BOX0)
    tspan = float(LT_BOTTOM - LT_TOP)

    out = bytearray(size * size * 4)
    mask = [0.0] * (size * size)

    for oy in range(size):
        row = oy * size
        for ox in range(size):
            cont = 0
            br = bg = bb = 0.0
            tr = tg = tb = 0.0
            for sy in range(SS):
                y = oy + (sy + 0.5) / SS
                if y < BOX0 or y > BOX1:
                    continue
                for sx in range(SS):
                    x = ox + (sx + 0.5) / SS
                    if not _in_rounded_rect(x, y, BOX0, BOX0, BOX1, BOX1, RADIUS):
                        continue
                    cont += 1
                    if _in_letter(x, y):
                        t = _clamp01((y - LT_TOP) / tspan)
                        tr += FG_TOP[0] + (FG_BOTTOM[0] - FG_TOP[0]) * t
                        tg += FG_TOP[1] + (FG_BOTTOM[1] - FG_TOP[1]) * t
                        tb += FG_TOP[2] + (FG_BOTTOM[2] - FG_TOP[2]) * t
                    else:
                        t = _clamp01((y - BOX0) / span)
                        br += BG_TOP[0] + (BG_BOTTOM[0] - BG_TOP[0]) * t
                        bg += BG_TOP[1] + (BG_BOTTOM[1] - BG_TOP[1]) * t
                        bb += BG_TOP[2] + (BG_BOTTOM[2] - BG_TOP[2]) * t

            idx = row + ox
            mask[idx] = cont * inv
            if cont:
                i = idx * 4
                out[i] = int((br + tr) / cont + 0.5)
                out[i + 1] = int((bg + tg) / cont + 0.5)
                out[i + 2] = int((bb + tb) / cont + 0.5)
                out[i + 3] = int(cont * 255 * inv + 0.5)
    return out, mask


def _box_h(src, size, r):
    dst = [0.0] * (size * size)
    w = 2 * r + 1
    for y in range(size):
        base = y * size
        acc = 0.0
        for x in range(-r, r + 1):
            if 0 <= x < size:
                acc += src[base + x]
        for x in range(size):
            dst[base + x] = acc / w
            out_x = x - r
            in_x = x + r + 1
            if out_x >= 0:
                acc -= src[base + out_x]
            if in_x < size:
                acc += src[base + in_x]
    return dst


def _box_v(src, size, r):
    dst = [0.0] * (size * size)
    w = 2 * r + 1
    for x in range(size):
        acc = 0.0
        for y in range(-r, r + 1):
            if 0 <= y < size:
                acc += src[y * size + x]
        for y in range(size):
            dst[y * size + x] = acc / w
            out_y = y - r
            in_y = y + r + 1
            if out_y >= 0:
                acc -= src[out_y * size + x]
            if in_y < size:
                acc += src[in_y * size + x]
    return dst


def apply_shadow(out, mask, size):
    r = max(1, int(size * 0.018))
    blur = _box_h(_box_v(_box_h(_box_v(_box_h(_box_v(mask, size, r), size, r), size, r), size, r), size, r), size, r)
    dy = max(1, int(size * 0.013))
    strength = 0.38
    for oy in range(size):
        sy = oy - dy
        if sy < 0:
            continue
        sbase = sy * size
        for ox in range(size):
            sa = blur[sbase + ox] * strength
            if sa <= 0.0:
                continue
            i = (oy * size + ox) * 4
            oa = out[i + 3] / 255.0
            na = oa + sa * (1.0 - oa)
            if na <= 0.0:
                continue
            for c in range(3):
                out[i + c] = int((out[i + c] * oa + SHADOW[c] * sa * (1.0 - oa)) / na + 0.5)
            out[i + 3] = int(na * 255.0 + 0.5)


def main():
    if shutil.which("iconutil") is None or shutil.which("sips") is None:
        raise SystemExit("make-icon.py needs macOS `sips` and `iconutil`")

    os.makedirs(ASSETS, exist_ok=True)
    work = tempfile.mkdtemp()
    try:
        base = os.path.join(work, "base.png")
        rgba, mask = render(BASE)
        apply_shadow(rgba, mask, BASE)
        write_png(base, BASE, BASE, rgba)

        iconset = os.path.join(work, "AppIcon.iconset")
        os.makedirs(iconset)
        sizes = [
            (16, "16x16"),
            (32, "16x16@2x"),
            (32, "32x32"),
            (64, "32x32@2x"),
            (128, "128x128"),
            (256, "128x128@2x"),
            (256, "256x256"),
            (512, "256x256@2x"),
            (512, "512x512"),
            (1024, "512x512@2x"),
        ]
        for px, name in sizes:
            subprocess.run(
                ["sips", "-z", str(px), str(px), base, "--out", os.path.join(iconset, f"icon_{name}.png")],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )

        subprocess.run(["iconutil", "-c", "icns", iconset, "-o", ICNS], check=True)

        # PNG for the About dialog (embedded in the binary via include_bytes!).
        subprocess.run(
            ["sips", "-z", "256", "256", base, "--out", os.path.join(ASSETS, "icon.png")],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

        print(f"Wrote {ICNS}")
        print(f"Wrote {os.path.join(ASSETS, 'icon.png')}")
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
