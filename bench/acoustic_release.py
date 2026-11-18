"""Windows speaker→mic→owned EDIT benchmark for a running offline, Right Ctrl, direct-typing KeyReed.
Pass --exe <keyreed.exe> <test.wav> [--repeat N]. Requires an unlocked desktop; no transcript is logged.
Measures synthetic release to first and last observed text length change, not photons on screen.
"""
import argparse
import ctypes as C
from ctypes import wintypes as W
from pathlib import Path
import subprocess
import time
import winsound
import json
import wave


def main():
    a = argparse.ArgumentParser(description=__doc__)
    a.add_argument('--exe', required=True, type=Path)
    a.add_argument('wav', type=Path)
    a.add_argument('--repeat', type=int, default=1)
    args = a.parse_args()
    with wave.open(str(args.wav), 'rb') as w:
        duration = w.getnframes() / w.getframerate()
    u = C.windll.user32
    k = C.windll.kernel32
    p = subprocess.Popen([str(args.exe.resolve()), '--e2e-target-window'])
    try:
        hwnd = 0
        for _ in range(100):
            hwnd = u.FindWindowW('KeyreedE2ETargetWindow', None)
            pid = W.DWORD()
            u.GetWindowThreadProcessId(hwnd, C.byref(pid))
            if hwnd and pid.value == p.pid and u.GetDlgItem(hwnd, 201):
                break
            time.sleep(.05)
        else:
            raise RuntimeError('owned target not found; no input sent')
        edit = u.GetDlgItem(hwnd, 201)
        if not edit:
            raise RuntimeError('test EDIT not found')
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
        time.sleep(.2)
        for run in range(args.repeat):
            if u.GetForegroundWindow() != hwnd or p.poll() is not None:
                raise RuntimeError('focus changed; no hotkey sent')
            u.SendMessageW(edit, 0xC, 0, '')
            u.keybd_event(0xA3, 0x1D, 1, 0)
            try:
                time.sleep(.35)
                winsound.PlaySound(str(args.wav), winsound.SND_FILENAME | winsound.SND_NODEFAULT)
            finally:
                release = time.perf_counter()
                u.keybd_event(0xA3, 0x1D, 3, 0)
            first = last = None
            length = 0
            while time.perf_counter() - release < 30:
                if u.GetForegroundWindow() != hwnd or p.poll() is not None:
                    raise RuntimeError('target lost focus')
                n = u.SendMessageW(edit, 0xE, 0, 0)
                t = time.perf_counter() - release
                if n != length:
                    first = t if first is None else first
                    last, length = t, n
                if last is not None and t - last > 1:
                    break
                time.sleep(.005)
            print(json.dumps(dict(run=run+1, wav=args.wav.name, audio_s=duration, chars=length,
                                  first_ms=None if first is None else first*1000,
                                  last_change_ms=None if last is None else last*1000)), flush=True)
            if not length:
                raise RuntimeError('no target text within 30 s; this trial is a failure')
    finally:
        p.terminate()
        p.wait(5)


if __name__ == '__main__':
    main()
