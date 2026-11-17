"""Gallery helpers with the standard library only (no Pillow on the build machine).

    python scripts/design/shots.py png <dir>                      # every .bmp in <dir> -> .png, the .bmp removed
    python scripts/design/shots.py sheet <out.png> <cols> <img>...  # images (bmp or png) on one contact sheet
    python scripts/design/shots.py strip <out.png> <img>...         # one row, for filmstrips

Gallery BMPs are 32-bit top-down (negative height); PNGs are written as 8-bit RGB.
"""
import struct
import sys
import zlib
from pathlib import Path

BG = (0x2A, 0x2E, 0x2C)  # neutral grey-green between light and dark shots
GAP = 16


def read_bmp(p):
    b = Path(p).read_bytes()
    off, = struct.unpack_from("<I", b, 10)
    w, h, _, bpp = struct.unpack_from("<iiHH", b, 18)
    assert bpp == 32, f"{p}: {bpp} bpp"
    top_down = h < 0
    h = abs(h)
    rows = []
    for y in range(h):
        sy = y if top_down else h - 1 - y
        src = b[off + sy * w * 4: off + (sy + 1) * w * 4]
        row = bytearray(w * 3)
        row[0::3], row[1::3], row[2::3] = src[2::4], src[1::4], src[0::4]
        rows.append(bytes(row))
    return w, h, rows


def read_png(p):
    b = Path(p).read_bytes()
    assert b[:8] == b"\x89PNG\r\n\x1a\n"
    i, idat, w = 8, b"", 0
    while i < len(b):
        n, = struct.unpack_from(">I", b, i)
        kind, data = b[i + 4:i + 8], b[i + 8:i + 8 + n]
        if kind == b"IHDR":
            w, h, depth, ctype = struct.unpack_from(">IIBB", data)
            assert (depth, ctype) == (8, 2), "only the RGB PNGs this script writes"
        elif kind == b"IDAT":
            idat += data
        i += 12 + n
    raw, stride, rows, prev = zlib.decompress(idat), w * 3, [], bytearray(w * 3)
    for y in range(h):
        f, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        assert f in (0, 2), "filter"
        if f == 2:
            for k in range(stride):
                line[k] = (line[k] + prev[k]) & 0xFF
        rows.append(bytes(line))
        prev = line
    return w, h, rows


def read(p):
    return read_png(p) if str(p).lower().endswith(".png") else read_bmp(p)


def write_png(p, w, h, rows):
    raw = b"".join(b"\x00" + r for r in rows)
    chunk = lambda k, d: struct.pack(">I", len(d)) + k + d + struct.pack(">I", zlib.crc32(k + d) & 0xFFFFFFFF)
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    Path(p).write_bytes(png)


def sheet(out, cols, paths):
    imgs = [read(p) for p in paths]
    cols = max(1, min(cols, len(imgs)))
    grid = [imgs[i:i + cols] for i in range(0, len(imgs), cols)]
    cw = [max(r[c][0] for r in grid if c < len(r)) for c in range(cols)]
    rh = [max(im[1] for im in r) for r in grid]
    W, H = sum(cw) + GAP * (cols + 1), sum(rh) + GAP * (len(grid) + 1)
    canvas = [bytearray(bytes(BG) * W) for _ in range(H)]
    y = GAP
    for r, row in enumerate(grid):
        x = GAP
        for c, (w, h, rows) in enumerate(row):
            for k in range(h):
                canvas[y + k][x * 3:(x + w) * 3] = rows[k]
            x += cw[c] + GAP
        y += rh[r] + GAP
    write_png(out, W, H, [bytes(r) for r in canvas])


if __name__ == "__main__":
    cmd = sys.argv[1]
    if cmd == "png":
        for bmp in sorted(Path(sys.argv[2]).glob("*.bmp")):
            write_png(bmp.with_suffix(".png"), *read_bmp(bmp))
            bmp.unlink()
    elif cmd == "sheet":
        sheet(sys.argv[2], int(sys.argv[3]), sys.argv[4:])
    elif cmd == "strip":
        sheet(sys.argv[2], len(sys.argv) - 3, sys.argv[3:])
    else:
        sys.exit(__doc__)
