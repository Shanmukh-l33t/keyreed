"""Live-path probe: start KeyReed itself (simulated mic) plus its owned target window, press and
release Right Ctrl, and report what the target received.

acoustic_release.py only spawns the *target* window, so it needs an already-running KeyReed. This
one starts both, which is what makes the insertion stage measurable on a headless-ish desktop.
Pass --exe <keyreed.exe> <wav> [--repeat N].
"""
import argparse
import ctypes as C
from ctypes import wintypes as W
import json
import os
import subprocess
import sys
import time
import wave
from pathlib import Path


def attach_foreground(hwnd, u, k):
    tid = u.GetWindowThreadProcessId(hwnd, None)
    me = k.GetCurrentThreadId()
    fg = u.GetWindowThreadProcessId(u.GetForegroundWindow(), None)
    fg_attached = fg not in (0, tid, me) and u.AttachThreadInput(me, fg, True)
    attached = u.AttachThreadInput(me, tid, True)
    try:
        u.ShowWindow(hwnd, 9)
        u.BringWindowToTop(hwnd)
        u.SetForegroundWindow(hwnd)
        u.SendMessageW(hwnd, 0x400 + 70, 0, 0)
    finally:
        if attached:
            u.AttachThreadInput(me, tid, False)
        if fg_attached:
            u.AttachThreadInput(me, fg, False)


def main():
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument('--exe', required=True, type=Path)
    a.add_argument('wav', type=Path)
    a.add_argument('--repeat', type=int, default=3)
    a.add_argument('--speed', type=float, default=6.0)
    a.add_argument('--gap', type=float, default=None,
                   help='seconds to wait for text before the next press; 0 = fire the next one immediately')
    args = a.parse_args()
    with wave.open(str(args.wav), 'rb') as w:
        duration = w.getnframes() / w.getframerate()

    u = C.windll.user32
    k = C.windll.kernel32
    env = dict(os.environ, KEYREED_TEST_AUDIO_WAV=str(args.wav.resolve()),
               KEYREED_TEST_AUDIO_SPEED=str(args.speed))
    daemon = subprocess.Popen([str(args.exe.resolve())], env=env,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    window = subprocess.Popen([str(args.exe.resolve()), '--e2e-target-window'], env=env,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        hwnd = 0
        for _ in range(160):
            hwnd = u.FindWindowW('KeyreedE2ETargetWindow', None)
            pid = W.DWORD()
            u.GetWindowThreadProcessId(hwnd, C.byref(pid))
            if hwnd and pid.value == window.pid and u.GetDlgItem(hwnd, 201):
                break
            time.sleep(.05)
        else:
            raise RuntimeError('target window not found')
        edit = u.GetDlgItem(hwnd, 201)
        time.sleep(3)  # let the speech model finish loading before the first press
        attach_foreground(hwnd, u, k)
        time.sleep(.3)
        for run in range(args.repeat):
            if u.GetForegroundWindow() != hwnd:
                attach_foreground(hwnd, u, k)
                time.sleep(.2)
                if u.GetForegroundWindow() != hwnd:
                    raise RuntimeError('target lost focus; no hotkey sent')
            before = u.SendMessageW(edit, 0xE, 0, 0)
            time.sleep(.05)
            u.keybd_event(0xA3, 0x1D, 1, 0)
            time.sleep(.3 + duration / args.speed)
            release = time.perf_counter()
            u.keybd_event(0xA3, 0x1D, 3, 0)
            first = last = None
            length = 0
            while time.perf_counter() - release < 25:
                n = u.SendMessageW(edit, 0xE, 0, 0)
                t = time.perf_counter() - release
                if n != length:
                    first = t if first is None else first
                    last, length = t, n
                if args.gap is not None and t >= args.gap:
                    break
                if args.gap is None and last is not None and t - last > 1.5:
                    break
                time.sleep(.005)
            print(json.dumps(dict(run=run + 1, audio_s=duration, chars=length - before,
                                  first_ms=None if first is None else round(first * 1000),
                                  last_change_ms=None if last is None else round(last * 1000))), flush=True)
        # Back-to-back presses are fired before the previous insert lands: let the queue drain so the
        # trial reports what the app actually delivered, not what had arrived when the last gap ended.
        if args.gap is not None:
            time.sleep(8)
            print(json.dumps(dict(run='final', chars=u.SendMessageW(edit, 0xE, 0, 0) - before)), flush=True)
    finally:
        window.terminate()
        daemon.terminate()
        for p in (window, daemon):
            try:
                p.wait(5)
            except Exception:
                pass


if __name__ == '__main__':
    sys.exit(main())