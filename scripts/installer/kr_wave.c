/*
 * KeyReedWave: NSIS plugin that draws the installer's progress page.
 *
 * A row of waveform bars scrolls in a seamless loop. The bars to the left of the real progress
 * position are filled with the accent colour; the position is read from the installer's own
 * progress bar (control 1004) on every frame, so the animation never moves progress on its own.
 * The percentage printed under the bars comes from the same value. Success()/Failed() are called
 * from .onInstSuccess/.onInstFailed and freeze the bars in a completed or failed look.
 *
 * Drawing is plain GDI into a DIB (no GDI+, no runtime, ~10 KB). With "Animate controls" turned
 * off in Windows (SPI_GETCLIENTAREAANIMATION) or high contrast on, the bars are static and only
 * redraw when progress changes.
 *
 * Build (32-bit unicode NSIS plugin):  see scripts/build-installer.sh
 */
#include <windows.h>
#include <math.h>
#include <stdio.h>
#include "nsis/api.h"

#define IDC_PROGRESS 1004
#define IDC_TOGGLE 1027
#define CLASS_NAME TEXT("KeyReedWave")
#define LOOP_MS 2600.0
#define PI 3.14159265358979

enum { RUNNING, DONE, FAILED };

static HINSTANCE g_inst;
static HWND g_wnd, g_bar;
static int g_state = RUNNING, g_animate = 1, g_hc = 0;
static DWORD g_t0;
static HFONT g_big, g_small;
static HBITMAP g_dib;
static unsigned int *g_px;
static int g_dw, g_dh;

static unsigned int rgb(int r, int g, int b) { return ((unsigned)r << 16) | ((unsigned)g << 8) | (unsigned)b; }
static unsigned int mix(unsigned int a, unsigned int b, double t) {
  if (t < 0) t = 0; if (t > 1) t = 1;
  int r = (int)(((a >> 16) & 255) * (1 - t) + ((b >> 16) & 255) * t + .5);
  int g = (int)(((a >> 8) & 255) * (1 - t) + ((b >> 8) & 255) * t + .5);
  int bl = (int)((a & 255) * (1 - t) + (b & 255) * t + .5);
  return rgb(r, g, bl);
}
static unsigned int sys(int idx) { DWORD c = GetSysColor(idx); return rgb(GetRValue(c), GetGValue(c), GetBValue(c)); }

/* Anti-aliased vertical capsule by signed-distance coverage. */
static void capsule(double cx, double cy, double w, double h, unsigned int col) {
  double r = w / 2;
  for (int y = (int)(cy - h / 2) - 1; y <= (int)(cy + h / 2) + 1; y++) {
    if (y < 0 || y >= g_dh) continue;
    for (int x = (int)(cx - r) - 1; x <= (int)(cx + r) + 1; x++) {
      if (x < 0 || x >= g_dw) continue;
      double dx = fabs(x + .5 - cx), dy = fabs(y + .5 - cy);
      double ey = dy - (h / 2 - r); if (ey < 0) ey = 0;
      double a = .5 - (sqrt(dx * dx + ey * ey) - r);
      if (a <= 0) continue;
      if (a > 1) a = 1;
      g_px[y * g_dw + x] = mix(g_px[y * g_dw + x], col, a);
    }
  }
}

static double frac(void) {
  if (g_state == DONE) return 1.0;
  LRESULT hi = SendMessage(g_bar, 0x0407 /* PBM_GETRANGE */, FALSE, 0);
  LRESULT pos = SendMessage(g_bar, 0x0408 /* PBM_GETPOS */, 0, 0);
  if (hi <= 0) return 0;
  double f = (double)pos / (double)hi;
  return f < 0 ? 0 : f > 1 ? 1 : f;
}

static void text(HDC dc, HFONT f, unsigned int col, const TCHAR *s, int x, int y, int w, int h, UINT fmt) {
  RECT r = { x, y, x + w, y + h };
  SelectObject(dc, f);
  SetTextColor(dc, RGB((col >> 16) & 255, (col >> 8) & 255, col & 255));
  DrawText(dc, s, -1, &r, fmt | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
}

static void paint(HDC wdc, int W, int H) {
  if (!g_dib || g_dw != W || g_dh != H) {
    if (g_dib) DeleteObject(g_dib);
    BITMAPINFO bi = { { sizeof(BITMAPINFOHEADER), W, -H, 1, 32, BI_RGB } };
    g_dib = CreateDIBSection(wdc, &bi, DIB_RGB_COLORS, (void **)&g_px, NULL, 0);
    g_dw = W; g_dh = H;
    if (!g_dib) return;
  }
  int dpi = GetDeviceCaps(wdc, LOGPIXELSX);
  double s = dpi / 96.0;
  unsigned int bg = sys(COLOR_BTNFACE); /* same as the wizard page, so the animation blends in */
  unsigned int on = g_hc ? sys(COLOR_HIGHLIGHT) : rgb(0x04, 0x78, 0x57);
  unsigned int off = g_hc ? sys(COLOR_GRAYTEXT) : rgb(0xB4, 0xC5, 0xBC);
  unsigned int ink = g_hc ? sys(COLOR_WINDOWTEXT) : rgb(0x0E, 0x1A, 0x14);
  unsigned int ink2 = g_hc ? sys(COLOR_WINDOWTEXT) : rgb(0x4A, 0x5A, 0x52);
  if (g_state == FAILED) { on = g_hc ? on : rgb(0xA3, 0x14, 0x3C); }
  for (int i = 0; i < W * H; i++) g_px[i] = bg;

  double textRow = 44 * s, waveH = H - textRow;
  double pitch = 9 * s, bw = 5 * s;
  int n = (int)((W + (pitch - bw)) / pitch);
  double x0 = (W - (n * pitch - (pitch - bw))) / 2;
  double t = (g_animate && g_state == RUNNING) ? fmod((double)(GetTickCount() - g_t0), LOOP_MS) / LOOP_MS : 0.35;
  double f = frac();
  for (int i = 0; i < n; i++) {
    /* Every term repeats an integer number of times per loop, so the loop has no seam. */
    double v = .5 * sin(2 * PI * (.11 * i - t)) + .3 * sin(2 * PI * (.23 * i - 2 * t + .3)) + .2 * sin(2 * PI * (.37 * i + t));
    double env = .55 + .45 * sin(PI * (i + .5) / n);
    double h = waveH * (.14 + .86 * env * (.5 + .5 * v));
    if (h < bw) h = bw;
    double cx = x0 + i * pitch + bw / 2;
    capsule(cx, waveH / 2, bw, h, mix(off, on, (f * W - cx) / pitch + .5));
  }

  HDC dc = CreateCompatibleDC(wdc);
  HGDIOBJ old = SelectObject(dc, g_dib);
  SetBkMode(dc, TRANSPARENT);
  if (!g_big) {
    g_big = CreateFont((int)(-22 * s), 0, 0, 0, FW_SEMIBOLD, 0, 0, 0, DEFAULT_CHARSET, 0, 0, CLEARTYPE_QUALITY, 0, TEXT("Segoe UI"));
    g_small = CreateFont((int)(-13 * s), 0, 0, 0, FW_NORMAL, 0, 0, 0, DEFAULT_CHARSET, 0, 0, CLEARTYPE_QUALITY, 0, TEXT("Segoe UI"));
  }
  TCHAR buf[32];
  wsprintf(buf, TEXT("%d%%"), (int)(f * 100));
  text(dc, g_big, g_state == FAILED ? on : ink, buf, (int)x0, (int)waveH, (int)(120 * s), (int)textRow, DT_LEFT);
  const TCHAR *label = g_state == DONE ? TEXT("Installation complete")
                     : g_state == FAILED ? TEXT("Installation did not finish")
                     : TEXT("Installing");
  text(dc, g_small, ink2, label, W / 2, (int)waveH, W / 2 - (int)x0, (int)textRow, DT_RIGHT);
  BitBlt(wdc, 0, 0, W, H, dc, 0, 0, SRCCOPY);
  SelectObject(dc, old);
  DeleteDC(dc);
}

static LRESULT CALLBACK wndproc(HWND h, UINT m, WPARAM w, LPARAM l) {
  switch (m) {
    case WM_ERASEBKGND: return 1;
    case WM_TIMER: InvalidateRect(h, NULL, FALSE); return 0;
    case WM_PRINTCLIENT: { /* PrintWindow, screen readers and screenshot tools ask for the content this way */
      RECT r; GetClientRect(h, &r);
      if (r.right > 0 && r.bottom > 0) paint((HDC)w, r.right, r.bottom);
      return 0;
    }
    case WM_PAINT: {
      PAINTSTRUCT ps; HDC dc = BeginPaint(h, &ps);
      RECT r; GetClientRect(h, &r);
      if (r.right > 0 && r.bottom > 0) paint(dc, r.right, r.bottom);
      EndPaint(h, &ps); return 0;
    }
    case WM_DESTROY: KillTimer(h, 1); g_wnd = NULL; return 0;
  }
  return DefWindowProc(h, m, w, l);
}

static UINT_PTR __cdecl callback(enum NSPIM msg) {
  if (msg == NSPIM_UNLOAD) {
    if (g_big) DeleteObject(g_big);
    if (g_small) DeleteObject(g_small);
    if (g_dib) DeleteObject(g_dib);
    g_big = g_small = NULL; g_dib = NULL;
  }
  return 0;
}

static void retimer(void) {
  if (g_wnd) { SetTimer(g_wnd, 1, (g_animate && g_state == RUNNING) ? 33 : 200, NULL); InvalidateRect(g_wnd, NULL, FALSE); }
}

__declspec(dllexport) void Start(HWND parent, int string_size, TCHAR *variables, void *stacktop, extra_parameters *extra) {
  (void)string_size; (void)variables; (void)stacktop;
  extra->RegisterPluginCallback(g_inst, callback); /* stay loaded: our window procedure lives here */
  /* The page dialog is a #32770 child of the wizard (1018 is only its placeholder). The previous page's dialog
     may still exist, so take the one that owns the progress bar. */
  HWND dlg = NULL;
  while ((dlg = FindWindowEx(parent, dlg, TEXT("#32770"), NULL)) != NULL)
    if (GetDlgItem(dlg, IDC_PROGRESS)) break;
  if (!dlg) return;
  g_bar = GetDlgItem(dlg, IDC_PROGRESS);
  HWND toggle = GetDlgItem(dlg, IDC_TOGGLE);
  if (!g_bar) return;
  WNDCLASS wc = { 0 };
  wc.lpfnWndProc = wndproc; wc.hInstance = g_inst; wc.lpszClassName = CLASS_NAME; wc.hCursor = LoadCursor(NULL, IDC_ARROW);
  RegisterClass(&wc);
  /* The area under the stock progress bar and its "Show details" button, down to the dialog's edge.
     (Not the details list: with ShowInstDetails hide it has no size.) */
  RECT bar, tg = { 0 }, cl;
  GetWindowRect(g_bar, &bar); MapWindowPoints(NULL, dlg, (POINT *)&bar, 2);
  if (toggle) { GetWindowRect(toggle, &tg); MapWindowPoints(NULL, dlg, (POINT *)&tg, 2); }
  GetClientRect(dlg, &cl);
  HDC sdc = GetDC(dlg);
  int gap = MulDiv(14, GetDeviceCaps(sdc, LOGPIXELSX), 96);
  ReleaseDC(dlg, sdc);
  RECT r = { bar.left, (tg.bottom > bar.bottom ? tg.bottom : bar.bottom) + gap, bar.right, cl.bottom - gap / 2 };
  if (r.bottom - r.top < 40) return; /* no room: keep the stock page */
  BOOL anim = TRUE; SystemParametersInfo(SPI_GETCLIENTAREAANIMATION, 0, &anim, 0);
  HIGHCONTRAST hc = { sizeof(hc) }; SystemParametersInfo(SPI_GETHIGHCONTRAST, sizeof(hc), &hc, 0);
  g_hc = (hc.dwFlags & HCF_HIGHCONTRASTON) != 0;
  g_animate = anim && !g_hc;
  g_state = RUNNING; g_t0 = GetTickCount();
  g_wnd = CreateWindowEx(0, CLASS_NAME, TEXT("Installation progress"), WS_CHILD | WS_VISIBLE,
                         r.left, r.top, r.right - r.left, r.bottom - r.top, dlg, NULL, g_inst, NULL);
  if (!g_wnd) return;
  /* Bottom of the z-order: if the user clicks "Show details" the log list covers the animation. */
  SetWindowPos(g_wnd, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
  retimer();
}

__declspec(dllexport) void Success(HWND parent, int string_size, TCHAR *variables, void *stacktop, extra_parameters *extra) {
  (void)parent; (void)string_size; (void)variables; (void)stacktop; (void)extra;
  g_state = DONE; retimer();
}

__declspec(dllexport) void Failed(HWND parent, int string_size, TCHAR *variables, void *stacktop, extra_parameters *extra) {
  (void)parent; (void)string_size; (void)variables; (void)stacktop; (void)extra;
  g_state = FAILED; retimer();
}

BOOL WINAPI DllMain(HINSTANCE inst, DWORD reason, LPVOID reserved) {
  (void)reserved;
  if (reason == DLL_PROCESS_ATTACH) g_inst = inst;
  return TRUE;
}
