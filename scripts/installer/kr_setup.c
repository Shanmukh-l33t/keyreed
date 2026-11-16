/*
 * KeyReedSetup: NSIS plugin that draws the whole KeyReed installer and uninstaller.
 *
 * NSIS stays the install engine (files, registry, shortcuts, uninstaller, upgrade, silent mode).
 * This plugin covers the wizard's client area with one KeyReed window per stage:
 *
 *   Welcome   logo, product, install location (Change...), desktop-shortcut switch, Install button
 *   Progress  "voice becomes text": a live waveform; bars left of the REAL progress point settle
 *             into lines of text. Progress and the status line are read from the installer's own
 *             progress bar (1004) and status text (1006) every frame. The animation never drives
 *             or delays progress.
 *   Finish    "KeyReed is ready." with a one-off ring draw-in, launch switch, Finish button
 *
 * The NSIS controls stay alive underneath (hidden); our buttons send the same WM_COMMAND IDOK /
 * IDCANCEL the wizard buttons would, so page flow, the abort warning and Esc/Enter keep working.
 *
 * Drawing: GDI text + signed-distance shapes into a 32-bit DIB. No GDI+, no runtime, no browser.
 * The static layer is cached; only the waveform band and status line repaint (30 fps) while
 * installing, and nothing repaints on the static screens. Reduced motion ("Animate controls"
 * off) or high contrast: static bars, no ring animation.
 *
 * Build (32-bit unicode NSIS plugin): see scripts/build-installer.sh
 */
#define COBJMACROS
#include <windows.h>
#include <shobjidl.h>
#include <math.h>
#include "nsis/api.h"

#define CLS L"KeyReedSetup"
#define ID_PRIMARY 2001
#define ID_SECONDARY 2002
#define ID_CHANGE 2003
#define ID_OPT 2004
#define ID_CLOSE 2005
#define TIP_MS 4200.0
#define WM_KR_FOCUS (WM_APP + 1)
#define INST_INSTDIR 21
#define PI 3.14159265358979
#define LOOP_MS 2600.0
#define CW 600 /* logical client size */
#define CH 440
#define BAND_N 60

typedef unsigned int u32;
typedef struct kr_stack { struct kr_stack *next; WCHAR text[1]; } kr_stack;

enum { S_NONE, S_WELCOME, S_PROGRESS, S_DONE, S_FAILED, S_FINISH };

static HINSTANCE g_inst;
static HWND g_parent, g_ov, g_bar, g_status, g_hot;
static WCHAR *g_vars;
static int g_strsz;
static int g_stage = S_NONE, g_uninstall = 0, g_animate = 1, g_hc = 0;
static int g_opt_desktop = 1, g_opt_launch = 1, g_round = 0;
static WCHAR g_version[32];
static double g_s = 1.0;
static DWORD g_t0, g_finish_t0;
static HFONT f_title, f_h2, f_tag, f_body, f_small, f_btn, f_label, f_icon;
static char g_word_end[BAND_N];

/* Render target (window DIB or a button DIB). */
typedef struct { u32 *px; int w, h; HDC dc; HBITMAP bmp; } Surf;
static Surf g_main;
static u32 *g_cache; /* static layer of the window */
static int g_cache_ok;
static Surf *T;

/* ---------------- palette (dark evergreen, KeyReed emerald) ---------------- */
static u32 rgb(int r, int g, int b) { return ((u32)r << 16) | ((u32)g << 8) | (u32)b; }
#define C_BG rgb(0x0C, 0x14, 0x11)
#define C_FOOT rgb(0x09, 0x10, 0x0D)
#define C_CARD rgb(0x12, 0x1D, 0x18)
#define C_LINE rgb(0x23, 0x33, 0x2B)
#define C_TEXT rgb(0xF2, 0xF7, 0xF4)
#define C_TEXT2 rgb(0xA9, 0xBB, 0xB2)
#define C_TEXT3 rgb(0x8D, 0xA3, 0x98)
#define C_MINT rgb(0x6E, 0xE7, 0xB7)
#define C_EM rgb(0x34, 0xD3, 0x99)
#define C_EM_HOT rgb(0x6E, 0xE7, 0xB7)
#define C_EM_PRESS rgb(0x10, 0xB9, 0x81)
#define C_ON_EM rgb(0x04, 0x24, 0x1A)
#define C_WAVE_LO rgb(0x1E, 0x3A, 0x2F)
#define C_INK rgb(0xD5, 0xE6, 0xDD)
#define C_ROSE rgb(0xFD, 0xA4, 0xAF)

static u32 mix(u32 a, u32 b, double t) {
  if (t <= 0) return a;
  if (t >= 1) return b;
  int r = (int)(((a >> 16) & 255) * (1 - t) + ((b >> 16) & 255) * t + .5);
  int g = (int)(((a >> 8) & 255) * (1 - t) + ((b >> 8) & 255) * t + .5);
  int bl = (int)((a & 255) * (1 - t) + (b & 255) * t + .5);
  return rgb(r, g, bl);
}
static double clamp01(double v) { return v < 0 ? 0 : v > 1 ? 1 : v; }
static double S(double v) { return v * g_s; }

/* ---------------- surfaces and primitives ---------------- */
static int surf_make(Surf *s, HDC ref, int w, int h) {
  BITMAPINFO bi = {{sizeof(BITMAPINFOHEADER), w, -h, 1, 32, BI_RGB}};
  s->bmp = CreateDIBSection(ref, &bi, DIB_RGB_COLORS, (void **)&s->px, NULL, 0);
  if (!s->bmp) return 0;
  s->dc = CreateCompatibleDC(ref);
  SelectObject(s->dc, s->bmp);
  SetBkMode(s->dc, TRANSPARENT);
  s->w = w; s->h = h;
  return 1;
}
static void surf_free(Surf *s) {
  if (s->dc) DeleteDC(s->dc);
  if (s->bmp) DeleteObject(s->bmp);
  s->dc = NULL; s->bmp = NULL; s->px = NULL;
}
static void fill(u32 c) { for (int i = 0; i < T->w * T->h; i++) T->px[i] = c; }
static void rect(int x, int y, int w, int h, u32 c) {
  for (int yy = y < 0 ? 0 : y; yy < y + h && yy < T->h; yy++)
    for (int xx = x < 0 ? 0 : x; xx < x + w && xx < T->w; xx++) T->px[yy * T->w + xx] = c;
}
/* Signed distance of a rounded rectangle centred at (cx,cy). */
static double sd_rrect(double px, double py, double cx, double cy, double hw, double hh, double r) {
  double qx = fabs(px - cx) - (hw - r), qy = fabs(py - cy) - (hh - r);
  double ox = qx > 0 ? qx : 0, oy = qy > 0 ? qy : 0;
  double in = qx > qy ? qx : qy;
  return sqrt(ox * ox + oy * oy) + (in < 0 ? in : 0) - r;
}
static void rrect(double x, double y, double w, double h, double r, u32 c, double a) {
  if (w <= 0 || h <= 0) return;
  if (r > w / 2) r = w / 2;
  if (r > h / 2) r = h / 2;
  double cx = x + w / 2, cy = y + h / 2;
  for (int yy = (int)floor(y) - 1; yy <= (int)ceil(y + h) + 1; yy++) {
    if (yy < 0 || yy >= T->h) continue;
    for (int xx = (int)floor(x) - 1; xx <= (int)ceil(x + w) + 1; xx++) {
      if (xx < 0 || xx >= T->w) continue;
      double cov = clamp01(.5 - sd_rrect(xx + .5, yy + .5, cx, cy, w / 2, h / 2, r)) * a;
      if (cov > 0) T->px[yy * T->w + xx] = mix(T->px[yy * T->w + xx], c, cov);
    }
  }
}
static void rrect_stroke(double x, double y, double w, double h, double r, double sw, u32 c) {
  double cx = x + w / 2, cy = y + h / 2;
  for (int yy = (int)floor(y) - 1; yy <= (int)ceil(y + h) + 1; yy++) {
    if (yy < 0 || yy >= T->h) continue;
    for (int xx = (int)floor(x) - 1; xx <= (int)ceil(x + w) + 1; xx++) {
      if (xx < 0 || xx >= T->w) continue;
      double d = sd_rrect(xx + .5, yy + .5, cx, cy, w / 2, h / 2, r);
      double cov = clamp01(.5 - d) * clamp01(d + sw + .5);
      if (cov > 0) T->px[yy * T->w + xx] = mix(T->px[yy * T->w + xx], c, cov);
    }
  }
}
/* Soft radial light behind the logo: one restrained glow, not a gradient background. */
static void glow(double cx, double cy, double rad, u32 c, double amax) {
  for (int yy = (int)(cy - rad); yy < (int)(cy + rad); yy++) {
    if (yy < 0 || yy >= T->h) continue;
    for (int xx = (int)(cx - rad); xx < (int)(cx + rad); xx++) {
      if (xx < 0 || xx >= T->w) continue;
      double d = sqrt((xx - cx) * (xx - cx) + (yy - cy) * (yy - cy)) / rad;
      if (d >= 1) continue;
      double t = 1 - d;
      T->px[yy * T->w + xx] = mix(T->px[yy * T->w + xx], c, amax * t * t * (3 - 2 * t));
    }
  }
}
/* Ring arc clockwise from 12 o'clock, `sweep` 0..1. */
static void arc(double cx, double cy, double r, double sw, double sweep, u32 c) {
  double end = sweep * 2 * PI;
  for (int yy = (int)(cy - r - sw); yy <= (int)(cy + r + sw); yy++) {
    if (yy < 0 || yy >= T->h) continue;
    for (int xx = (int)(cx - r - sw); xx <= (int)(cx + r + sw); xx++) {
      if (xx < 0 || xx >= T->w) continue;
      double dx = xx + .5 - cx, dy = yy + .5 - cy, len = sqrt(dx * dx + dy * dy);
      double rc = clamp01(sw / 2 - fabs(len - r) + .5);
      if (rc <= 0) continue;
      double a = atan2(dx, -dy);
      if (a < 0) a += 2 * PI;
      double ac = a <= end ? 1 : clamp01(1 - (a - end) * len);
      if (rc * ac > 0) T->px[yy * T->w + xx] = mix(T->px[yy * T->w + xx], c, rc * ac);
    }
  }
}
static void text(const WCHAR *s, HFONT f, u32 c, int x, int y, int w, int h, UINT fmt) {
  RECT rc = {x, y, x + w, y + h};
  SelectObject(T->dc, f);
  SetTextColor(T->dc, RGB((c >> 16) & 255, (c >> 8) & 255, c & 255));
  DrawTextW(T->dc, s, -1, &rc, fmt | DT_NOPREFIX);
}
/* The official icon embedded in this setup/uninstall EXE (NSIS icon resource 103), at size n. */
static void logo(int x, int y, int n) {
  HICON ic = (HICON)LoadImageW(GetModuleHandleW(NULL), MAKEINTRESOURCEW(103), IMAGE_ICON, n, n, LR_DEFAULTCOLOR);
  if (!ic) return;
  DrawIconEx(T->dc, x, y, ic, n, n, 0, NULL, DI_NORMAL);
  DestroyIcon(ic);
}

/* ---------------- fonts ---------------- */
static HFONT mkfont(const WCHAR *face, double px, int weight) {
  return CreateFontW(-(int)(px * g_s + .5), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, OUT_TT_PRECIS, 0, CLEARTYPE_QUALITY, 0, face);
}
static int face_exists(const WCHAR *face) {
  HFONT f = mkfont(face, 12, 400);
  HDC dc = GetDC(NULL);
  HGDIOBJ o = SelectObject(dc, f);
  WCHAR got[64] = {0};
  GetTextFaceW(dc, 64, got);
  SelectObject(dc, o);
  ReleaseDC(NULL, dc);
  DeleteObject(f);
  return lstrcmpiW(got, face) == 0;
}
static void fonts(void) {
  if (f_title) return;
  const WCHAR *disp = face_exists(L"Segoe UI Variable Display") ? L"Segoe UI Variable Display" : L"Segoe UI";
  const WCHAR *txt = face_exists(L"Segoe UI Variable Text") ? L"Segoe UI Variable Text" : L"Segoe UI";
  f_title = mkfont(disp, 34, FW_SEMIBOLD);
  f_h2 = mkfont(disp, 24, FW_SEMIBOLD);
  f_tag = mkfont(disp, 17, FW_NORMAL);
  f_body = mkfont(txt, 15, FW_NORMAL);
  f_small = mkfont(txt, 13, FW_NORMAL);
  f_btn = mkfont(txt, 14.5, FW_SEMIBOLD);
  f_label = mkfont(txt, 14.5, FW_NORMAL);
  f_icon = mkfont(face_exists(L"Segoe Fluent Icons") ? L"Segoe Fluent Icons" : L"Segoe MDL2 Assets", 10, FW_NORMAL);
}

/* Shown while installing, one at a time. Each one describes a shipped feature. */
static const WCHAR *TIPS[] = {
  L"Hold Right Ctrl in any app, speak, and let go. Your words are typed where your cursor is.",
  L"Speech is turned into text on this PC. Cloud processing is off unless you add your own key.",
  L"Prefer not to hold a key? Switch to toggle-to-talk in Settings: press once to start, again to stop.",
  L"Add names and jargon to your personal dictionary so they come out right every time.",
  L"Writing modes shape the result: polished prose, your exact words, or code-friendly text.",
  L"KeyReed can adapt to the app you are in: professional in email, casual in chat.",
  L"KeyReed never types into password fields.",
};
#define N_TIPS (int)(sizeof(TIPS) / sizeof(TIPS[0]))

/* ---------------- NSIS helpers ---------------- */
static void pop(void *stacktop, WCHAR *out, int n) {
  kr_stack **st = (kr_stack **)stacktop;
  out[0] = 0;
  if (!st || !*st) return;
  kr_stack *t = *st;
  lstrcpynW(out, t->text, n);
  *st = t->next;
  GlobalFree(t);
}
static void push(void *stacktop, const WCHAR *s) {
  kr_stack **st = (kr_stack **)stacktop;
  if (!st) return;
  kr_stack *t = (kr_stack *)GlobalAlloc(GPTR, sizeof(kr_stack) + g_strsz * sizeof(WCHAR));
  if (!t) return;
  lstrcpynW(t->text, s, g_strsz);
  t->next = *st;
  *st = t;
}
static WCHAR *instdir(void) { return g_vars ? g_vars + (size_t)INST_INSTDIR * g_strsz : L""; }

/* ---------------- layout ---------------- */
static int foot_y(void) { return (int)S(CH - 76); }

/* Word boundaries of the "text" the waveform turns into: fixed, 2-7 bars per word. */
static void words(void) {
  unsigned int x = 2166136261u;
  int left = 0;
  for (int i = 0; i < BAND_N; i++) {
    if (left == 0) { x = x * 16777619u ^ (unsigned)i; left = 2 + (int)(x % 6); }
    left--;
    g_word_end[i] = (left == 0) || i == BAND_N - 1;
  }
}

static double progress(void) {
  if (g_stage == S_DONE) return 1;
  if (!g_bar) return 0;
  LRESULT hi = SendMessageW(g_bar, 0x0407 /* PBM_GETRANGE */, FALSE, 0);
  LRESULT pos = SendMessageW(g_bar, 0x0408 /* PBM_GETPOS */, 0, 0);
  return hi > 0 ? clamp01((double)pos / (double)hi) : 0;
}

/* The static layer: everything except the waveform band, status line and the finish ring. */
static void paint_static(void) {
  int W = T->w, H = T->h;
  fill(C_BG);
  WCHAR buf[600];
  if (g_stage == S_WELCOME) {
    glow(S(84), S(70), S(260), rgb(0x10, 0xB9, 0x81), .10);
    logo((int)S(44), (int)S(42), (int)S(64));
    text(L"KeyReed", f_title, C_TEXT, (int)S(44), (int)S(122), (int)S(500), (int)S(46), DT_SINGLELINE);
    if (g_uninstall) {
      text(L"Remove KeyReed from this PC?", f_tag, C_MINT, (int)S(44), (int)S(168), (int)S(500), (int)S(26), DT_SINGLELINE);
      text(L"The app, its Start menu entries and the desktop shortcut are removed.", f_body, C_TEXT2, (int)S(44), (int)S(200), (int)S(512), (int)S(22), DT_SINGLELINE | DT_END_ELLIPSIS);
    } else {
      text(L"Voice it. Everywhere.", f_tag, C_MINT, (int)S(44), (int)S(168), (int)S(500), (int)S(26), DT_SINGLELINE);
      text(L"Turn your speech into polished text, anywhere on Windows.", f_body, C_TEXT2, (int)S(44), (int)S(200), (int)S(512), (int)S(22), DT_SINGLELINE | DT_END_ELLIPSIS);
    }
    /* Details card */
    double cx = S(44), cy = S(240), cw = S(512), ch = g_uninstall ? S(84) : S(104);
    rrect(cx, cy, cw, ch, S(12), C_CARD, 1);
    rrect_stroke(cx, cy, cw, ch, S(12), 1, C_LINE);
    if (g_uninstall) {
      text(L"KEPT ON THIS PC", f_small, C_TEXT3, (int)(cx + S(18)), (int)(cy + S(14)), (int)S(460), (int)S(18), DT_SINGLELINE);
      text(L"Your settings, personal dictionary and downloaded speech models", f_label, C_TEXT, (int)(cx + S(18)), (int)(cy + S(34)), (int)S(476), (int)S(20), DT_SINGLELINE | DT_END_ELLIPSIS);
      text(L"stay in %LOCALAPPDATA%\\keyreed, ready for a reinstall.", f_label, C_TEXT2, (int)(cx + S(18)), (int)(cy + S(55)), (int)S(476), (int)S(20), DT_SINGLELINE | DT_END_ELLIPSIS);
    } else {
      text(L"INSTALL LOCATION", f_small, C_TEXT3, (int)(cx + S(18)), (int)(cy + S(13)), (int)S(300), (int)S(18), DT_SINGLELINE);
      lstrcpynW(buf, instdir(), 600);
      int bl = lstrlenW(buf);
      if (bl > 3 && buf[bl - 1] == L'\\') buf[bl - 1] = 0; /* display only; $INSTDIR is unchanged */
      text(buf, f_label, C_TEXT, (int)(cx + S(18)), (int)(cy + S(32)), (int)S(370), (int)S(20), DT_SINGLELINE | DT_PATH_ELLIPSIS);
      rect((int)(cx + S(18)), (int)(cy + S(62)), (int)(cw - S(36)), 1, C_LINE);
    }
    /* Footer */
    rect(0, foot_y(), W, H - foot_y(), C_FOOT);
    rect(0, foot_y(), W, 1, C_LINE);
    wsprintfW(buf, L"Version %s", g_version);
    text(buf, f_small, C_TEXT3, (int)S(44), foot_y() + (int)S(20), (int)S(200), (int)S(18), DT_SINGLELINE);
    text(g_uninstall ? L"Uninstall" : L"No administrator rights needed", f_small, C_TEXT3, (int)S(44), foot_y() + (int)S(38), (int)S(220), (int)S(18), DT_SINGLELINE | DT_END_ELLIPSIS);
  } else if (g_stage == S_PROGRESS || g_stage == S_DONE || g_stage == S_FAILED) {
    glow(S(CW / 2), S(78), S(240), rgb(0x10, 0xB9, 0x81), .09);
    logo((int)S(CW / 2 - 28), (int)S(50), (int)S(56));
    const WCHAR *title = g_stage == S_FAILED ? (g_uninstall ? L"KeyReed was not fully removed" : L"KeyReed was not installed")
                       : g_stage == S_DONE ? (g_uninstall ? L"KeyReed is removed" : L"KeyReed is installed")
                       : (g_uninstall ? L"Removing KeyReed" : L"Installing KeyReed");
    text(title, f_h2, C_TEXT, 0, (int)S(116), W, (int)S(36), DT_SINGLELINE | DT_CENTER);
    if (g_stage != S_FAILED)
      text(g_uninstall ? L"Your settings and speech models stay on this PC." : L"Voice it. Everywhere.", g_uninstall ? f_body : f_tag,
           g_uninstall ? C_TEXT2 : C_MINT, 0, (int)S(154), W, (int)S(24), DT_SINGLELINE | DT_CENTER);
    if (g_stage == S_FAILED) {
      rect(0, foot_y(), W, H - foot_y(), C_FOOT);
      rect(0, foot_y(), W, 1, C_LINE);
    }
  } else if (g_stage == S_FINISH) {
    glow(S(CW / 2), S(92), S(260), rgb(0x10, 0xB9, 0x81), .11);
    logo((int)S(CW / 2 - 36), (int)S(56), (int)S(72));
    text(g_uninstall ? L"KeyReed has been removed." : L"KeyReed is ready.", f_title, C_TEXT, 0, (int)S(176), W, (int)S(46), DT_SINGLELINE | DT_CENTER);
    if (g_uninstall) {
      text(L"Your settings and downloaded speech models were kept.", f_body, C_TEXT2, 0, (int)S(228), W, (int)S(22), DT_SINGLELINE | DT_CENTER);
      text(L"Thanks for trying KeyReed.", f_body, C_TEXT2, 0, (int)S(252), W, (int)S(22), DT_SINGLELINE | DT_CENTER);
    } else {
      text(L"Hold Right Ctrl in any app, speak, and let go.", f_body, C_TEXT2, 0, (int)S(228), W, (int)S(22), DT_SINGLELINE | DT_CENTER);
      text(L"The first launch walks you through a short setup.", f_body, C_TEXT2, 0, (int)S(252), W, (int)S(22), DT_SINGLELINE | DT_CENTER);
    }
    rect(0, foot_y(), W, H - foot_y(), C_FOOT);
    rect(0, foot_y(), W, 1, C_LINE);
  }
  if (!g_round) { rect(0, 0, W, 1, C_LINE); rect(0, H - 1, W, 1, C_LINE); rect(0, 0, 1, H, C_LINE); rect(W - 1, 0, 1, H, C_LINE); }
}

/* "Voice becomes text": bars left of the real progress point settle into words. */
static void paint_band(double f, double t) {
  double bx0 = S(70), bw_all = S(CW - 140), bandH = S(78), cy = S(236);
  double pitch = bw_all / BAND_N, bw = pitch * .5;
  double px = bx0 + f * bw_all;
  int failed = g_stage == S_FAILED;
  double th = S(7);
  u32 ink = failed ? mix(C_WAVE_LO, C_ROSE, .55) : C_INK;
  char whole[BAND_N] = {0};
  /* Words whose every bar has fully turned into text are one pill, like a line of text. */
  for (int a = 0; a < BAND_N;) {
    int b = a;
    while (b < BAND_N - 1 && !g_word_end[b]) b++;
    double xa = bx0 + (a + .5) * pitch - bw / 2, xb = bx0 + (b + .5) * pitch + bw / 2;
    if (px - (bx0 + (b + .5) * pitch) >= pitch * 5) {
      rrect(xa, cy - th / 2, xb - xa, th, th / 2, ink, 1);
      for (int i = a; i <= b; i++) whole[i] = 1;
    }
    a = b + 1;
  }
  for (int i = 0; i < BAND_N; i++) {
    if (whole[i]) continue;
    double cx = bx0 + (i + .5) * pitch;
    /* Every term repeats an integer number of times per loop, so the loop has no seam. */
    double v = .5 * sin(2 * PI * (.11 * i - t)) + .3 * sin(2 * PI * (.23 * i - 2 * t + .3)) + .2 * sin(2 * PI * (.37 * i + t));
    double env = .45 + .55 * sin(PI * (i + .5) / BAND_N);
    double amp = env * (.5 + .5 * v);
    double hw = bandH * (.10 + .90 * amp);
    double k = clamp01((px - cx) / (pitch * 5)); /* 0 = sound, 1 = text */
    k = k * k * (3 - 2 * k);
    double h = hw + (th - hw) * k;
    double w = bw + ((g_word_end[i] ? bw : pitch) - bw) * k;
    double x = cx - bw / 2;
    u32 wave = mix(C_WAVE_LO, C_EM, .25 + .75 * amp);
    u32 col = failed ? mix(C_WAVE_LO, C_ROSE, .35 * k + .2) : mix(wave, ink, k);
    if (h < S(4)) h = S(4);
    rrect(x, cy - h / 2, w, h, S(3.5) < h / 2 ? S(3.5) : h / 2, col, 1);
  }
  /* Caret at the real progress point while installing. */
  if (!failed && f > 0 && f < 1) rrect(px - S(1), cy - S(14), S(2), S(28), S(1), C_MINT, 1);
}

static void paint_dynamic(void) {
  if (g_stage == S_PROGRESS || g_stage == S_DONE || g_stage == S_FAILED) {
    double t = (g_animate && g_stage == S_PROGRESS) ? fmod((double)(GetTickCount() - g_t0), LOOP_MS) / LOOP_MS : .35;
    double f = progress();
    paint_band(f, t);
    WCHAR pct[16];
    if (g_stage == S_FAILED) {
      WCHAR st[512] = {0};
      if (g_status) GetWindowTextW(g_status, st, 512);
      text(st, f_label, C_ROSE, (int)S(70), (int)S(296), (int)S(CW - 140), (int)S(44), DT_WORDBREAK | DT_CENTER | DT_END_ELLIPSIS);
    } else {
      /* Real progress from the installer's own bar; the tips below are not a status. */
      wsprintfW(pct, L"%d%%", (int)(f * 100));
      text(pct, f_btn, C_TEXT, 0, (int)S(290), T->w, (int)S(22), DT_SINGLELINE | DT_CENTER);
      if (!g_uninstall) {
        double el = (double)(GetTickCount() - g_t0);
        int i = (int)(el / TIP_MS) % N_TIPS;
        double ph = fmod(el, TIP_MS), a = 1;
        if (g_animate) a = clamp01(ph / 300.0) * clamp01((TIP_MS - ph) / 300.0);
        text(TIPS[i], f_body, mix(C_BG, C_TEXT2, a), (int)S(90), (int)S(326), (int)S(CW - 180), (int)S(44), DT_WORDBREAK | DT_CENTER);
      }
    }
  } else if (g_stage == S_FINISH) {
    double p = 1;
    if (g_animate) {
      p = clamp01((GetTickCount() - g_finish_t0) / 700.0);
      p = 1 - (1 - p) * (1 - p) * (1 - p);
    }
    arc(S(CW / 2), S(92), S(50), S(3), p, C_EM);
  }
}

static void paint(HDC wdc) {
  RECT r;
  GetClientRect(g_ov, &r);
  if (r.right <= 0 || r.bottom <= 0) return;
  if (!g_main.px || g_main.w != r.right || g_main.h != r.bottom) {
    surf_free(&g_main);
    if (!surf_make(&g_main, wdc, r.right, r.bottom)) return;
    if (g_cache) HeapFree(GetProcessHeap(), 0, g_cache);
    g_cache = (u32 *)HeapAlloc(GetProcessHeap(), 0, (size_t)r.right * r.bottom * 4);
    g_cache_ok = 0;
  }
  T = &g_main;
  size_t n = (size_t)g_main.w * g_main.h * 4;
  if (!g_cache_ok || !g_cache) {
    paint_static();
    if (g_cache) { CopyMemory(g_cache, g_main.px, n); g_cache_ok = 1; }
  } else {
    CopyMemory(g_main.px, g_cache, n);
  }
  paint_dynamic();
  BitBlt(wdc, 0, 0, g_main.w, g_main.h, g_main.dc, 0, 0, SRCCOPY);
}

/* ---------------- buttons (owner-drawn, real controls: Tab, Space, Enter, screen readers) ---------------- */
static WNDPROC g_btn_proc;
static LRESULT CALLBACK btn_wndproc(HWND h, UINT m, WPARAM w, LPARAM l) {
  switch (m) {
    case WM_MOUSEMOVE:
      if (g_hot != h) {
        HWND old = g_hot;
        g_hot = h;
        TRACKMOUSEEVENT tme = {sizeof(tme), TME_LEAVE, h, 0};
        TrackMouseEvent(&tme);
        InvalidateRect(h, NULL, FALSE);
        if (old) InvalidateRect(old, NULL, FALSE);
      }
      break;
    case WM_MOUSELEAVE:
      if (g_hot == h) { g_hot = NULL; InvalidateRect(h, NULL, FALSE); }
      break;
    case WM_ERASEBKGND:
      return 1;
    case WM_GETDLGCODE:
      /* Let Enter reach this button instead of the wizard's (hidden) default button. */
      if (l && ((MSG *)l)->message == WM_KEYDOWN && ((MSG *)l)->wParam == VK_RETURN) return DLGC_WANTMESSAGE | DLGC_BUTTON;
      break;
    case WM_KEYDOWN:
      if (w == VK_RETURN) {
        SendMessageW(GetParent(h), WM_COMMAND, MAKEWPARAM(GetDlgCtrlID(h), BN_CLICKED), (LPARAM)h);
        return 0;
      }
      break;
    case WM_SETFOCUS:
    case WM_KILLFOCUS:
      InvalidateRect(h, NULL, FALSE);
      break;
  }
  return CallWindowProcW(g_btn_proc, h, m, w, l);
}

static void opt_label(int id, WCHAR *out) {
  int on = g_stage == S_FINISH ? g_opt_launch : g_opt_desktop;
  (void)id;
  wsprintfW(out, L"%s: %s", g_stage == S_FINISH ? L"Launch KeyReed now" : L"Create a desktop shortcut", on ? L"On" : L"Off");
}

static HWND button(int id, const WCHAR *label, double x, double y, double w, double h) {
  HWND b = CreateWindowExW(0, L"BUTTON", label, WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW,
                           (int)S(x), (int)S(y), (int)S(w), (int)S(h), g_ov, (HMENU)(INT_PTR)id, g_inst, NULL);
  if (!b) return NULL;
  WNDPROC old = (WNDPROC)SetWindowLongPtrW(b, GWLP_WNDPROC, (LONG_PTR)btn_wndproc);
  if (!g_btn_proc) g_btn_proc = old;
  return b;
}

static void draw_button(DRAWITEMSTRUCT *di) {
  int w = di->rcItem.right - di->rcItem.left, h = di->rcItem.bottom - di->rcItem.top;
  Surf s = {0};
  if (!surf_make(&s, di->hDC, w, h)) return;
  Surf *prev = T;
  T = &s;
  int id = (int)di->CtlID;
  POINT o = {0, 0};
  MapWindowPoints(di->hwndItem, g_ov, &o, 1);
  /* Background: copy what the window painted under the button (card or footer). */
  if (g_cache && g_cache_ok) {
    for (int y = 0; y < h; y++)
      for (int x = 0; x < w; x++) {
        int sx = o.x + x, sy = o.y + y;
        s.px[y * w + x] = (sx >= 0 && sy >= 0 && sx < g_main.w && sy < g_main.h) ? g_cache[sy * g_main.w + sx] : C_BG;
      }
  } else {
    fill(C_BG);
  }
  int hot = g_hot == di->hwndItem, pressed = (di->itemState & ODS_SELECTED) != 0, focus = (di->itemState & ODS_FOCUS) != 0;
  WCHAR label[128];
  GetWindowTextW(di->hwndItem, label, 128);
  double r = S(8);
  if (id == ID_PRIMARY) {
    rrect(1, 1, w - 2, h - 2, r, pressed ? C_EM_PRESS : hot ? C_EM_HOT : C_EM, 1);
    text(label, f_btn, C_ON_EM, 0, 0, w, h, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
  } else if (id == ID_SECONDARY) {
    if (hot || pressed) rrect(1, 1, w - 2, h - 2, r, pressed ? rgb(0x22, 0x32, 0x2A) : rgb(0x17, 0x23, 0x1D), 1);
    rrect_stroke(1, 1, w - 2, h - 2, r, 1, rgb(0x5B, 0x70, 0x65));
    text(label, f_btn, C_TEXT, 0, 0, w, h, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
  } else if (id == ID_CHANGE) {
    if (hot || pressed) rrect(1, 1, w - 2, h - 2, r, rgb(0x1C, 0x2B, 0x24), 1);
    text(label, f_btn, C_MINT, 0, 0, w, h, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
  } else if (id == ID_CLOSE) {
    if (hot || pressed) fill(pressed ? rgb(0x9B, 0x22, 0x18) : rgb(0xC4, 0x2B, 0x1C));
    text(L"\xE8BB", f_icon, hot || pressed ? rgb(255, 255, 255) : C_TEXT2, 0, 0, w, h, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
    r = 0;
  } else if (id == ID_OPT) {
    int on = g_stage == S_FINISH ? g_opt_launch : g_opt_desktop;
    const WCHAR *name = g_stage == S_FINISH ? L"Launch KeyReed now" : L"Create a desktop shortcut";
    double sw = S(40), sh = S(22), sx = w - sw - S(2), sy = (h - sh) / 2;
    int lx = g_stage == S_FINISH ? (int)(sw + S(14)) : 0;
    if (g_stage == S_FINISH) sx = S(2);
    text(name, f_label, C_TEXT, lx, 0, (int)(w - sw - S(16)), h, DT_SINGLELINE | DT_VCENTER);
    if (on) {
      rrect(sx, sy, sw, sh, sh / 2, hot ? C_EM_HOT : C_EM, 1);
      rrect(sx + sw - sh + S(4), sy + S(4), sh - S(8), sh - S(8), (sh - S(8)) / 2, C_ON_EM, 1);
    } else {
      rrect_stroke(sx, sy, sw, sh, sh / 2, 1.5 * g_s, hot ? C_TEXT : C_TEXT3);
      rrect(sx + S(5), sy + S(5), sh - S(10), sh - S(10), (sh - S(10)) / 2, hot ? C_TEXT : C_TEXT3, 1);
    }
  }
  if (focus) rrect_stroke(0, 0, w, h, r + 1, 2 * g_s, C_TEXT);
  BitBlt(di->hDC, di->rcItem.left, di->rcItem.top, w, h, s.dc, 0, 0, SRCCOPY);
  T = prev;
  surf_free(&s);
}

/* ---------------- stage management ---------------- */
static BOOL CALLBACK kill_child(HWND h, LPARAM l) {
  (void)l;
  if (GetParent(h) == g_ov) DestroyWindow(h);
  return TRUE;
}
/* Hide the wizard's own controls (they keep working underneath) and keep ours on top. */
static void cover(void) {
  if (!g_parent || !g_ov) return;
  for (HWND c = GetWindow(g_parent, GW_CHILD); c; c = GetWindow(c, GW_HWNDNEXT))
    if (c != g_ov && IsWindowVisible(c)) ShowWindow(c, SW_HIDE);
  if (GetWindow(g_parent, GW_CHILD) != g_ov) SetWindowPos(g_ov, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
}
/* 30 fps only while something moves; 10 fps for progress text with reduced motion; 4 Hz otherwise
   (that slow tick only keeps the wizard's own controls hidden). */
static void retimer(void) {
  int ms = 250;
  if (g_stage == S_PROGRESS) ms = g_animate ? 33 : 100;
  else if (g_stage == S_FINISH && g_animate && GetTickCount() - g_finish_t0 < 900) ms = 33;
  SetTimer(g_ov, 1, ms, NULL);
}

static void choose_folder(void) {
  HRESULT hrInit = CoInitializeEx(NULL, COINIT_APARTMENTTHREADED);
  IFileOpenDialog *d = NULL;
  if (SUCCEEDED(CoCreateInstance(&CLSID_FileOpenDialog, NULL, CLSCTX_INPROC_SERVER, &IID_IFileOpenDialog, (void **)&d))) {
    DWORD opt = 0;
    IFileOpenDialog_GetOptions(d, &opt);
    IFileOpenDialog_SetOptions(d, opt | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST);
    IFileOpenDialog_SetTitle(d, L"Choose where to install KeyReed");
    IFileOpenDialog_SetOkButtonLabel(d, L"Install here");
    if (SUCCEEDED(IFileOpenDialog_Show(d, g_parent))) {
      IShellItem *res = NULL;
      if (SUCCEEDED(IFileOpenDialog_GetResult(d, &res))) {
        WCHAR *path = NULL;
        if (SUCCEEDED(IShellItem_GetDisplayName(res, SIGDN_FILESYSPATH, &path)) && path) {
          WCHAR out[MAX_PATH * 2];
          lstrcpynW(out, path, MAX_PATH);
          int n = lstrlenW(out);
          const WCHAR *last = out;
          for (int i = 0; i < n; i++) if (out[i] == L'\\') last = out + i + 1;
          /* Always install into a KeyReed folder, never straight into the chosen folder:
             the uninstaller removes the whole install folder. */
          if (lstrcmpiW(last, L"KeyReed") != 0) {
            if (n > 0 && out[n - 1] != L'\\') lstrcatW(out, L"\\");
            lstrcatW(out, L"KeyReed");
          }
          if (g_vars) lstrcpynW(instdir(), out, g_strsz);
          CoTaskMemFree(path);
        }
        IShellItem_Release(res);
      }
    }
    IFileOpenDialog_Release(d);
  }
  if (SUCCEEDED(hrInit)) CoUninitialize();
  g_cache_ok = 0;
  InvalidateRect(g_ov, NULL, FALSE);
}

static void set_stage(int stage) {
  g_stage = stage;
  g_cache_ok = 0;
  g_hot = NULL;
  EnumChildWindows(g_ov, kill_child, 0);
  WCHAR lab[96];
  HWND focus = NULL;
  if (stage == S_WELCOME) {
    if (g_uninstall) {
      focus = button(ID_PRIMARY, L"Remove KeyReed", CW - 44 - 168, CH - 76 + 20, 168, 36);
      button(ID_SECONDARY, L"Cancel", CW - 44 - 168 - 12 - 96, CH - 76 + 20, 96, 36);
    } else {
      button(ID_CHANGE, L"Change\x2026", 44 + 512 - 18 - 96, 240 + 18, 96, 32);
      opt_label(ID_OPT, lab);
      button(ID_OPT, lab, 44 + 18, 240 + 64, 512 - 36, 36);
      focus = button(ID_PRIMARY, L"Install KeyReed", CW - 44 - 168, CH - 76 + 20, 168, 36);
      button(ID_SECONDARY, L"Cancel", CW - 44 - 168 - 12 - 96, CH - 76 + 20, 96, 36);
    }
  } else if (stage == S_FAILED) {
    focus = button(ID_SECONDARY, L"Close", CW - 44 - 96, CH - 76 + 20, 96, 36);
  } else if (stage == S_FINISH) {
    if (!g_uninstall) {
      opt_label(ID_OPT, lab);
      button(ID_OPT, lab, 44, CH - 76 + 20, 260, 36);
    }
    focus = button(ID_PRIMARY, g_uninstall ? L"Close" : L"Finish", CW - 44 - 128, CH - 76 + 20, 128, 36);
    g_finish_t0 = GetTickCount();
  }
  if (stage == S_WELCOME || stage == S_FAILED || stage == S_FINISH) button(ID_CLOSE, L"Close", CW - 46, 0, 46, 32);
  WCHAR name[160];
  wsprintfW(name, L"KeyReed %s", g_uninstall ? L"uninstall" : L"setup");
  SetWindowTextW(g_ov, name);
  cover();
  retimer();
  InvalidateRect(g_ov, NULL, FALSE);
  if (focus) PostMessageW(g_ov, WM_KR_FOCUS, (WPARAM)focus, 0);
}

static LRESULT CALLBACK ov_wndproc(HWND h, UINT m, WPARAM w, LPARAM l) {
  switch (m) {
    case WM_ERASEBKGND:
      return 1;
    case WM_PAINT: {
      PAINTSTRUCT ps;
      HDC dc = BeginPaint(h, &ps);
      paint(dc);
      EndPaint(h, &ps);
      return 0;
    }
    case WM_PRINTCLIENT:
      paint((HDC)w);
      return 0;
    case WM_TIMER:
      cover();
      if (g_stage == S_PROGRESS || (g_stage == S_FINISH && GetTickCount() - g_finish_t0 < 900)) {
        RECT r;
        GetClientRect(h, &r);
        InvalidateRect(h, &r, FALSE);
      }
      retimer();
      return 0;
    case WM_LBUTTONDOWN:
      ReleaseCapture();
      SendMessageW(g_parent, WM_NCLBUTTONDOWN, HTCAPTION, 0);
      return 0;
    case WM_KR_FOCUS:
      if (IsWindow((HWND)w)) SetFocus((HWND)w);
      return 0;
    case WM_DRAWITEM:
      draw_button((DRAWITEMSTRUCT *)l);
      return TRUE;
    case WM_COMMAND: {
      int id = LOWORD(w);
      if (HIWORD(w) != BN_CLICKED && HIWORD(w) != BN_DOUBLECLICKED) break;
      if (id == ID_PRIMARY) PostMessageW(g_parent, WM_COMMAND, IDOK, 0);
      else if (id == ID_SECONDARY) PostMessageW(g_parent, WM_COMMAND, IDCANCEL, 0);
      else if (id == ID_CHANGE) choose_folder();
      else if (id == ID_CLOSE) {
        /* Finish page: closing means done, without launching. Elsewhere it is Cancel (with the usual confirmation). */
        if (g_stage == S_FINISH) { g_opt_launch = 0; PostMessageW(g_parent, WM_COMMAND, IDOK, 0); }
        else PostMessageW(g_parent, WM_COMMAND, IDCANCEL, 0);
      }
      else if (id == ID_OPT) {
        if (g_stage == S_FINISH) g_opt_launch = !g_opt_launch;
        else g_opt_desktop = !g_opt_desktop;
        WCHAR lab[96];
        opt_label(id, lab);
        SetWindowTextW((HWND)l, lab);
        InvalidateRect((HWND)l, NULL, FALSE);
      }
      return 0;
    }
    case WM_DESTROY:
      KillTimer(h, 1);
      g_ov = NULL;
      return 0;
  }
  return DefWindowProcW(h, m, w, l);
}

static UINT_PTR __cdecl unload_cb(enum NSPIM msg) {
  if (msg == NSPIM_UNLOAD) {
    surf_free(&g_main);
    if (g_cache) { HeapFree(GetProcessHeap(), 0, g_cache); g_cache = NULL; }
    HFONT *fs[] = {&f_title, &f_h2, &f_tag, &f_body, &f_small, &f_btn, &f_label, &f_icon};
    for (int i = 0; i < 8; i++) if (*fs[i]) { DeleteObject(*fs[i]); *fs[i] = NULL; }
  }
  return 0;
}

/* Size the wizard to the KeyReed layout, darken its caption, create the covering window. */
static void ensure(HWND parent, int string_size, WCHAR *variables, extra_parameters *extra) {
  g_vars = variables;
  g_strsz = string_size;
  if (g_ov && g_parent == parent) return;
  g_parent = parent;
  extra->RegisterPluginCallback(g_inst, unload_cb); /* stay loaded: our window procedures live here */
  HDC sdc = GetDC(NULL);
  g_s = GetDeviceCaps(sdc, LOGPIXELSX) / 96.0;
  ReleaseDC(NULL, sdc);
  if (g_s < 1) g_s = 1;
  BOOL anim = TRUE;
  SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &anim, 0);
  HIGHCONTRASTW hc = {sizeof(hc)};
  SystemParametersInfoW(SPI_GETHIGHCONTRAST, sizeof(hc), &hc, 0);
  g_hc = (hc.dwFlags & HCF_HIGHCONTRASTON) != 0;
  g_animate = anim && !g_hc;
  fonts();
  words();

  /* Frameless window: no title bar; KeyReed draws its own close button and the window drags from anywhere. */
  SetWindowLongW(parent, GWL_STYLE, (LONG)((DWORD)GetWindowLongW(parent, GWL_STYLE) & ~(WS_CAPTION | WS_THICKFRAME | WS_MAXIMIZEBOX)));
  SetWindowLongW(parent, GWL_EXSTYLE, (LONG)((DWORD)GetWindowLongW(parent, GWL_EXSTYLE) & ~(WS_EX_DLGMODALFRAME | WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE)));
  RECT rc = {0, 0, (int)S(CW), (int)S(CH)};
  DWORD st = (DWORD)GetWindowLongW(parent, GWL_STYLE), ex = (DWORD)GetWindowLongW(parent, GWL_EXSTYLE);
  AdjustWindowRectEx(&rc, st, FALSE, ex);
  int ww = rc.right - rc.left, wh = rc.bottom - rc.top;
  MONITORINFO mi = {sizeof(mi)};
  GetMonitorInfoW(MonitorFromWindow(parent, MONITOR_DEFAULTTONEAREST), &mi);
  int x = mi.rcWork.left + (mi.rcWork.right - mi.rcWork.left - ww) / 2;
  int y = mi.rcWork.top + (mi.rcWork.bottom - mi.rcWork.top - wh) / 2;
  SetWindowPos(parent, NULL, x, y, ww, wh, SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);

  /* Windows 11 dark caption in the KeyReed colour (ignored on older builds). */
  HMODULE dwm = LoadLibraryW(L"dwmapi.dll");
  if (dwm) {
    typedef HRESULT(WINAPI * SetAttr)(HWND, DWORD, LPCVOID, DWORD);
    SetAttr set = (SetAttr)(void *)GetProcAddress(dwm, "DwmSetWindowAttribute");
    if (set) {
      BOOL dark = TRUE;
      COLORREF cap = RGB(0x0C, 0x14, 0x11), txt = RGB(0xE8, 0xF1, 0xEC), border = RGB(0x23, 0x33, 0x2B);
      set(parent, 20, &dark, sizeof(dark));
      set(parent, 35, &cap, sizeof(cap));
      set(parent, 36, &txt, sizeof(txt));
      set(parent, 34, &border, sizeof(border));
      int corner = 2; /* DWMWCP_ROUND: Windows 11 rounds and borders the window; older Windows gets our drawn border */
      g_round = SUCCEEDED(set(parent, 33, &corner, sizeof(corner)));
    }
  }

  WNDCLASSW wc = {0};
  wc.lpfnWndProc = ov_wndproc;
  wc.hInstance = g_inst;
  wc.lpszClassName = CLS;
  wc.hCursor = LoadCursorW(NULL, (LPCWSTR)IDC_ARROW);
  RegisterClassW(&wc);
  GetClientRect(parent, &rc);
  g_ov = CreateWindowExW(WS_EX_CONTROLPARENT, CLS, L"KeyReed setup", WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
                         0, 0, rc.right, rc.bottom, parent, NULL, g_inst, NULL);
  cover();
}

static void mode(void *stacktop) {
  WCHAR m[32];
  pop(stacktop, m, 32);
  g_uninstall = lstrcmpiW(m, L"uninstall") == 0;
}

/* KeyReedSetup::Init "install|uninstall"  (from .onGUIInit: no grey wizard is ever shown) */
__declspec(dllexport) void Init(HWND parent, int string_size, WCHAR *variables, void *stacktop, extra_parameters *extra) {
  mode(stacktop);
  ensure(parent, string_size, variables, extra);
}

/* KeyReedSetup::Welcome "install|uninstall" "<version>" */
__declspec(dllexport) void Welcome(HWND parent, int string_size, WCHAR *variables, void *stacktop, extra_parameters *extra) {
  mode(stacktop);
  pop(stacktop, g_version, 32);
  ensure(parent, string_size, variables, extra);
  set_stage(S_WELCOME);
}

/* KeyReedSetup::Progress "install|uninstall"  (from the instfiles page's show function) */
__declspec(dllexport) void Progress(HWND parent, int string_size, WCHAR *variables, void *stacktop, extra_parameters *extra) {
  mode(stacktop);
  ensure(parent, string_size, variables, extra);
  /* The instfiles page dialog is the #32770 child that owns the progress bar. */
  HWND dlg = NULL;
  while ((dlg = FindWindowExW(parent, dlg, L"#32770", NULL)) != NULL)
    if (GetDlgItem(dlg, 1004)) break;
  g_bar = dlg ? GetDlgItem(dlg, 1004) : NULL;
  g_status = dlg ? GetDlgItem(dlg, 1006) : NULL;
  g_t0 = GetTickCount();
  set_stage(S_PROGRESS);
}

__declspec(dllexport) void Success(HWND parent, int string_size, WCHAR *variables, void *stacktop, extra_parameters *extra) {
  (void)parent; (void)string_size; (void)variables; (void)stacktop; (void)extra;
  if (g_ov) set_stage(S_DONE);
}

__declspec(dllexport) void Failed(HWND parent, int string_size, WCHAR *variables, void *stacktop, extra_parameters *extra) {
  (void)parent; (void)string_size; (void)variables; (void)stacktop; (void)extra;
  if (g_ov) set_stage(S_FAILED);
}

/* KeyReedSetup::Finish "install|uninstall" */
__declspec(dllexport) void Finish(HWND parent, int string_size, WCHAR *variables, void *stacktop, extra_parameters *extra) {
  mode(stacktop);
  ensure(parent, string_size, variables, extra);
  set_stage(S_FINISH);
}

/* KeyReedSetup::Option "desktop|launch"  -> pushes "1" or "0" */
__declspec(dllexport) void Option(HWND parent, int string_size, WCHAR *variables, void *stacktop, extra_parameters *extra) {
  (void)parent; (void)variables; (void)extra;
  g_strsz = string_size;
  WCHAR name[32];
  pop(stacktop, name, 32);
  int v = lstrcmpiW(name, L"launch") == 0 ? g_opt_launch : g_opt_desktop;
  push(stacktop, v ? L"1" : L"0");
}

BOOL WINAPI DllMain(HINSTANCE inst, DWORD reason, LPVOID reserved) {
  (void)reserved;
  if (reason == DLL_PROCESS_ATTACH) g_inst = inst;
  return TRUE;
}
