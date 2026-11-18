#!/usr/bin/env python3
"""Test KeyReedSetup::StopApp (the native "stop a running KeyReed" step of the installer and uninstaller).

  python3 scripts/installer/test_stop_app.py

Builds a 64-bit copy of the plugin (the shipped one is 32-bit and cannot be loaded into 64-bit Python), starts decoy
processes (ping.exe copied under the names keyreed.exe and llama-server.exe), calls StopApp and checks that exactly
the right ones end: the keyreed.exe and llama-server.exe that run from the install folder, and nothing else. It never
touches a real KeyReed: the decoys live in a temp folder, and StopApp only ends processes that run from that folder.
Needs llvm-mingw and NSIS under ~/.local/toolchain, like build-installer.sh.
"""
import ctypes, os, shutil, subprocess, sys, tempfile, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TOOLS = Path(os.environ.get("KEYREED_TOOLCHAIN", Path.home() / ".local/toolchain"))
results = []


def check(name, ok, detail=""):
    results.append(bool(ok))
    print(("PASS " if ok else "FAIL ") + name + (f"  [{detail}]" if detail else ""), flush=True)


def build(out):
    cc = TOOLS / "llvm-mingw/bin/x86_64-w64-mingw32-clang.exe"
    nsis = TOOLS / "nsis/Examples/Plugin"
    subprocess.run([str(cc), "-shared", "-O1", "-DUNICODE", "-D_UNICODE", f"-I{nsis}", str(ROOT / "scripts/installer/kr_setup.c"),
                    "-o", str(out), "-lgdi32", "-luser32", "-lole32", "-luuid", "-lm"], check=True)


def decoy(path):
    path.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy(Path(os.environ["SystemRoot"]) / "System32/PING.EXE", path)
    return subprocess.Popen([str(path), "-t", "127.0.0.1"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def stop_app(dll, folder):
    k = ctypes.windll.kernel32
    k.GlobalAlloc.restype = ctypes.c_void_p
    text = str(folder)
    node = k.GlobalAlloc(0x40, 8 + (len(text) + 1) * 2)  # kr_stack: next pointer, then the text
    ctypes.memmove(node + 8, ctypes.create_unicode_buffer(text), (len(text) + 1) * 2)
    top = ctypes.c_void_p(node)
    t0 = time.time()
    dll.StopApp(None, 1024, None, ctypes.byref(top), None)
    return time.time() - t0


def main():
    work = Path(tempfile.mkdtemp(prefix="kr_stop_"))
    dll = ctypes.CDLL(str(build_to(work)))
    inst, other = work / "KeyReed", work / "Other"
    procs = {
        "keyreed.exe in the install folder": decoy(inst / "keyreed.exe"),
        "llama-server.exe in runtime/llama": decoy(inst / "runtime/llama/llama-server.exe"),
        "keyreed.exe elsewhere": decoy(other / "keyreed.exe"),
        "llama-server.exe elsewhere": decoy(other / "llama-server.exe"),
        "another exe in the install folder": decoy(inst / "helper.exe"),
    }
    time.sleep(0.5)
    took = stop_app(dll, inst)
    time.sleep(0.2)
    alive = {n: p.poll() is None for n, p in procs.items()}
    check("ends keyreed.exe running from the install folder", not alive["keyreed.exe in the install folder"])
    check("ends the llama-server.exe running from runtime/llama", not alive["llama-server.exe in runtime/llama"])
    check("leaves a keyreed.exe running from another folder", alive["keyreed.exe elsewhere"])
    check("leaves a llama-server.exe running from another folder", alive["llama-server.exe elsewhere"])
    check("leaves other programs in the install folder", alive["another exe in the install folder"])
    check("takes well under a second (was 5-7 s)", took < 1.0, f"{took * 1000:.0f} ms")
    try:  # the files are released: they can be replaced or deleted right away
        (inst / "keyreed.exe").unlink()
        (inst / "runtime/llama/llama-server.exe").unlink()
        released = True
    except OSError as e:
        released = False
        print(e)
    check("its files are released by the time it returns", released)
    again = stop_app(dll, inst)
    check("nothing to stop returns at once", again < 0.3, f"{again * 1000:.0f} ms")
    for p in procs.values():
        if p.poll() is None:
            p.kill()
    time.sleep(0.3)
    shutil.rmtree(work, ignore_errors=True)
    print(f"{sum(results)}/{len(results)} passed")
    sys.exit(0 if all(results) else 1)


def build_to(work):
    out = work / "KeyReedSetup64.dll"
    build(out)
    return out


if __name__ == "__main__":
    main()
