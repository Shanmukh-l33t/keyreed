#!/usr/bin/env python3
"""Functional tests for the KeyReed installer (Windows). Runs ONLY the sandbox build (--test), which has its
own registry keys, install folder and Start menu folder and never kills processes.

  python3 scripts/installer/test_installer.py dist/KeyReed-Setup-TEST.exe PAYLOAD_DIR

Checks: fresh install, silent upgrade over an existing install (files replaced, user data kept), uninstall,
reinstall after uninstall, cancel, and a failed install into an unwritable folder. It also records the real
KeyReed's config, registry values and shortcuts before and after and fails if any of them changed.
"""
import hashlib, json, os, re, subprocess, sys, time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import drive_setup as d

EXE, PAYLOAD = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
HOME = Path(os.environ["LOCALAPPDATA"])
INST = HOME / "KeyReedInstallerTest"
SM = Path(os.environ["APPDATA"]) / "Microsoft/Windows/Start Menu/Programs/KeyReed (installer test)"
REAL_CFG = HOME / "keyreed" / "config.json"
results = []
# Expected version comes from the installer script itself, so a release bump cannot leave this test stale.
APP_VERSION = re.search(r'!define APP_VERSION "([^"]+)"', (Path(__file__).parent.parent / "keyreed.nsi").read_text(encoding="utf-8")).group(1)


def ps(cmd):
    return subprocess.run(["powershell.exe", "-NoProfile", "-Command", cmd], capture_output=True, text=True).stdout.strip()


def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f"  [{detail}]" if detail else ""), flush=True)


def reg(path, name):
    return ps(f"(Get-ItemProperty 'HKCU:\\{path}' -ErrorAction SilentlyContinue).'{name}'")


def tree_hashes(root):
    return {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(root.rglob("*")) if p.is_file()}


def real_snapshot():
    """Everything of the user's real KeyReed that an installer test must never touch."""
    return {
        "config": hashlib.sha256(REAL_CFG.read_bytes()).hexdigest() if REAL_CFG.exists() else None,
        "run_key": reg(r"Software\Microsoft\Windows\CurrentVersion\Run", "Keyreed"),
        "install_dir": reg(r"Software\KeyReed", "InstallDir"), "version": reg(r"Software\KeyReed", "Version"),
        "uninstall": reg(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\KeyReed", "UninstallString"),
        "shortcuts": ps(r"Get-ChildItem (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\KeyReed') -ErrorAction SilentlyContinue | % { $_.Name + ' ' + $_.LastWriteTime.Ticks }"),
        "running_keyreed": ps("(Get-Process keyreed -ErrorAction SilentlyContinue | Measure-Object).Count"),
    }


def reset_sandbox():
    ps(f"Remove-Item -Recurse -Force '{INST}' -ErrorAction SilentlyContinue; "
       "Remove-Item -Recurse -Force 'HKCU:\\Software\\KeyReedInstallerTest','HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\KeyReedInstallerTest' -ErrorAction SilentlyContinue; "
       f"Remove-Item -Recurse -Force '{SM}' -ErrorAction SilentlyContinue; "
       "Remove-Item ([Environment]::GetFolderPath('Desktop') + '\\KeyReed (installer test).lnk') -ErrorAction SilentlyContinue")


def silent(*args, exe=EXE):
    return subprocess.run([str(exe), "/S", *args], timeout=180).returncode


def gui_cancel():
    p = subprocess.Popen([str(EXE)])
    hw = d.find("Setup", p.pid)
    time.sleep(0.6)
    d.click(hw, d.SECONDARY)  # the KeyReed "Cancel" button
    dlg = None
    for _ in range(30):  # the "are you sure" message box is a second top-level window of the same process
        time.sleep(0.2)
        dlg = d.find("", p.pid, timeout=0.3) if False else None
        h = d.u.FindWindowW("#32770", None)
        while h:
            pid = d.wt.DWORD(); d.u.GetWindowThreadProcessId(h, d.ctypes.byref(pid))
            if pid.value == p.pid and h != hw and d.u.IsWindowVisible(h):
                dlg = h; break
            h = d.u.FindWindowExW(None, h, "#32770", None)
        if dlg:
            break
    if dlg:
        d.shot(dlg, str(Path(os.environ.get("TMP_SHOTS", ".")) / "cancel_confirm.bmp"))
        d.u.PostMessageW(d.u.GetDlgItem(dlg, 6), d.BM_CLICK, 0, 0)  # IDYES
    try:
        p.wait(timeout=15)
    except subprocess.TimeoutExpired:
        p.kill()
    return dlg is not None, p.returncode


def main():
    before = real_snapshot()
    print("real install snapshot:", {k: (v[:12] + '..' if isinstance(v, str) and len(v) > 14 else v) for k, v in before.items()})
    reset_sandbox()
    payload_files = tree_hashes(PAYLOAD)

    # 1. fresh silent install
    rc = silent()
    check("fresh install exits 0", rc == 0, f"rc={rc}")
    got = tree_hashes(INST) if INST.exists() else {}
    got.pop("Uninstall.exe", None)
    check("installed files match the payload byte for byte", got == payload_files, f"{len(got)} vs {len(payload_files)} files")
    check("Uninstall.exe written", (INST / "Uninstall.exe").exists())
    u = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\KeyReedInstallerTest"
    check("uninstall entry: DisplayName/Publisher/Version", (reg(u, "DisplayName"), reg(u, "Publisher"), reg(u, "DisplayVersion")) == ("KeyReed", "KeyReed", APP_VERSION), str((reg(u, 'DisplayName'), reg(u, 'Publisher'), reg(u, 'DisplayVersion'))))
    check("uninstall entry: icon + quiet uninstall point into the install dir", reg(u, "DisplayIcon") == str(INST / "keyreed.exe") and "Uninstall.exe" in reg(u, "QuietUninstallString"))
    lnk = ps(f"$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{SM}\\KeyReed.lnk'); $s.TargetPath + '|' + $s.IconLocation")
    check("Start menu shortcut targets keyreed.exe with its icon", lnk.lower().startswith(str(INST / "keyreed.exe").lower() + "|" + str(INST / "keyreed.exe").lower()), lnk)
    st = ps(f"$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{SM}\\KeyReed Settings.lnk'); $s.Arguments")
    check("Settings shortcut passes --settings", st == "--settings", st)
    desk = Path(ps("[Environment]::GetFolderPath('Desktop')")) / "KeyReed (installer test).lnk"
    check("desktop shortcut created (default component selection)", desk.exists())

    # 2. upgrade over the existing install: replaced files, kept extra user files
    (INST / "keyreed.exe").write_bytes(b"stale")
    (INST / "user-note.txt").write_text("mine")
    rc = silent()
    check("upgrade/reinstall over existing install exits 0", rc == 0)
    check("upgrade restored the replaced program file", hashlib.sha256((INST / "keyreed.exe").read_bytes()).hexdigest() == payload_files["keyreed.exe"])
    check("upgrade kept an unrelated file in the folder", (INST / "user-note.txt").exists())

    # 2b. upgrade while the app is running from the install folder: it is stopped and its files replaced
    import shutil
    shutil.copy(Path(os.environ["SystemRoot"]) / "System32/PING.EXE", INST / "keyreed.exe")  # a stand-in process, never a real KeyReed
    running = subprocess.Popen([str(INST / "keyreed.exe"), "-t", "127.0.0.1"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(0.5)
    t0 = time.time()
    rc = silent()
    took = time.time() - t0
    check("upgrade over a running keyreed.exe exits 0 and stops it", rc == 0 and running.poll() is not None, f"rc={rc} running={running.poll() is None}")
    check("upgrade over a running keyreed.exe replaced the locked file", hashlib.sha256((INST / "keyreed.exe").read_bytes()).hexdigest() == payload_files["keyreed.exe"])
    print(f"  (silent upgrade over a running app took {took:.1f}s)")
    if running.poll() is None:
        running.kill()

    # 3. uninstall
    rc = subprocess.run([str(INST / "Uninstall.exe"), "/S", "_?=" + str(INST)], timeout=120).returncode
    time.sleep(1)
    (INST / "Uninstall.exe").unlink(missing_ok=True)
    check("uninstall exits 0", rc == 0, f"rc={rc}")
    left = [p.name for p in INST.iterdir()] if INST.exists() else []
    check("uninstall removed the program files", "keyreed.exe" not in left and "models" not in left, str(left))
    check("uninstall removed shortcuts and registry entries", not SM.exists() and not desk.exists() and reg(u, "DisplayName") == "" and reg("Software\\KeyReedInstallerTest", "InstallDir") == "")

    # 4. reinstall after uninstall
    reset_sandbox()
    rc = silent()
    check("reinstall after uninstall exits 0 and restores files", rc == 0 and (INST / "keyreed.exe").exists())

    # 5. cancel (GUI)
    reset_sandbox()
    asked, rc = gui_cancel()
    check("cancel asks for confirmation, then leaves nothing behind", asked and not INST.exists() and reg("Software\\KeyReedInstallerTest", "InstallDir") == "", f"asked={asked} rc={rc}")

    # 6. failure: unwritable folder (GUI path would block on the error box, so use the exit code of /S)
    bad = r"C:\Windows\System32\KeyReedInstallerTestDeny"
    rc = subprocess.run([str(EXE), "/S", "/D=" + bad], timeout=120).returncode
    check("install into an unwritable folder fails with a non-zero exit code", rc != 0, f"rc={rc}")
    check("failed install left no files and no uninstall entry", not Path(bad).exists() and reg(u, "DisplayName") == "")

    reset_sandbox()
    after = real_snapshot()
    check("real KeyReed untouched (config, Run key, install/uninstall registry, shortcuts, running instances)", before == after,
          "" if before == after else str({k: (before[k], after[k]) for k in before if before[k] != after[k]}))
    bad_n = [n for n, ok in results if not ok]
    print(f"\n{len(results) - len(bad_n)}/{len(results)} passed" + (f"; FAILED: {bad_n}" if bad_n else ""))
    sys.exit(1 if bad_n else 0)


if __name__ == "__main__":
    main()
