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
 * Frames come from a 60 Hz clock thread on a high-resolution waitable timer (WM_TIMER is quantised to the
 * 15.6 ms system tick and gave ~20 fps). Only the rows that change are repainted: the background (glow, footer)
 * is cached, the waveform band repaints each frame, the text rows only when their text changes, and nothing
 * repaints on a settled screen. The progress shown eases toward the installer's real progress and never runs
 * ahead of it. Screens crossfade; a page's groups rise in a stagger; hover and switches are tweened.
 * Reduced motion ("Animate controls" off) or high contrast: no clock, no transitions, static bars.
 *
 * Build (32-bit unicode NSIS plugin): see scripts/build-installer.sh
 */
#define COBJMACROS
#include <windows.h>
#include <shobjidl.h>
#include <tlhelp32.h>
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
static HFONT f_title, f_h2, f_tag, f_body, f_small, f_btn, f_label, f_icon;
static char g_word_end[BAND_N];

#define WM_KR_FRAME (WM_APP + 2)
#define ENTER_MS 900.0         /* a page's entrance: its groups rise and fade in a stagger, buttons last */
#define TR_MS 240.0            /* crossfade from the previous screen */
#define PTAU 140.0             /* the progress shown eases to the real one with this time constant (ms) */
#define HOVER_TAU 70.0
#define MIN_PROGRESS_MS 1100.0 /* the progress screen is never a flash: an instant uninstall still shows it this long */
static double g_stage_ms, g_prog_ms, g_clk_last, g_pdisp, g_tr_t0, g_sw, g_lbl_t0;
static double g_hv[5];         /* eased hover of each control, ids 2001..2005 */
static int g_finishing, g_tr_on, g_tr_ok, g_clock_ok, g_last_pct = -1, g_last_tip_q = -1;
static u32 *g_prev;            /* the previous screen, for the crossfade */
static volatile LONG g_pending, g_clock_on;
static HANDLE g_wake;
static WCHAR g_lbl[160], g_lbl_next[160];
/* Entrance state applied by the primitives: rise (px), opacity, and the rows being repainted. */
static double G_DY, G_A = 1;
static int G_Y0, G_Y1 = 1 << 30;

/* Render target (window DIB or a button DIB). */
typedef struct { u32 *px; int w, h; HDC dc; HBITMAP bmp; } Surf;
static Surf g_main;
static u32 *g_cache; /* static layer of the window */
static int g_cache_ok;
static Surf *T;

/* Opt-in frame log: set KEYREED_SETUP_PERF=<file> and every painted frame's start time and cost is written there
   when the setup exits (ms; stage). Costs nothing otherwise. */
#define PERF_MAX 8192
static struct { float at, cost, ph[3]; char stage; } g_perf[PERF_MAX];
static double g_ph[3]; /* time spent in: copy+content+dynamic, blend, BitBlt (ms), of the frame being painted */
static int g_perf_n, g_perf_on = -1;
static LARGE_INTEGER g_qpf, g_q0;
static double qpc_ms(void) { LARGE_INTEGER n; QueryPerformanceCounter(&n); return (double)(n.QuadPart - g_q0.QuadPart) * 1000.0 / (double)g_qpf.QuadPart; }
static void perf_init(void) {
  WCHAR f[8];
  g_perf_on = GetEnvironmentVariableW(L"KEYREED_SETUP_PERF", f, 8) > 0;
  QueryPerformanceFrequency(&g_qpf); QueryPerformanceCounter(&g_q0);
}
static void perf_frame(double at, double cost) {
  if (g_perf_on && g_perf_n < PERF_MAX) { g_perf[g_perf_n].at = (float)at; g_perf[g_perf_n].cost = (float)cost; for (int k = 0; k < 3; k++) g_perf[g_perf_n].ph[k] = (float)g_ph[k]; g_perf[g_perf_n++].stage = (char)g_stage; }
}
static void perf_dump(void) {
  WCHAR path[MAX_PATH];
  if (!g_perf_on || !g_perf_n || !GetEnvironmentVariableW(L"KEYREED_SETUP_PERF", path, MAX_PATH)) return;
  HANDLE h = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
  if (h == INVALID_HANDLE_VALUE) return;
  char line[64]; DWORD wr;
  for (int i = 0; i < g_perf_n; i++) {
    int n = wsprintfA(line, "%d %d %d %d %d %d\n", (int)(g_perf[i].at * 1000), (int)(g_perf[i].cost * 1000), g_perf[i].stage,
                      (int)(g_perf[i].ph[0] * 1000), (int)(g_perf[i].ph[1] * 1000), (int)(g_perf[i].ph[2] * 1000));
    WriteFile(h, line, n, &wr, NULL);
  }
  CloseHandle(h);
}

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
/* The surface colour at a point: what a faded element fades toward. */
static u32 under(int x, int y) {
  if (x < 0) x = 0;
  if (y < 0) y = 0;
  if (x >= T->w) x = T->w - 1;
  if (y >= T->h) y = T->h - 1;
  return T->px[y * T->w + x];
}
static int row_ok(int y) { return y >= 0 && y < T->h && y >= G_Y0 && y < G_Y1; }
static void rrect(double x, double y, double w, double h, double r, u32 c, double a) {
  if (w <= 0 || h <= 0) return;
  y += G_DY;
  a *= G_A;
  if (r > w / 2) r = w / 2;
  if (r > h / 2) r = h / 2;
  double cx = x + w / 2, cy = y + h / 2;
  for (int yy = (int)floor(y) - 1; yy <= (int)ceil(y + h) + 1; yy++) {
    if (!row_ok(yy)) continue;
    for (int xx = (int)floor(x) - 1; xx <= (int)ceil(x + w) + 1; xx++) {
      if (xx < 0 || xx >= T->w) continue;
      double cov = clamp01(.5 - sd_rrect(xx + .5, yy + .5, cx, cy, w / 2, h / 2, r)) * a;
      if (cov > 0) T->px[yy * T->w + xx] = mix(T->px[yy * T->w + xx], c, cov);
    }
  }
}
static void rrect_stroke(double x, double y, double w, double h, double r, double sw, u32 c) {
  y += G_DY;
  if (G_A < 1) c = mix(C_BG, c, G_A);
  double cx = x + w / 2, cy = y + h / 2;
  for (int yy = (int)floor(y) - 1; yy <= (int)ceil(y + h) + 1; yy++) {
    if (!row_ok(yy)) continue;
    for (int xx = (int)floor(x) - 1; xx <= (int)ceil(x + w) + 1; xx++) {
      if (xx < 0 || xx >= T->w) continue;
      double d = sd_rrect(xx + .5, yy + .5, cx, cy, w / 2, h / 2, r);
      double cov = clamp01(.5 - d) * clamp01(d + sw + .5);
      if (cov > 0) T->px[yy * T->w + xx] = mix(T->px[yy * T->w + xx], c, cov);
    }
  }
}
/* Soft radial light: one restrained glow, not a gradient background. */
static void glow(double cx, double cy, double rad, u32 c, double amax) {
  for (int yy = (int)(cy - rad); yy < (int)(cy + rad); yy++) {
    if (!row_ok(yy)) continue;
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
    if (!row_ok(yy)) continue;
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
  y += (int)G_DY;
  if (y + h <= G_Y0 || y >= G_Y1) return; /* outside the rows being repainted */
  if (G_A < 1) c = mix(under(x + 2, y + h / 2), c, G_A);
  RECT rc = {x, y, x + w, y + h};
  SelectObject(T->dc, f);
  SetTextColor(T->dc, RGB((c >> 16) & 255, (c >> 8) & 255, c & 255));
  DrawTextW(T->dc, s, -1, &rc, fmt | DT_NOPREFIX);
}
/* The official icon embedded in this setup/uninstall EXE (NSIS icon resource 103), at size n. It settles in
   from 88% of its size while it rises. */
static void logo(int x, int y, int n) {
  int m = (int)(n * (.88 + .12 * G_A));
  x += (n - m) / 2;
  y += (n - m) / 2 + (int)G_DY;
  if (y + m <= G_Y0 || y >= G_Y1) return;
  HICON ic = (HICON)LoadImageW(GetModuleHandleW(NULL), MAKEINTRESOURCEW(103), IMAGE_ICON, m, m, LR_DEFAULTCOLOR);
  if (!ic) return;
  DrawIconEx(T->dc, x, y, ic, m, m, 0, NULL, DI_NORMAL);
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
  if (g_stage == S_DONE || g_finishing) return 1;
  if (!g_bar) return 0;
  LRESULT hi = SendMessageW(g_bar, 0x0407 /* PBM_GETRANGE */, FALSE, 0);
  LRESULT pos = SendMessageW(g_bar, 0x0408 /* PBM_GETPOS */, 0, 0);
  return hi > 0 ? clamp01((double)pos / (double)hi) : 0;
}

/* Page entrance: group k rises and fades in, staggered. Called before drawing each group. */
static void grp(int k) {
  if (!g_animate) { G_DY = 0; G_A = 1; return; }
  double p = clamp01((qpc_ms() - g_stage_ms - 40 - k * 55) / 460);
  p = 1 - (1 - p) * (1 - p) * (1 - p);
  G_A = p;
  G_DY = (1 - p) * S(14);
}
static void grp_off(void) { G_DY = 0; G_A = 1; }

/* The cached layer: background, glow, footer, border. It never moves. */
static void paint_bg(void) {
  int W = T->w, H = T->h;
  fill(C_BG);
  if (g_stage == S_WELCOME) glow(S(84), S(70), S(260), rgb(0x10, 0xB9, 0x81), .10);
  else if (g_stage == S_FINISH) glow(S(CW / 2), S(92), S(260), rgb(0x10, 0xB9, 0x81), .11);
  else glow(S(CW / 2), S(78), S(240), rgb(0x10, 0xB9, 0x81), .09);
  if (g_stage == S_WELCOME || g_stage == S_FINISH || g_stage == S_FAILED) {
    rect(0, foot_y(), W, H - foot_y(), C_FOOT);
    rect(0, foot_y(), W, 1, C_LINE);
  }
  if (!g_round) { rect(0, 0, W, 1, C_LINE); rect(0, H - 1, W, 1, C_LINE); rect(0, 0, 1, H, C_LINE); rect(W - 1, 0, 1, H, C_LINE); }
}

/* Everything that sits on the background, drawn as staggered groups. Cheap: a few texts and shapes, and only the
   rows being repainted are touched. */
static void paint_content(void) {
  int W = T->w;
  WCHAR buf[600];
  if (g_stage == S_WELCOME) {
    grp(0);
    logo((int)S(44), (int)S(42), (int)S(64));
    grp(1);
    text(L"KeyReed", f_title, C_TEXT, (int)S(44), (int)S(122), (int)S(500), (int)S(46), DT_SINGLELINE);
    grp(2);
    text(g_uninstall ? L"Remove KeyReed from this PC?" : L"Voice it. Everywhere.", f_tag, C_MINT, (int)S(44), (int)S(168), (int)S(500), (int)S(26), DT_SINGLELINE);
    grp(3);
    text(g_uninstall ? L"The app, its Start menu entries and the desktop shortcut are removed." : L"Turn your speech into polished text, anywhere on Windows.",
         f_body, C_TEXT2, (int)S(44), (int)S(200), (int)S(512), (int)S(22), DT_SINGLELINE | DT_END_ELLIPSIS);
    grp(4);
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
      rrect(cx + S(18), cy + S(62), cw - S(36), 1, 0, C_LINE, 1);
    }
    grp(5);
    wsprintfW(buf, L"Version %s", g_version);
    text(buf, f_small, C_TEXT3, (int)S(44), foot_y() + (int)S(20), (int)S(200), (int)S(18), DT_SINGLELINE);
    text(g_uninstall ? L"Uninstall" : L"No administrator rights needed", f_small, C_TEXT3, (int)S(44), foot_y() + (int)S(38), (int)S(220), (int)S(18), DT_SINGLELINE | DT_END_ELLIPSIS);
  } else if (g_stage == S_PROGRESS || g_stage == S_DONE || g_stage == S_FAILED) {
    grp(0);
    logo((int)S(CW / 2 - 28), (int)S(50), (int)S(56));
    const WCHAR *title = g_stage == S_FAILED ? (g_uninstall ? L"KeyReed was not fully removed" : L"KeyReed was not installed")
                       : g_stage == S_DONE ? (g_uninstall ? L"KeyReed is removed" : L"KeyReed is installed")
                       : (g_uninstall ? L"Removing KeyReed" : L"Installing KeyReed");
    grp(1);
    text(title, f_h2, C_TEXT, 0, (int)S(116), W, (int)S(36), DT_SINGLELINE | DT_CENTER);
    grp(2);
    if (g_stage != S_FAILED)
      text(g_uninstall ? L"Your settings and speech models stay on this PC." : L"Voice it. Everywhere.", g_uninstall ? f_body : f_tag,
           g_uninstall ? C_TEXT2 : C_MINT, 0, (int)S(154), W, (int)S(24), DT_SINGLELINE | DT_CENTER);
  } else if (g_stage == S_FINISH) {
    grp(0);
    logo((int)S(CW / 2 - 36), (int)S(56), (int)S(72));
    grp(1);
    text(g_uninstall ? L"KeyReed has been removed." : L"KeyReed is ready.", f_title, C_TEXT, 0, (int)S(176), W, (int)S(46), DT_SINGLELINE | DT_CENTER);
    grp(2);
    if (g_uninstall) text(L"Your settings and downloaded speech models were kept.", f_body, C_TEXT2, 0, (int)S(228), W, (int)S(22), DT_SINGLELINE | DT_CENTER);
    else text(L"Hold Right Ctrl in any app, speak, and let go.", f_body, C_TEXT2, 0, (int)S(228), W, (int)S(22), DT_SINGLELINE | DT_CENTER);
    grp(3);
    text(g_uninstall ? L"Thanks for trying KeyReed." : L"The first launch walks you through a short setup.", f_body, C_TEXT2, 0, (int)S(252), W, (int)S(22), DT_SINGLELINE | DT_CENTER);
  }
  grp_off();
}

/* "Voice becomes text": bars left of the (eased) progress point settle into words. */
static void paint_band(double f, double t) {
  double bx0 = S(70), bw_all = S(CW - 140), bandH = S(78), cy = S(236);
  double pitch = bw_all / BAND_N, bw = pitch * .5;
  double px = bx0 + f * bw_all;
  int failed = g_stage == S_FAILED;
  double th = S(7);
  u32 ink = failed ? mix(C_WAVE_LO, C_ROSE, .55) : C_INK;
  char whole[BAND_N] = {0};
  /* A soft light rides the caret: the point where sound becomes text. */
  if (!failed && f > 0 && f < 1) glow(px, cy - G_DY, S(46), C_EM, .16);
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
  /* Caret at the progress point while installing. */
  if (!failed && f > 0 && f < 1) rrect(px - S(1), cy - S(14), S(2), S(28), S(1), C_MINT, 1);
}

static int pct_of(double f) { int p = (int)(f * 100 + .5); return p > 100 ? 100 : p; }
/* The tip's opacity: it fades in and out around each 4.2 s slot. */
static double tip_alpha(double el, int *idx) {
  *idx = (int)(el / TIP_MS) % N_TIPS;
  double ph = fmod(el, TIP_MS);
  return g_animate ? clamp01(ph / 300.0) * clamp01((TIP_MS - ph) / 300.0) : 1;
}
/* The installer's own status line ("Installing application files") crossfades when it changes. */
static void label_update(double t) {
  WCHAR cur[160] = {0};
  if (g_status) GetWindowTextW(g_status, cur, 160);
  if (cur[0] && lstrcmpW(cur, g_lbl_next)) { lstrcpynW(g_lbl_next, cur, 160); g_lbl_t0 = t; }
}
static double label_alpha(double t) {
  if (!g_animate || !g_lbl[0]) { lstrcpynW(g_lbl, g_lbl_next, 160); return 1; }
  double e = t - g_lbl_t0;
  if (e < 140) return lstrcmpW(g_lbl, g_lbl_next) ? 1 - e / 140 : 1;
  lstrcpynW(g_lbl, g_lbl_next, 160);
  return e < 280 ? (e - 140) / 140 : 1;
}

/* What moves: the band, the percentage, the stage label, the tip, the finish ring. */
static void paint_dynamic(void) {
  double t = qpc_ms();
  if (g_stage == S_PROGRESS || g_stage == S_DONE || g_stage == S_FAILED) {
    double ph = (g_animate && g_stage == S_PROGRESS) ? fmod(t - g_prog_ms, LOOP_MS) / LOOP_MS : .35;
    double f = g_stage == S_FAILED ? progress() : g_pdisp;
    grp(3);
    paint_band(f, ph);
    grp(4);
    if (g_stage == S_FAILED) {
      WCHAR st[512] = {0};
      if (g_status) GetWindowTextW(g_status, st, 512);
      text(st, f_label, C_ROSE, (int)S(70), (int)S(296), (int)S(CW - 140), (int)S(44), DT_WORDBREAK | DT_CENTER | DT_END_ELLIPSIS);
    } else {
      WCHAR pct[16];
      wsprintfW(pct, L"%d%%", pct_of(f));
      text(pct, f_btn, C_TEXT, 0, (int)S(288), T->w, (int)S(22), DT_SINGLELINE | DT_CENTER);
      if (g_stage == S_PROGRESS && g_lbl_next[0]) {
        double la = label_alpha(t);
        WCHAR lb[170];
        wsprintfW(lb, L"%s\x2026", g_lbl);
        text(lb, f_label, mix(C_BG, C_TEXT2, la), 0, (int)S(314), T->w, (int)S(22), DT_SINGLELINE | DT_CENTER | DT_END_ELLIPSIS);
      }
      if (g_stage == S_PROGRESS && !g_uninstall) {
        grp(5);
        int i;
        double a = tip_alpha(t - g_prog_ms, &i);
        text(TIPS[i], f_body, mix(C_BG, C_TEXT2, a), (int)S(90), (int)S(360), (int)S(CW - 180), (int)S(44), DT_WORDBREAK | DT_CENTER);
      }
    }
  } else if (g_stage == S_FINISH) {
    double ts = t - g_stage_ms, cx = S(CW / 2), cy = S(92);
    double p = g_animate ? clamp01(ts / 700.0) : 1;
    p = 1 - (1 - p) * (1 - p) * (1 - p);
    grp(0);
    arc(cx, cy + G_DY, S(50), S(3), p, C_EM);
    /* A ripple leaves the ring as it closes. */
    double q = g_animate ? clamp01((ts - 600) / 900.0) : 1;
    if (q > 0 && q < 1) {
      q = 1 - (1 - q) * (1 - q);
      arc(cx, cy + G_DY, S(50) + S(30) * q, S(2), 1, mix(C_BG, C_EM, (1 - q) * .5));
    }
  }
  grp_off();
}

/* Crossfade from the previous screen over rows y0..y1. */
static void blend(int y0, int y1) {
  double p = clamp01((qpc_ms() - g_tr_t0) / TR_MS);
  if (p >= 1 || !g_tr_ok) { g_tr_on = 0; return; }
  unsigned ia = (unsigned)(p * p * (3 - 2 * p) * 256), ib = 256 - ia;
  u32 *a = g_prev, *b = g_main.px;
  for (size_t i = (size_t)y0 * g_main.w, e = (size_t)y1 * g_main.w; i < e; i++) {
    u32 x = a[i], y = b[i];
    b[i] = ((((x & 0xFF00FF) * ib + (y & 0xFF00FF) * ia) >> 8) & 0xFF00FF) | ((((x & 0x00FF00) * ib + (y & 0x00FF00) * ia) >> 8) & 0x00FF00);
  }
}

/* Paint the rows y0..y1 of the dirty rectangle (all of them when `dirty` is NULL). */
static void paint(HDC wdc, const RECT *dirty) {
  double t_in = qpc_ms();
  RECT r;
  GetClientRect(g_ov, &r);
  if (r.right <= 0 || r.bottom <= 0) return;
  if (!g_main.px || g_main.w != r.right || g_main.h != r.bottom) {
    surf_free(&g_main);
    if (!surf_make(&g_main, wdc, r.right, r.bottom)) return;
    if (g_cache) HeapFree(GetProcessHeap(), 0, g_cache);
    if (g_prev) HeapFree(GetProcessHeap(), 0, g_prev);
    g_cache = (u32 *)HeapAlloc(GetProcessHeap(), 0, (size_t)r.right * r.bottom * 4);
    g_prev = (u32 *)HeapAlloc(GetProcessHeap(), 0, (size_t)r.right * r.bottom * 4);
    g_cache_ok = 0;
    g_tr_ok = 0;
  }
  int y0 = 0, y1 = r.bottom, W = g_main.w;
  if (dirty) { if (dirty->top > y0) y0 = dirty->top; if (dirty->bottom < y1) y1 = dirty->bottom; }
  if (y1 <= y0) return;
  T = &g_main;
  if (!g_cache_ok || !g_cache) {
    G_Y0 = 0; G_Y1 = 1 << 30;
    paint_bg();
    if (g_cache) { CopyMemory(g_cache, g_main.px, (size_t)W * g_main.h * 4); g_cache_ok = 1; }
  } else {
    CopyMemory(g_main.px + (size_t)y0 * W, g_cache + (size_t)y0 * W, (size_t)(y1 - y0) * W * 4);
  }
  G_Y0 = y0; G_Y1 = y1;
  paint_content();
  paint_dynamic();
  G_Y0 = 0; G_Y1 = 1 << 30;
  double t1 = qpc_ms();
  if (g_tr_on) blend(y0, y1);
  double t2 = qpc_ms();
  BitBlt(wdc, 0, y0, W, y1 - y0, g_main.dc, 0, y0, SRCCOPY);
  double t3 = qpc_ms();
  g_ph[0] = t1 - t_in; g_ph[1] = t2 - t1; g_ph[2] = t3 - t2;
  perf_frame(t_in, t3 - t_in);
}

static void cover(void);

/* ---------------- frame clock ---------------- */
/* A thread on a high-resolution waitable timer posts one WM_KR_FRAME at a time (60 Hz) while something moves and
   parks otherwise. The window decides to stop only after drawing a frame, from what that frame drew. */
static DWORD WINAPI clock_thread(LPVOID unused) {
  (void)unused;
  HANDLE t = CreateWaitableTimerExW(NULL, NULL, 0x2 /* CREATE_WAITABLE_TIMER_HIGH_RESOLUTION */, 0x1F0003);
  if (!t) t = CreateWaitableTimerW(NULL, FALSE, NULL); /* shortcut: ~15.6 ms steps before Windows 10 1803; fine, upgrade never */
  if (!t) { g_clock_ok = 0; return 0; }
  for (;;) {
    WaitForSingleObject(g_wake, INFINITE);
    LARGE_INTEGER due;
    due.QuadPart = -166667; /* 16.7 ms */
    SetWaitableTimer(t, &due, 16, NULL, NULL, FALSE);
    while (g_clock_on) {
      WaitForSingleObject(t, 100);
      if (g_ov && !InterlockedExchange(&g_pending, 1)) PostMessageW(g_ov, WM_KR_FRAME, 0, 0);
    }
    CancelWaitableTimer(t);
    ResetEvent(g_wake);
    if (g_clock_on) SetEvent(g_wake); /* started again between the loop and the reset */
  }
}
static void clock_start(void) {
  if (!g_clock_ok) return;
  if (!g_clock_on) g_clk_last = qpc_ms();
  InterlockedExchange(&g_clock_on, 1);
  SetEvent(g_wake);
}
/* Ask for a frame: the clock while animating, one message otherwise (reduced motion snaps instead of tweening). */
static void request_frame(void) {
  if (g_animate && g_clock_ok) clock_start();
  else if (g_ov) PostMessageW(g_ov, WM_KR_FRAME, 0, 0);
}

static double hv_target(int i) { HWND b = g_ov ? GetDlgItem(g_ov, ID_PRIMARY + i) : NULL; return b && b == g_hot; }
static double sw_target(void) { return g_stage == S_FINISH ? g_opt_launch : g_opt_desktop; }

/* Whether another frame is needed, judged from what was just drawn. */
static int animating(void) {
  if (!g_animate) return 0;
  double ts = qpc_ms() - g_stage_ms;
  if (g_stage == S_PROGRESS || g_tr_on || ts < ENTER_MS + 100) return 1;
  if (g_stage == S_DONE && g_pdisp < .999) return 1;
  if (g_stage == S_FINISH && ts < 1600) return 1;
  for (int i = 0; i < 5; i++) if (fabs(g_hv[i] - hv_target(i)) > .002) return 1;
  return fabs(g_sw - sw_target()) > .002;
}

/* One frame: ease the state, invalidate exactly the rows that changed, paint them now. */
static void tick(void) {
  if (!g_ov) return;
  cover();
  double t = qpc_ms(), dt = t - g_clk_last;
  if (dt < 0 || dt > 100) dt = 16.7;
  g_clk_last = t;
  double ts = t - g_stage_ms;
  RECT cr;
  GetClientRect(g_ov, &cr);
  int full = g_tr_on || (g_animate && ts < ENTER_MS), band = 0, txt = 0, ring = 0;
  if (g_stage == S_PROGRESS || g_stage == S_DONE) {
    double target = progress();
    g_pdisp = g_animate ? g_pdisp + (target - g_pdisp) * (1 - exp(-dt / PTAU)) : target;
    if (g_pdisp > target || target - g_pdisp < .0005) g_pdisp = target;
    label_update(t);
    int pc = pct_of(g_pdisp), ti;
    int tq = 0;
    if (g_stage == S_PROGRESS && !g_uninstall) { double ta = tip_alpha(t - g_prog_ms, &ti); tq = ti * 100 + (int)(ta * 50); }
    txt = pc != g_last_pct || tq != g_last_tip_q || (t - g_lbl_t0) < 300;
    g_last_pct = pc;
    g_last_tip_q = tq;
    band = g_animate && g_stage == S_PROGRESS;
  } else if (g_stage == S_FINISH && g_animate && ts < 1600) {
    ring = 1;
  }
  if (full) {
    InvalidateRect(g_ov, NULL, FALSE);
  } else {
    RECT r = {0, 0, cr.right, 0};
    if (band) { r.top = (int)S(190); r.bottom = (int)S(284); InvalidateRect(g_ov, &r, FALSE); }
    if (txt) { r.top = (int)S(284); r.bottom = (int)S(412); InvalidateRect(g_ov, &r, FALSE); }
    if (ring) { r.top = (int)S(8); r.bottom = (int)S(176); InvalidateRect(g_ov, &r, FALSE); }
  }
  UpdateWindow(g_ov);
  /* Controls are windows of their own: tween hover and the switch, and repaint those that changed. */
  double k = 1 - exp(-dt / HOVER_TAU);
  double sw_t = sw_target(), sw_before = g_sw;
  g_sw = g_animate ? g_sw + (sw_t - g_sw) * k : sw_t;
  if (fabs(g_sw - sw_t) < .004) g_sw = sw_t;
  for (int i = 0; i < 5; i++) {
    HWND b = GetDlgItem(g_ov, ID_PRIMARY + i);
    if (!b) continue;
    double tg = hv_target(i), before = g_hv[i];
    g_hv[i] = g_animate ? g_hv[i] + (tg - g_hv[i]) * k : tg;
    if (fabs(g_hv[i] - tg) < .004) g_hv[i] = tg;
    if (full || g_hv[i] != before || (ID_PRIMARY + i == ID_OPT && g_sw != sw_before)) { InvalidateRect(b, NULL, FALSE); UpdateWindow(b); }
  }
  if (!animating()) InterlockedExchange(&g_clock_on, 0);
}

/* ---------------- buttons (owner-drawn, real controls: Tab, Space, Enter, screen readers) ---------------- */
static WNDPROC g_btn_proc;
static LRESULT CALLBACK btn_wndproc(HWND h, UINT m, WPARAM w, LPARAM l) {
  switch (m) {
    case WM_MOUSEMOVE:
      if (g_hot != h) {
        HWND old = g_hot;
        (void)old;
        g_hot = h;
        TRACKMOUSEEVENT tme = {sizeof(tme), TME_LEAVE, h, 0};
        TrackMouseEvent(&tme);
        request_frame();
      }
      break;
    case WM_MOUSELEAVE:
      if (g_hot == h) { g_hot = NULL; request_frame(); }
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

/* Buttons fade in last during a page's entrance; the close button is always there. */
static double btn_alpha(int id) {
  if (id == ID_CLOSE || !g_animate) return 1;
  double p = clamp01((qpc_ms() - g_stage_ms - 280) / 260);
  return 1 - (1 - p) * (1 - p) * (1 - p);
}

static void draw_button(DRAWITEMSTRUCT *di) {
  int w = di->rcItem.right - di->rcItem.left, h = di->rcItem.bottom - di->rcItem.top;
  Surf s = {0};
  if (!surf_make(&s, di->hDC, w, h)) return;
  Surf *prev = T;
  T = &s;
  double sdy = G_DY, sa = G_A;
  int sy0 = G_Y0, sy1 = G_Y1;
  G_DY = 0; G_A = 1; G_Y0 = 0; G_Y1 = 1 << 30;
  int id = (int)di->CtlID;
  double hv = id >= ID_PRIMARY && id < ID_PRIMARY + 5 ? g_hv[id - ID_PRIMARY] : 0;
  POINT o = {0, 0};
  MapWindowPoints(di->hwndItem, g_ov, &o, 1);
  /* Background: what the window painted under the button (card or footer). */
  for (int y = 0; y < h; y++)
    for (int x = 0; x < w; x++) {
      int sx = o.x + x, sy = o.y + y;
      s.px[y * w + x] = (g_main.px && sx >= 0 && sy >= 0 && sx < g_main.w && sy < g_main.h) ? g_main.px[sy * g_main.w + sx] : C_BG;
    }
  u32 *bgc = (u32 *)HeapAlloc(GetProcessHeap(), 0, (size_t)w * h * 4);
  if (bgc) CopyMemory(bgc, s.px, (size_t)w * h * 4);
  int pressed = (di->itemState & ODS_SELECTED) != 0, focus = (di->itemState & ODS_FOCUS) != 0;
  WCHAR label[128];
  GetWindowTextW(di->hwndItem, label, 128);
  double r = S(8);
  if (id == ID_PRIMARY) {
    rrect(1, 1, w - 2, h - 2, r, pressed ? C_EM_PRESS : mix(C_EM, C_EM_HOT, hv), 1);
    text(label, f_btn, C_ON_EM, 0, 0, w, h, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
  } else if (id == ID_SECONDARY) {
    rrect(1, 1, w - 2, h - 2, r, pressed ? rgb(0x22, 0x32, 0x2A) : rgb(0x17, 0x23, 0x1D), pressed ? 1 : hv);
    rrect_stroke(1, 1, w - 2, h - 2, r, 1, mix(rgb(0x5B, 0x70, 0x65), C_TEXT3, hv));
    text(label, f_btn, C_TEXT, 0, 0, w, h, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
  } else if (id == ID_CHANGE) {
    rrect(1, 1, w - 2, h - 2, r, rgb(0x1C, 0x2B, 0x24), pressed ? 1 : hv);
    text(label, f_btn, C_MINT, 0, 0, w, h, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
  } else if (id == ID_CLOSE) {
    double a = pressed ? 1 : hv;
    u32 red = pressed ? rgb(0x9B, 0x22, 0x18) : rgb(0xC4, 0x2B, 0x1C);
    if (a > 0) for (int i = 0; i < w * h; i++) s.px[i] = mix(s.px[i], red, a);
    text(L"\xE8BB", f_icon, mix(C_TEXT2, rgb(255, 255, 255), a), 0, 0, w, h, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
    r = 0;
  } else if (id == ID_OPT) {
    /* The switch is one shape tweened between off and on: track, ring, knob position and size. */
    const WCHAR *name = g_stage == S_FINISH ? L"Launch KeyReed now" : L"Create a desktop shortcut";
    double sw = S(40), sh = S(22), sx = w - sw - S(2), sy = (h - sh) / 2, on = g_sw;
    int lx = g_stage == S_FINISH ? (int)(sw + S(14)) : 0;
    if (g_stage == S_FINISH) sx = S(2);
    text(name, f_label, C_TEXT, lx, 0, (int)(w - sw - S(16)), h, DT_SINGLELINE | DT_VCENTER);
    u32 off_c = mix(C_TEXT3, C_TEXT, hv);
    rrect(sx, sy, sw, sh, sh / 2, mix(C_EM, C_EM_HOT, hv), on);
    if (on < 1) rrect_stroke(sx, sy, sw, sh, sh / 2, 1.5 * g_s, mix(under((int)sx + 2, (int)sy + (int)(sh / 2)), off_c, 1 - on));
    double ks = S(10) - S(2) * on;          /* knob inset: 5 off, 4 on */
    double kd = sh - 2 * ks;
    double kx = sx + ks + (sw - sh) * on;   /* slides to the right end */
    rrect(kx, sy + ks, kd, kd, kd / 2, mix(off_c, C_ON_EM, on), 1);
  }
  if (focus) rrect_stroke(0, 0, w, h, r + 1, 2 * g_s, C_TEXT);
  double a = btn_alpha(id);
  if (a < 1 && bgc) for (int i = 0; i < w * h; i++) s.px[i] = mix(bgc[i], s.px[i], a);
  if (bgc) HeapFree(GetProcessHeap(), 0, bgc);
  BitBlt(di->hDC, di->rcItem.left, di->rcItem.top, w, h, s.dc, 0, 0, SRCCOPY);
  T = prev;
  G_DY = sdy; G_A = sa; G_Y0 = sy0; G_Y1 = sy1;
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
  for (HWND c = GetWindow(g_parent, GW_CHILD); c; c = GetWindow(c, GW_HWNDNEXT)) {
    if (c == g_ov) continue;
    /* An empty region keeps NSIS's own page invisible even when NSIS shows it again a moment later (no grey flash). */
    if (!GetPropW(c, L"krcov")) {
      SetWindowRgn(c, CreateRectRgn(0, 0, 0, 0), FALSE);
      SetPropW(c, L"krcov", (HANDLE)1);
    }
    if (IsWindowVisible(c)) ShowWindow(c, SW_HIDE);
  }
  if (GetWindow(g_parent, GW_CHILD) != g_ov) SetWindowPos(g_ov, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
}
/* The frame clock drives animation. This slow timer only keeps the wizard's own controls hidden; it also stands in
   for the clock when reduced motion is on (progress text at 10 Hz) or the clock could not start (60 Hz timer). */
static void retimer(void) {
  int ms = 250;
  if (g_animate && !g_clock_ok) ms = 16;
  else if (g_stage == S_PROGRESS && !g_animate) ms = 100;
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

/* The buttons are windows of their own, so they are not in the saved frame: draw them into it, or they would vanish
   the instant the page changes while everything else fades. */
static void snap_buttons_into_prev(void) {
  for (HWND c = GetWindow(g_ov, GW_CHILD); c; c = GetWindow(c, GW_HWNDNEXT)) {
    RECT rc;
    GetWindowRect(c, &rc);
    POINT o = {rc.left, rc.top};
    ScreenToClient(g_ov, &o);
    int w = rc.right - rc.left, h = rc.bottom - rc.top;
    Surf s = {0};
    if (!IsWindowVisible(c) || !surf_make(&s, g_main.dc, w, h)) continue;
    DRAWITEMSTRUCT di = {0};
    di.CtlType = ODT_BUTTON;
    di.CtlID = (UINT)GetDlgCtrlID(c);
    di.hwndItem = c;
    di.hDC = s.dc;
    di.rcItem.right = w;
    di.rcItem.bottom = h;
    di.itemState = GetFocus() == c ? ODS_FOCUS : 0;
    draw_button(&di); /* the same drawing the screen shows, into the saved frame */
    for (int y = 0; y < h; y++)
      for (int x = 0; x < w; x++) {
        int dx = o.x + x, dy = o.y + y;
        if (dx >= 0 && dy >= 0 && dx < g_main.w && dy < g_main.h) g_prev[dy * g_main.w + dx] = s.px[y * w + x];
      }
    surf_free(&s);
  }
}

static void set_stage(int stage) {
  double now = qpc_ms();
  /* Crossfade from the screen that is showing. */
  if (g_animate && g_main.px && g_prev && g_cache_ok && g_stage != S_NONE && stage != g_stage) {
    CopyMemory(g_prev, g_main.px, (size_t)g_main.w * g_main.h * 4);
    snap_buttons_into_prev();
    g_tr_ok = 1;
    g_tr_on = 1;
    g_tr_t0 = now;
  }
  /* "Installing" turning into "installed" is one page: its entrance does not play again. */
  if (!(g_stage == S_PROGRESS && stage == S_DONE)) g_stage_ms = now;
  g_stage = stage;
  g_cache_ok = 0;
  g_hot = NULL;
  for (int i = 0; i < 5; i++) g_hv[i] = 0;
  g_sw = sw_target();
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
  }
  if (stage == S_WELCOME || stage == S_FAILED || stage == S_FINISH) button(ID_CLOSE, L"Close", CW - 46, 0, 46, 32);
  WCHAR name[160];
  wsprintfW(name, L"KeyReed %s", g_uninstall ? L"uninstall" : L"setup");
  SetWindowTextW(g_ov, name);
  cover();
  retimer();
  InvalidateRect(g_ov, NULL, FALSE);
  request_frame();
  if (focus) PostMessageW(g_ov, WM_KR_FOCUS, (WPARAM)focus, 0);
}

static LRESULT CALLBACK ov_wndproc(HWND h, UINT m, WPARAM w, LPARAM l) {
  switch (m) {
    case WM_ERASEBKGND:
      return 1;
    case WM_PAINT: {
      PAINTSTRUCT ps;
      HDC dc = BeginPaint(h, &ps);
      paint(dc, &ps.rcPaint);
      EndPaint(h, &ps);
      return 0;
    }
    case WM_PRINTCLIENT:
      paint((HDC)w, NULL);
      return 0;
    case WM_KR_FRAME:
      InterlockedExchange(&g_pending, 0);
      tick();
      return 0;
    case WM_TIMER:
      cover();
      if (!g_clock_ok || !g_animate) tick();
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
        request_frame(); /* the switch slides */
      }
      return 0;
    }
    case WM_DESTROY:
      KillTimer(h, 1);
      InterlockedExchange(&g_clock_on, 0);
      g_ov = NULL;
      return 0;
  }
  return DefWindowProcW(h, m, w, l);
}

static UINT_PTR __cdecl unload_cb(enum NSPIM msg) {
  if (msg == NSPIM_UNLOAD) {
    perf_dump();
    surf_free(&g_main);
    if (g_cache) { HeapFree(GetProcessHeap(), 0, g_cache); g_cache = NULL; }
    if (g_prev) { HeapFree(GetProcessHeap(), 0, g_prev); g_prev = NULL; }
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
  { /* test hook: KEYREED_SETUP_SCALE=1.5 draws the setup at 150% on a 100% screen, to check every scaled size */
    WCHAR sc[16];
    if (GetEnvironmentVariableW(L"KEYREED_SETUP_SCALE", sc, 16) > 0) { double v = wcstod(sc, NULL); if (v >= 1 && v <= 3) g_s = v; }
  }
  BOOL anim = TRUE;
  SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &anim, 0);
  HIGHCONTRASTW hc = {sizeof(hc)};
  SystemParametersInfoW(SPI_GETHIGHCONTRAST, sizeof(hc), &hc, 0);
  g_hc = (hc.dwFlags & HCF_HIGHCONTRASTON) != 0;
  g_animate = anim && !g_hc;
  fonts();
  words();
  if (!g_wake) {
    g_wake = CreateEventW(NULL, TRUE, FALSE, NULL);
    HANDLE th = g_wake ? CreateThread(NULL, 0, clock_thread, NULL, 0, NULL) : NULL;
    g_clock_ok = th != NULL;
    if (th) CloseHandle(th);
  }

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

/* Keeps the window animating (messages pump, the clock runs) for `ms`, and with `to_full` until the shown progress has
   reached 100% and the progress screen has been up MIN_PROGRESS_MS: an uninstall that finishes in 0.2 s still shows
   its screen, instead of flashing past. Gives up after 2.5 s. */
static void pump_until(double ms, int to_full) {
  if (!g_animate) return;
  MSG m;
  double t0 = qpc_ms();
  while (qpc_ms() - t0 < 2500 && (qpc_ms() - t0 < ms || (to_full && (g_pdisp < .995 || qpc_ms() - g_prog_ms < MIN_PROGRESS_MS)))) {
    while (PeekMessageW(&m, NULL, 0, 0, PM_REMOVE)) {
      if (m.message == WM_QUIT) { PostQuitMessage((int)m.wParam); return; }
      TranslateMessage(&m);
      DispatchMessageW(&m);
    }
    MsgWaitForMultipleObjects(0, NULL, FALSE, 8, QS_ALLINPUT);
  }
}

/* ---------------- stopping a running KeyReed ---------------- */
static int starts_with_i(const WCHAR *s, const WCHAR *prefix) {
  int n = lstrlenW(prefix);
  return lstrlenW(s) >= n && CompareStringOrdinal(s, n, prefix, n, TRUE) == CSTR_EQUAL;
}
/* A file that cannot be opened for writing is still mapped or open by a process. */
static int locked(const WCHAR *path) {
  HANDLE h = CreateFileW(path, GENERIC_WRITE, 0, NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
  if (h != INVALID_HANDLE_VALUE) { CloseHandle(h); return 0; }
  DWORD e = GetLastError();
  return e == ERROR_SHARING_VIOLATION || e == ERROR_USER_MAPPED_FILE;
}
static int any_locked(const WCHAR *dir, const WCHAR *llama) {
  WCHAR p[MAX_PATH * 2];
  static const WCHAR *pats[] = {L"*.dll", L"*.exe"};
  wsprintfW(p, L"%s\\keyreed.exe", dir);
  if (locked(p)) return 1;
  for (int k = 0; k < 2; k++) {
    WIN32_FIND_DATAW fd;
    wsprintfW(p, L"%s%s", llama, pats[k]);
    HANDLE f = FindFirstFileW(p, &fd);
    if (f == INVALID_HANDLE_VALUE) continue;
    do {
      wsprintfW(p, L"%s%s", llama, fd.cFileName);
      if (locked(p)) { FindClose(f); return 1; }
    } while (FindNextFileW(f, &fd));
    FindClose(f);
  }
  return 0;
}
/* Ends the keyreed.exe that runs from <dir> and the llama-server.exe that runs from <dir>\runtime\llama (never another
   copy of KeyReed, such as a dev build, nor another app's llama-server), then waits until Windows has released their
   files. Replaces taskkill + PowerShell/WMI + a fixed sleep, which took 5-7 s at 0%; this takes milliseconds, and
   nothing at all when KeyReed is not running. Native, so it is also correct for the 64-bit processes a 32-bit
   installer cannot inspect from PowerShell. */
static void stop_app(const WCHAR *dirin) {
  WCHAR dir[MAX_PATH * 2], llama[MAX_PATH * 2];
  lstrcpynW(dir, dirin, MAX_PATH);
  int n = lstrlenW(dir);
  while (n > 3 && dir[n - 1] == L'\\') dir[--n] = 0;
  wsprintfW(llama, L"%s\\runtime\\llama\\", dir);
  WCHAR app[MAX_PATH * 2];
  wsprintfW(app, L"%s\\", dir);
  HANDLE snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
  if (snap == INVALID_HANDLE_VALUE) return;
  HANDLE hs[MAXIMUM_WAIT_OBJECTS];
  int cnt = 0;
  PROCESSENTRY32W pe = {sizeof(pe)};
  for (BOOL ok = Process32FirstW(snap, &pe); ok; ok = Process32NextW(snap, &pe)) {
    int kr = lstrcmpiW(pe.szExeFile, L"keyreed.exe") == 0, ll = lstrcmpiW(pe.szExeFile, L"llama-server.exe") == 0;
    if ((!kr && !ll) || pe.th32ProcessID == GetCurrentProcessId() || cnt == MAXIMUM_WAIT_OBJECTS) continue;
    HANDLE h = OpenProcess(PROCESS_TERMINATE | SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pe.th32ProcessID);
    if (!h) continue;
    WCHAR path[MAX_PATH * 2];
    DWORD len = MAX_PATH * 2;
    if (!QueryFullProcessImageNameW(h, 0, path, &len) || !starts_with_i(path, ll ? llama : app)) { CloseHandle(h); continue; }
    if (TerminateProcess(h, 1)) hs[cnt++] = h; else CloseHandle(h);
  }
  CloseHandle(snap);
  if (!cnt) return;
  WaitForMultipleObjects(cnt, hs, TRUE, 3000);
  for (int i = 0; i < cnt; i++) CloseHandle(hs[i]);
  for (int i = 0; i < 100 && any_locked(dir, llama); i++) Sleep(20); /* up to 2 s for the DLLs to be released */
}

/* KeyReedSetup::StopApp "<install dir>" */
__declspec(dllexport) void StopApp(HWND parent, int string_size, WCHAR *variables, void *stacktop, extra_parameters *extra) {
  (void)parent; (void)string_size; (void)variables; (void)extra;
  WCHAR dir[MAX_PATH * 2];
  pop(stacktop, dir, MAX_PATH * 2);
  if (dir[0]) stop_app(dir);
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
  g_prog_ms = qpc_ms();
  g_pdisp = 0;
  g_last_pct = g_last_tip_q = -1;
  g_finishing = 0;
  g_lbl[0] = g_lbl_next[0] = 0;
  set_stage(S_PROGRESS);
}

/* KeyReedSetup::Finishing  (from the progress page's leave function: the work is done, the Finish page is next) */
__declspec(dllexport) void Finishing(HWND parent, int string_size, WCHAR *variables, void *stacktop, extra_parameters *extra) {
  (void)parent; (void)string_size; (void)variables; (void)stacktop; (void)extra;
  if (!g_ov || g_stage != S_PROGRESS) return;
  g_finishing = 1; /* the bar eases to 100% under the "Installing" title, then the page says it is done */
  pump_until(0, 1);
  set_stage(S_DONE);
  pump_until(450, 0);
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
  if (reason == DLL_PROCESS_ATTACH) { g_inst = inst; perf_init(); }
  return TRUE;
}
