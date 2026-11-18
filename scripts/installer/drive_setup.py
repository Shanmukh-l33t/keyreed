#!/usr/bin/env python3
"""Drive a KeyReed setup EXE through its real wizard and screenshot every page (Windows, stdlib only).

  python3 scripts/installer/drive_setup.py dist/KeyReed-Setup-TEST.exe OUTDIR [--cancel] [--fail DIR]
  python3 scripts/installer/drive_setup.py <install dir>/Uninstall.exe OUTDIR --uninstall

Every screen is captured with PrintWindow (what a user sees); buttons are real BM_CLICKs on the
KeyReedSetup buttons. Only run it against the --test build (own registry keys / folders); see keyreed.nsi.
"""
import ctypes, ctypes.wintypes as wt, subprocess, sys, time, struct
from pathlib import Path

u, g = ctypes.windll.user32, ctypes.windll.gdi32
u.SetProcessDPIAware()
WNDENUM = ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
BM_CLICK, IDOK, IDCANCEL = 0x00F5, 1, 2


def find(title_part, pid=None, timeout=20):
    end = time.time() + timeout
    while time.time() < end:
        hit = []

        def cb(h, _):
            n = u.GetWindowTextLengthW(h)
            if n and u.IsWindowVisible(h):
                b = ctypes.create_unicode_buffer(n + 1); u.GetWindowTextW(h, b, n + 1)
                p = wt.DWORD(); u.GetWindowThreadProcessId(h, ctypes.byref(p))
                if title_part in b.value and (pid is None or p.value == pid):
                    hit.append(h)
            return True
        u.EnumWindows(WNDENUM(cb), 0)
        if hit:
            return hit[0]
        time.sleep(0.1)
    return None


def shot(hwnd, path):
    """Render the window itself with PrintWindow(PW_RENDERFULLCONTENT): independent of what covers it."""
    r = wt.RECT(); u.GetWindowRect(hwnd, ctypes.byref(r))
    w, h = r.right - r.left, r.bottom - r.top
    sdc = u.GetDC(0); mdc = g.CreateCompatibleDC(sdc); bmp = g.CreateCompatibleBitmap(sdc, w, h)
    g.SelectObject(mdc, bmp)
    u.PrintWindow(hwnd, mdc, 2)
    bi = struct.pack("<IiiHHIIiiII", 40, w, -h, 1, 24, 0, 0, 0, 0, 0, 0)
    stride = (w * 3 + 3) & ~3; buf = ctypes.create_string_buffer(stride * h)
    g.GetDIBits(mdc, bmp, 0, h, buf, bi, 0)
    data = bytes(buf)
    Path(path).write_bytes(b"BM" + struct.pack("<IHHI", 54 + len(data), 0, 0, 54) +
                           struct.pack("<IiiHHIIiiII", 40, w, -h, 1, 24, 0, len(data), 2835, 2835, 0, 0) + data)
    g.DeleteObject(bmp); g.DeleteDC(mdc); u.ReleaseDC(0, sdc)


def overlay(hwnd, timeout=10):
    """The KeyReedSetup window that covers the wizard (scripts/installer/kr_setup.c)."""
    end = time.time() + timeout
    while time.time() < end:
        o = u.FindWindowExW(hwnd, None, "KeyReedSetup", None)
        if o:
            return o
        time.sleep(0.1)
    return None


# Control ids of the KeyReedSetup buttons.
PRIMARY, SECONDARY, CHANGE, OPTION = 2001, 2002, 2003, 2004


def click(hwnd, ctl):
    """Click a KeyReed button (ids above) on the overlay of wizard `hwnd`."""
    b = u.GetDlgItem(overlay(hwnd), ctl)
    assert b, f"control {ctl} not found"
    u.PostMessageW(b, BM_CLICK, 0, 0)


def btn_text(hwnd, ctl):
    b = u.GetDlgItem(overlay(hwnd), ctl)
    n = 64; s = ctypes.create_unicode_buffer(n); u.GetWindowTextW(b, s, n); return s.value


def wait_btn(hwnd, ctl, want, timeout=120):
    end = time.time() + timeout
    while time.time() < end:
        if u.IsWindow(hwnd) and want in btn_text(hwnd, ctl):
            return True
        time.sleep(0.05)
    return False


def run(exe, out, title, flags, args=()):
    p = subprocess.Popen([str(exe), *args])
    hw = find(title, None if "uninstall" in title.lower() else p.pid)
    assert hw, f"{title} window not found"
    assert overlay(hw), "KeyReed screen not found"
    u.SetForegroundWindow(hw); time.sleep(1.4)  # the page entrance takes 0.9 s
    shot(hw, out / "01_welcome.bmp")
    if "--cancel" in flags:
        click(hw, SECONDARY); time.sleep(0.8)
        print("cancel path captured"); return p, hw
    click(hw, PRIMARY)
    frames, t0 = 0, time.time()
    while time.time() - t0 < 120:
        if "--quiet" not in flags:  # screenshots add paints of their own; --quiet is for frame-timing runs
            shot(hw, out / f"progress_{frames:03d}.bmp")
        frames += 1
        if btn_text(hw, PRIMARY) in ("Finish", "Close") or btn_text(hw, SECONDARY) == "Close":
            break
        time.sleep(0.05)
    print("progress frames:", frames, "in %.1fs" % (time.time() - t0))
    time.sleep(1.0)  # let the one-off completion ring finish drawing
    shot(hw, out / "03_finish.bmp")
    if btn_text(hw, SECONDARY) == "Close":
        print("install failed: failure screen captured"); return p, hw
    click(hw, PRIMARY)
    return p, hw


def main():
    exe, out = Path(sys.argv[1]).resolve(), Path(sys.argv[2]); out.mkdir(parents=True, exist_ok=True)
    flags = sys.argv[3:]
    args = ["/D=" + flags[flags.index("--fail") + 1]] if "--fail" in flags else []
    title = "Uninstall" if "--uninstall" in flags else "Setup"
    p, _ = run(exe, out, title, flags, args)
    try:
        p.wait(timeout=30); print("exit code", p.returncode)
    except subprocess.TimeoutExpired:
        print("still running (waiting on a dialog)")


if __name__ == "__main__":
    main()
