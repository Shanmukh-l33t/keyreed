#!/usr/bin/env python3
"""Drive a KeyReed setup EXE through its real wizard and screenshot every page (Windows, stdlib only).

  python3 scripts/installer/drive_setup.py dist/KeyReed-Setup-TEST.exe OUTDIR [--cancel] [--fail DIR] [--uninstall]

Pages are captured from the screen (what a user sees), buttons are real BM_CLICKs on the wizard's
own controls. Only run it against the --test build (own registry keys / folders); see keyreed.nsi.
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


def click(hwnd, ctl):
    b = u.GetDlgItem(hwnd, ctl)
    assert b, f"control {ctl} not found"
    u.PostMessageW(b, BM_CLICK, 0, 0)


def btn_text(hwnd, ctl):
    b = u.GetDlgItem(hwnd, ctl)
    n = 64; s = ctypes.create_unicode_buffer(n); u.GetWindowTextW(b, s, n); return s.value


def wait_btn(hwnd, ctl, want, timeout=60):
    end = time.time() + timeout
    while time.time() < end:
        if want in btn_text(hwnd, ctl).replace("&", "") and u.IsWindowEnabled(u.GetDlgItem(hwnd, ctl)):
            return True
        time.sleep(0.1)
    return False


def main():
    exe, out = Path(sys.argv[1]).resolve(), Path(sys.argv[2]); out.mkdir(parents=True, exist_ok=True)
    flags = sys.argv[3:]
    args = [str(exe)] + (["/D=" + flags[flags.index("--fail") + 1]] if "--fail" in flags else [])
    p = subprocess.Popen(args)
    hw = find("Setup", p.pid)
    assert hw, "setup window not found"
    u.SetForegroundWindow(hw); time.sleep(0.6)
    pages = ["01_welcome"]
    shot(hw, out / "01_welcome.bmp")
    if "--cancel" in flags:
        click(hw, IDCANCEL); time.sleep(0.8)
        dlg = find("Setup", p.pid)  # abort confirmation is a message box
        shot(hw, out / "cancel_confirm.bmp")
        print("cancel path captured; buttons:", btn_text(hw, IDCANCEL)); return
    click(hw, IDOK); time.sleep(0.7); shot(hw, out / "02_directory.bmp")
    click(hw, IDOK); time.sleep(0.7); shot(hw, out / "03_components.bmp")
    click(hw, IDOK)  # Install
    frames, t0, installing = 0, time.time(), False
    while time.time() - t0 < 120:
        shot(hw, out / f"progress_{frames:03d}.bmp"); frames += 1
        en = u.IsWindowEnabled(u.GetDlgItem(hw, IDOK))
        installing = installing or not en
        if (installing or time.time() - t0 > 3) and en:
            break  # the Next/Close button is enabled again: the engine has finished
        time.sleep(0.1)
    time.sleep(0.5)
    print("progress frames:", frames, "in %.1fs" % (time.time() - t0), "| button:", btn_text(hw, IDOK))
    shot(hw, out / "05_finish.bmp")  # MUI moves to the Finish page by itself when the install succeeds
    if "--fail" in flags:
        return
    click(hw, IDOK)
    p.wait(timeout=30); print("setup exit code", p.returncode)


if __name__ == "__main__":
    main()
