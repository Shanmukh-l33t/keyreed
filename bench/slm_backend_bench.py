import subprocess, time, urllib.request, json, sys, os
EXE_DIR = r"C:\Users\ImMandatory\keyreed\bench\cache\llama-vk"
CPU_DIR = r"C:\Users\ImMandatory\keyreed\dist\keyreed-win-x64\runtime\llama"
GGUF = r"C:\Users\ImMandatory\keyreed\dist\keyreed-win-x64\models\Qwen3.5-0.8B-Q4_K_M.gguf"
PORT = 18950
SYS = "You are a conservative voice-dictation editor. Fix grammar, punctuation, and capitalization while preserving exact meaning, names, numbers, and identifiers. Never add facts. Output ONLY the edited text."
TEXTS = [
    "we checked the logs but we are not able to understand why is the worker crashing on port 8080",
    "the patch fixes the memory leak in the audio buffer however it also changes the default sample rate so we need to update the documentation and notify the mobile team before we merge it",
    "thank you for your time yesterday it was great to learn about your roadmap and we would like to schedule a follow up call next week to discuss pricing and the integration timeline with the platform team",
]


def run(label, exe_dir, extra):
    t0 = time.perf_counter()
    p = subprocess.Popen([os.path.join(exe_dir, "llama-server.exe"), "-fit", "off", "-m", GGUF, "--host", "127.0.0.1", "--port", str(PORT),
                          "-c", "2048", "-np", "1", "--reasoning-budget", "0"] + extra, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(400):
            try:
                if urllib.request.urlopen(f"http://127.0.0.1:{PORT}/health", timeout=0.3).status == 200:
                    break
            except Exception:
                time.sleep(0.05)
        boot = (time.perf_counter() - t0) * 1000
        res = []
        for rep in range(3):
            for t in TEXTS:
                body = json.dumps({"id_slot": 0, "messages": [{"role": "system", "content": SYS}, {"role": "user", "content": f"<transcript>{t}</transcript>"}],
                                   "temperature": 0, "max_tokens": 120, "cache_prompt": True, "chat_template_kwargs": {"enable_thinking": False}}).encode()
                t1 = time.perf_counter()
                r = json.loads(urllib.request.urlopen(urllib.request.Request(f"http://127.0.0.1:{PORT}/v1/chat/completions", data=body, headers={"Content-Type": "application/json"}), timeout=60).read())
                res.append(((time.perf_counter() - t1) * 1000, r["timings"]["predicted_per_second"], r["choices"][0]["message"]["content"], len(t.split())))
        warm = [x for i, x in enumerate(res) if i >= 3]
        ms = sorted(x[0] for x in warm)
        print(f"{label:34s} boot={boot:6.0f}ms warm p50={ms[len(ms)//2]:5.0f}ms max={ms[-1]:5.0f}ms tok/s={sum(x[1] for x in warm)/len(warm):5.1f}")
        for x in res[:3]:
            print("    ", x[3], "words ->", repr(x[2][:90]))
    finally:
        p.kill()
        p.wait()


run("CPU 4 threads (shipped)", CPU_DIR, ["-t", "4", "-tb", "4", "-ngl", "0", "-sm", "none"])
run("CPU 6 threads", CPU_DIR, ["-t", "6", "-tb", "6", "-ngl", "0", "-sm", "none"])
run("Vulkan Quadro P2000 (ngl 99)", EXE_DIR, ["-ngl", "99", "--device", "Vulkan1", "-t", "2"])
run("Vulkan UHD 630 iGPU (ngl 99)", EXE_DIR, ["-ngl", "99", "--device", "Vulkan0", "-t", "2"])
