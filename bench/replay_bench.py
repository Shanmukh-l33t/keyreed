"""Replay WAV folder/list at mic speed through one KeyReed process. No clipboard or target delivery.
python bench/replay_bench.py target/release/keyreed.exe bench/corpus/real_v1/wavs
"""
import argparse
import json
import math
import statistics
import subprocess
import tempfile
from pathlib import Path
from run_bench import parse_json_stream


def stats(values):
    return statistics.median(values), sorted(values)[math.ceil(.95 * len(values)) - 1]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('exe', type=Path)
    ap.add_argument('input', type=Path)
    ap.add_argument('--config', type=Path, help='local settings; otherwise factory defaults')
    a = ap.parse_args()
    exe = str(a.exe.resolve(strict=True))
    help_text = subprocess.check_output([exe, '--help'], encoding='utf-8')
    if '--bench-wav-list' not in help_text or '--bench-config' not in help_text:
        ap.error('binary does not support isolated replay')
    paths = sorted(a.input.rglob('*.wav')) if a.input.is_dir() else [
        Path(s) for s in a.input.read_text(encoding='utf-8-sig').splitlines() if s.strip()]
    if not paths or not all(p.is_file() for p in paths):
        ap.error('empty WAV list or missing file')
    cfg = json.loads(a.config.read_text(encoding='utf-8-sig')) if a.config else {}
    cfg.update(processing_profile='max_privacy_offline', output_mode='direct_typing', debug_dump_wav=None)
    cfg.pop('cloud', None)
    with tempfile.TemporaryDirectory() as tmp:
        playlist, config = (Path(tmp) / name for name in ('list.txt', 'config.json'))
        playlist.write_text('\n'.join(str(p.resolve()) for p in paths), encoding='utf-8')
        config.write_text(json.dumps(cfg), encoding='utf-8')
        stdout = subprocess.check_output([exe, '--bench-wav-list', str(playlist), '--bench-config', str(config), '--live'], encoding='utf-8')
    rows = parse_json_stream(stdout)
    if [r['wav_index'] for r in rows] != list(range(len(paths))):
        raise RuntimeError('incomplete or out-of-order replay')
    metrics = [r['result']['metrics'] for r in rows]
    total = 'total_release_to_ready_ms'
    print(f'Cold replay: {metrics[0][total]:.1f} ms; warm n={len(metrics)-1}. No target delivery.')
    warm = metrics[1:]
    if warm:
        print('stage                         p50 ms    p95 ms  share')
        for key in ('post_release_asr_ms', 'rewrite_ms', total):
            values = [m[key] for m in warm]
            p50, p95 = stats(values)
            print(f'{key:28} {p50:7.1f} {p95:9.1f} {100*sum(values)/sum(m[total] for m in warm):5.1f}%')


if __name__ == '__main__':
    assert stats(range(1, 21)) == (10.5, 19)
    main()