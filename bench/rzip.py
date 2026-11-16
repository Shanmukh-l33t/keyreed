"""Stdlib-only remote ZIP reader: list / extract single members over HTTP Range.

usage: python3 rzip.py URL                 -> list members (name, size)
       python3 rzip.py URL PREFIX OUTDIR   -> extract members whose name starts with PREFIX
shortcut: one Range request per read() with a 1 MiB read-ahead cache; fine for
picking a few hundred files, switch to a full download for whole-archive use.
"""
import io, os, sys, urllib.request, zipfile

class HttpFile(io.RawIOBase):
    def __init__(self, url, block=1 << 20):
        self.url, self.pos, self.block, self.buf, self.bufpos, self.fetched = url, 0, block, b"", -1, 0
        r = urllib.request.urlopen(urllib.request.Request(url, headers={"Range": "bytes=0-0", "User-Agent": "curl/8"}))
        cr = r.headers.get("Content-Range")
        assert r.status == 206 and cr, f"server ignores Range (status {r.status})"  # else we'd pull the whole file
        self.size = int(cr.rsplit("/", 1)[1])
    def seekable(self): return True
    def readable(self): return True
    def tell(self): return self.pos
    def seek(self, off, whence=0):
        self.pos = off if whence == 0 else self.pos + off if whence == 1 else self.size + off
        return self.pos
    def read(self, n=-1):
        if n < 0: n = self.size - self.pos
        n = min(n, self.size - self.pos)
        if n <= 0: return b""
        if not (self.bufpos <= self.pos and self.pos + n <= self.bufpos + len(self.buf)):
            end = min(self.size, self.pos + max(n, self.block)) - 1
            req = urllib.request.Request(self.url, headers={"Range": f"bytes={self.pos}-{end}", "User-Agent": "curl/8"})
            self.buf, self.bufpos = urllib.request.urlopen(req).read(), self.pos
            self.fetched += len(self.buf)
        o = self.pos - self.bufpos
        self.pos += n
        return self.buf[o:o + n]

if __name__ == "__main__":
    f = HttpFile(sys.argv[1])
    z = zipfile.ZipFile(f)
    if len(sys.argv) == 2:
        for i in z.infolist(): print(i.file_size, i.filename)
    else:
        for i in z.infolist():
            if i.filename.startswith(sys.argv[2]) and not i.is_dir():
                z.extract(i, sys.argv[3]); print("extracted", i.filename)
    print(f"# archive {f.size} bytes, fetched {f.fetched} bytes", file=sys.stderr)
