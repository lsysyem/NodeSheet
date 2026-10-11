"""生成程序图标：assets/icon.png（窗口图标）和 assets/icon.ico（exe 图标，多种尺寸）。

图案：深灰圆角方块里一张表（橙色表头）+ 一个 Houdini 式的节点（右端蓝色显示旗标），黄色连线从表连到节点。
32px 及以下只画表，免得糊成一团。改了图案重新运行：python assets/make_icon.py
"""
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).parent
S = 1024  # 先画大图再缩小（抗锯齿）

BG = (43, 43, 43)
EDGE = (70, 70, 70)
TABLE = (64, 64, 64)
GRID = (96, 96, 96)
HEADER = (232, 163, 61)
NODE = (90, 90, 90)
NODE_EDGE = (22, 22, 22)
CAP = (68, 68, 68)
DISPLAY = (61, 139, 253)
WIRE = (242, 192, 55)


def background(d):
    d.rounded_rectangle((40, 40, S - 40, S - 40), radius=210, fill=EDGE)
    d.rounded_rectangle((64, 64, S - 64, S - 64), radius=190, fill=BG)


def table(d, box, header_h, cols, rows, line):
    x0, y0, x1, y1 = box
    r = 46
    d.rounded_rectangle(box, radius=r, fill=TABLE)
    # 表头：上面两个角圆、下面直
    d.rounded_rectangle((x0, y0, x1, y0 + header_h), radius=r, fill=HEADER)
    d.rectangle((x0, y0 + header_h - r, x1, y0 + header_h), fill=HEADER)
    w = (x1 - x0) / cols
    for c in range(1, cols):
        x = x0 + c * w
        d.line((x, y0 + header_h, x, y1 - 8), fill=GRID, width=line)
    h = (y1 - y0 - header_h) / rows
    for k in range(1, rows):
        y = y0 + header_h + k * h
        d.line((x0 + 8, y, x1 - 8, y), fill=GRID, width=line)


def full():
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    background(d)
    table(d, (170, 190, 690, 600), 120, 3, 3, 16)
    # 连线：表底下 → 节点顶上（三次贝塞尔取点画粗线）
    p0, p1, p2, p3 = (430, 600), (430, 700), (660, 640), (660, 728)
    pts = []
    for i in range(41):
        t = i / 40
        u = 1 - t
        pts.append((u ** 3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t ** 3 * p3[0],
                    u ** 3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t ** 3 * p3[1]))
    d.line(pts, fill=WIRE, width=30, joint="curve")
    for end in (p0, p3):
        d.ellipse((end[0] - 15, end[1] - 15, end[0] + 15, end[1] + 15), fill=WIRE)
    # 节点：窄条，左端旗标灰、右端显示旗标蓝
    n = (470, 730, 850, 860)
    d.rounded_rectangle((n[0] - 12, n[1] - 12, n[2] + 12, n[3] + 12), radius=46, fill=NODE_EDGE)
    d.rounded_rectangle(n, radius=36, fill=NODE)
    d.rounded_rectangle((n[0], n[1], n[0] + 80, n[3]), radius=36, fill=CAP)
    d.rectangle((n[0] + 40, n[1], n[0] + 80, n[3]), fill=CAP)
    d.rounded_rectangle((n[2] - 80, n[1], n[2], n[3]), radius=36, fill=DISPLAY)
    d.rectangle((n[2] - 80, n[1], n[2] - 40, n[3]), fill=DISPLAY)
    # 节点中间的小漏斗（筛选）
    cx, cy = (n[0] + n[2]) / 2, (n[1] + n[3]) / 2
    d.polygon([(cx - 48, cy - 38), (cx + 48, cy - 38), (cx + 12, cy + 4), (cx + 12, cy + 40),
               (cx - 12, cy + 30), (cx - 12, cy + 4)], fill=(120, 175, 255))
    return img


def small():
    """32px 及以下：只有表，线条更粗。"""
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    background(d)
    table(d, (190, 220, 834, 804), 190, 3, 2, 44)
    return img


def main():
    big, little = full(), small()
    sizes = [16, 24, 32, 48, 64, 128, 256]
    images = [(little if s <= 32 else big).resize((s, s), Image.LANCZOS) for s in sizes]
    big.resize((256, 256), Image.LANCZOS).save(HERE / "icon.png")
    images[-1].save(HERE / "icon.ico", sizes=[(s, s) for s in sizes], append_images=images[:-1])
    # 看一眼各尺寸的样子
    sheet = Image.new("RGBA", (sum(sizes) + 10 * len(sizes), 256), (28, 28, 28, 255))
    x = 0
    for s, im in zip(sizes, images):
        sheet.paste(im, (x, 256 - s), im)
        x += s + 10
    sheet.save(HERE / "icon-preview.png")


if __name__ == "__main__":
    main()
