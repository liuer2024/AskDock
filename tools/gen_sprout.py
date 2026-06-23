#!/usr/bin/env python3
"""Generate the built-in "sprout" pet spritesheet in Codex-Pets format.

Atlas: 1536x1872, 8 columns x 9 rows, each cell 192x208, transparent.
Row order (fixed by spec): idle, running-right, running-left, waving,
jumping, failed, waiting, running, review.

Each creature is drawn on a 2x tile then downsampled (LANCZOS) for clean
anti-aliased edges that look good when the webview scales it down further.
"""
import math
from PIL import Image, ImageDraw

CELL_W, CELL_H = 192, 208
COLS, ROWS = 8, 9
SS = 2  # supersample factor
TW, TH = CELL_W * SS, CELL_H * SS

# palette
BODY = (95, 207, 142, 255)      # friendly green
BODY_D = (60, 158, 102, 255)    # shade
OUTLINE = (28, 70, 52, 255)
BELLY = (210, 245, 226, 255)
LEAF = (120, 222, 160, 255)
LEAF_D = (74, 175, 116, 255)
EYE_W = (255, 255, 255, 255)
PUP = (30, 44, 38, 255)
CHEEK = (255, 150, 120, 110)
BOX = (230, 170, 96, 255)
BOX_D = (193, 130, 64, 255)


def lerp(a, b, t):
    return a + (b - a) * t


def rotate_paste(sheet, tile, col, row, angle=0.0):
    img = tile
    if angle:
        img = tile.rotate(angle, resample=Image.BICUBIC, center=(TW / 2, TH * 0.74))
    small = img.resize((CELL_W, CELL_H), Image.LANCZOS)
    sheet.alpha_composite(small, (col * CELL_W, row * CELL_H))


def draw_creature(p):
    """Draw one creature frame onto a fresh transparent tile (2x). Returns tile."""
    tile = Image.new("RGBA", (TW, TH), (0, 0, 0, 0))
    d = ImageDraw.Draw(tile)
    cx = TW / 2 + p.get("dx", 0) * SS
    # baseline (feet) near bottom of cell; bob shifts whole body up
    base = TH * 0.92 - p.get("bob", 0) * SS
    sq = p.get("squash", 1.0)            # >1 taller/narrower, <1 shorter/wider
    bw = 50 * SS / sq                    # body half-width
    bh = 54 * SS * sq                    # body half-height
    body_cy = base - bh

    # ---- feet ----
    fw, fh = 15 * SS, 9 * SS
    fl = p.get("foot_l", 0) * SS
    fr = p.get("foot_r", 0) * SS
    for sx, lift in ((-1, fl), (1, fr)):
        fx = cx + sx * bw * 0.5
        fy = base - lift
        d.ellipse([fx - fw, fy - fh, fx + fw, fy + fh], fill=BODY_D, outline=OUTLINE, width=2 * SS)

    # ---- arms ----
    def arm(side, pose):
        sx = side
        sh_x = cx + sx * bw * 0.86
        sh_y = body_cy + 6 * SS
        if pose == "up":
            ex, ey = sh_x + sx * 16 * SS, sh_y - 40 * SS
        elif pose == "wave1":
            ex, ey = sh_x + sx * 30 * SS, sh_y - 36 * SS
        elif pose == "wave2":
            ex, ey = sh_x + sx * 34 * SS, sh_y - 16 * SS
        elif pose == "cheek":
            ex, ey = sh_x - sx * 6 * SS, sh_y - 14 * SS  # 手抬到脸侧，撑着脸朝你张望
        elif pose == "hold":
            ex, ey = sh_x + sx * 6 * SS, sh_y + 26 * SS
        else:  # down
            ex, ey = sh_x + sx * 6 * SS, sh_y + 30 * SS
        d.line([sh_x, sh_y, ex, ey], fill=BODY_D, width=11 * SS)
        d.ellipse([ex - 8 * SS, ey - 8 * SS, ex + 8 * SS, ey + 8 * SS], fill=BODY, outline=OUTLINE, width=2 * SS)
    arm(-1, p.get("arm_l", "down"))
    arm(1, p.get("arm_r", "down"))

    # ---- body ----
    d.ellipse([cx - bw, body_cy - bh, cx + bw, body_cy + bh], fill=BODY, outline=OUTLINE, width=3 * SS)
    # belly highlight
    d.ellipse([cx - bw * 0.55, body_cy - bh * 0.1, cx + bw * 0.55, body_cy + bh * 0.78],
              fill=BELLY)

    # ---- sprout leaves on head ----
    if p.get("leaf", True):
        topx, topy = cx, body_cy - bh + 4 * SS
        if p.get("face") == "left":
            topx += 9 * SS  # 侧脸朝左时，芽偏向后脑（右）
        sway = p.get("leaf_sway", 0) * SS
        for sx in (-1, 1):
            lx = topx + sx * 12 * SS + sway
            d.ellipse([lx - 16 * SS, topy - 30 * SS, lx + 16 * SS, topy + 6 * SS],
                      fill=LEAF, outline=LEAF_D, width=2 * SS)
        d.line([topx, topy + 6 * SS, topx + sway, topy - 18 * SS], fill=LEAF_D, width=4 * SS)

    # ---- face ----
    eye = p.get("eyes", "open")
    ey = body_cy - 6 * SS
    er = 11 * SS  # eye radius
    look = p.get("look", 0) * SS
    # 朝向：front=正脸（双眼对称）；left=3/4 侧脸朝左（双眼偏左，远侧右眼更小更靠中线）。
    if p.get("face") == "left":
        eye_specs = [(-22 * SS, er), (5 * SS, er * 0.78)]
        mouth_cx = cx - 7 * SS
        cheek_offs = [-1]
    else:
        eye_specs = [(-20 * SS, er), (20 * SS, er)]
        mouth_cx = cx
        cheek_offs = [-1, 1]
    for off, r in eye_specs:
        ex = cx + off
        if eye == "blink":
            d.line([ex - r, ey, ex + r, ey], fill=PUP, width=3 * SS)
        elif eye == "happy":
            d.arc([ex - r, ey - r, ex + r, ey + r], start=200, end=340, fill=PUP, width=3 * SS)
        elif eye == "x":
            d.line([ex - r * 0.7, ey - r * 0.7, ex + r * 0.7, ey + r * 0.7], fill=PUP, width=3 * SS)
            d.line([ex - r * 0.7, ey + r * 0.7, ex + r * 0.7, ey - r * 0.7], fill=PUP, width=3 * SS)
        else:
            d.ellipse([ex - r, ey - r, ex + r, ey + r], fill=EYE_W, outline=OUTLINE, width=2 * SS)
            pr = 5 * SS * (r / er)
            d.ellipse([ex - pr + look, ey - pr + 2 * SS, ex + pr + look, ey + pr + 2 * SS], fill=PUP)
            d.ellipse([ex - pr + look + SS, ey - pr + 2 * SS, ex - pr + look + 4 * SS, ey - pr + 5 * SS], fill=EYE_W)

    # cheeks
    if eye in ("open", "happy"):
        for sx in cheek_offs:
            ex = cx + sx * (22 * SS)
            d.ellipse([ex - 7 * SS, ey + 8 * SS, ex + 7 * SS, ey + 18 * SS], fill=CHEEK)

    # mouth
    mouth = p.get("mouth", "smile")
    my = ey + 24 * SS
    if mouth == "o":
        d.ellipse([mouth_cx - 6 * SS, my - 6 * SS, mouth_cx + 6 * SS, my + 8 * SS], fill=PUP)
    elif mouth == "flat":
        d.line([mouth_cx - 9 * SS, my, mouth_cx + 9 * SS, my], fill=PUP, width=3 * SS)
    elif mouth == "wide":
        d.arc([mouth_cx - 16 * SS, my - 16 * SS, mouth_cx + 16 * SS, my + 8 * SS], start=15, end=165, fill=PUP, width=3 * SS)
    else:  # smile
        d.arc([mouth_cx - 11 * SS, my - 12 * SS, mouth_cx + 11 * SS, my + 6 * SS], start=20, end=160, fill=PUP, width=3 * SS)

    # ---- review box (hugged at the tummy, drawn last so it's in front) ----
    if p.get("box"):
        bx, by = cx, body_cy + bh * 0.72
        s = 16 * SS
        d.rectangle([bx - s, by - s, bx + s, by + s], fill=BOX, outline=BOX_D, width=3 * SS)
        d.line([bx - s, by - s * 0.15, bx + s, by - s * 0.15], fill=BOX_D, width=2 * SS)
        d.line([bx, by - s, bx, by + s], fill=BOX_D, width=2 * SS)

    return tile


def blink_seq(n, blink_at):
    return ["blink" if i in blink_at else "open" for i in range(n)]


def build_rows():
    """Return list of rows; each row is a list of param dicts (one per frame)."""
    rows = []

    # 0 idle (peek)：朝左探头、歪头看你；一只手撑着脸张望，偶尔招呼/眨眼。
    idle = []
    for i in range(6):
        ph = i / 6 * 2 * math.pi
        idle.append(dict(
            bob=3 + 2 * math.sin(ph),
            look=-9,                 # 眼睛看向左（你这边）
            rot=4,                  # 身体微微左倾＝探头
            leaf_sway=2 + 2 * math.sin(ph),
            arm_l="wave1" if i == 2 else "cheek",  # 近侧手：平时撑脸张望，偶尔招呼一下
            arm_r="down",
            eyes="blink" if i == 4 else "open",
            mouth="smile",
            face="left",
        ))
    rows.append(idle)

    # 1 running-right (8): lean right, legs cycle
    rr = []
    for i in range(8):
        ph = i / 8 * 2 * math.pi
        rr.append(dict(bob=3 + 4 * abs(math.sin(ph)), dx=4, look=4,
                       foot_l=max(0, 14 * math.sin(ph)), foot_r=max(0, 14 * math.sin(ph + math.pi)),
                       arm_l="up" if math.sin(ph) > 0 else "down",
                       arm_r="down" if math.sin(ph) > 0 else "up",
                       eyes="open", mouth="o"))
    rows.append(rr)

    # 2 running-left (8): mirror
    rl = []
    for i in range(8):
        ph = i / 8 * 2 * math.pi
        rl.append(dict(bob=3 + 4 * abs(math.sin(ph)), dx=-4, look=-4,
                       foot_l=max(0, 14 * math.sin(ph + math.pi)), foot_r=max(0, 14 * math.sin(ph)),
                       arm_l="down" if math.sin(ph) > 0 else "up",
                       arm_r="up" if math.sin(ph) > 0 else "down",
                       eyes="open", mouth="o"))
    rows.append(rl)

    # 3 waving (4): 侧脸朝你、用近侧手（左）招手，歪着头开心
    wv = [
        dict(bob=4, look=-7, rot=4, face="left", arm_l="wave1", arm_r="down", eyes="happy", mouth="wide", leaf_sway=2),
        dict(bob=5, look=-7, rot=4, face="left", arm_l="wave2", arm_r="down", eyes="happy", mouth="wide", leaf_sway=-2),
        dict(bob=4, look=-7, rot=4, face="left", arm_l="wave1", arm_r="down", eyes="happy", mouth="wide", leaf_sway=2),
        dict(bob=3, look=-7, rot=4, face="left", arm_l="cheek", arm_r="down", eyes="open", mouth="smile", leaf_sway=0),
    ]
    rows.append(wv)

    # 4 jumping (5): crouch, launch, peak, fall, land
    jp = [
        dict(bob=0, squash=0.82, foot_l=0, foot_r=0, eyes="open", mouth="flat"),
        dict(bob=14, squash=1.12, arm_l="up", arm_r="up", eyes="happy", mouth="o"),
        dict(bob=30, squash=1.06, arm_l="up", arm_r="up", eyes="happy", mouth="wide"),
        dict(bob=14, squash=1.02, arm_l="up", arm_r="up", eyes="open", mouth="o"),
        dict(bob=0, squash=0.86, eyes="open", mouth="smile"),
    ]
    rows.append(jp)

    # 5 failed (8): droop + shake, X eyes
    fl = []
    for i in range(8):
        ph = i / 8 * 2 * math.pi
        fl.append(dict(bob=-2, dx=3 * math.sin(ph * 2), leaf_sway=-6,
                       eyes="x" if i % 4 < 2 else "blink", mouth="flat",
                       arm_l="down", arm_r="down"))
    rows.append(fl)

    # 6 waiting (6): stare at you, occasional blink + look around
    wt = []
    for i in range(6):
        wt.append(dict(bob=2, look=[-5, -5, 0, 5, 5, 0][i],
                       eyes="blink" if i == 3 else "open", mouth="flat", leaf_sway=1))
    rows.append(wt)

    # 7 running (6): generic happy run-in-place
    rn = []
    for i in range(6):
        ph = i / 6 * 2 * math.pi
        rn.append(dict(bob=3 + 4 * abs(math.sin(ph)),
                       foot_l=max(0, 14 * math.sin(ph)), foot_r=max(0, 14 * math.sin(ph + math.pi)),
                       arm_l="up" if math.sin(ph) > 0 else "down",
                       arm_r="down" if math.sin(ph) > 0 else "up",
                       eyes="open", mouth="o"))
    rows.append(rn)

    # 8 review (6): holds a box, happy, little nod
    rv = []
    for i in range(6):
        ph = i / 6 * 2 * math.pi
        rv.append(dict(bob=2 + 2 * math.sin(ph), box=True, arm_l="hold", arm_r="hold",
                       eyes="happy", mouth="wide", leaf_sway=1))
    rows.append(rv)

    return rows


def main():
    sheet = Image.new("RGBA", (CELL_W * COLS, CELL_H * ROWS), (0, 0, 0, 0))
    rows = build_rows()
    assert len(rows) == ROWS, len(rows)
    for r, frames in enumerate(rows):
        for c, p in enumerate(frames):
            tile = draw_creature(p)
            rotate_paste(sheet, tile, c, r, angle=p.get("rot", 0))
    out = "/private/tmp/claude-501/-Users-smiler-Documents-98-personal-askDock/3da0c8ab-3892-413c-8c13-b5713dbf3289/scratchpad/sprout-spritesheet.png"
    sheet.save(out)
    # also a single idle frame preview enlarged
    prev = sheet.crop((0, 0, CELL_W, CELL_H)).resize((CELL_W * 2, CELL_H * 2), Image.LANCZOS)
    prev.save(out.replace("spritesheet", "preview-idle"))
    print("wrote", out, sheet.size)


if __name__ == "__main__":
    main()
