# Dark and light theme for the Settings window

Settings now follows Windows **Settings > Personalization > Colors > "Choose your default app mode"**
(`HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\AppsUseLightTheme`).
No new KeyReed setting and no change to saved configuration: the choice is read from Windows, so it
switches with Windows (a `WM_SETTINGCHANGE` re-reads it and repaints). A missing or unreadable value
means light. High-contrast mode still wins and uses the system colours. The listener capsule was already
dark in both modes and is unchanged.

`KEYREED_UI_THEME=dark|light` overrides the Windows value; it exists for screenshots and testing.

## What changed (presentation only)

`crates/vi-win32/src/settings.rs`: a second `Theme` palette, the registry reader, the dark title bar
(`DWMWA_USE_IMMERSIVE_DARK_MODE`), and `SetWindowTheme` for the system-drawn drop-downs, text boxes and
tooltips (`DarkMode_CFD` / `DarkMode_Explorer`; older Windows builds ignore the names and keep the light
look). No handler, control id, layout, setting or persistence was touched.

## Dark palette (all pairs checked, enforced by the `both_themes_meet_wcag_aa` unit test)

| Token | Value | Role | Contrast |
|---|---|---|---|
| bg / sidebar / surface | `#0F1713` / `#0B120F` / `#17231D` | page / navigation / cards | |
| text / text2 / text3 | `#E8F1EC` / `#A9BBB2` / `#8DA398` | primary / secondary / metadata | 14.1 / 8.1 / 6.0 on surface |
| accent (text, hover fill) | `#5EE0B0` | links, selected nav icon | 9.9 on surface |
| accent_fill / press | `#34D399` / `#10B981` | primary button, switch ON | dark label `#04241A` 8.6 / 6.5 |
| accent_soft | `#143C2E` | selected row tint | accent text 7.4 |
| control_border | `#6F8479` | outline of secondary buttons | 4.1 (needs 3) |
| ok / warn / error | `#7BE8BC` / `#F8CE85` / `#FDA4AF` on tinted fills | status chips | 8.2-9.2 |

The light palette keeps the contrast fixes from `01-design-audit.md`; the same test enforces it.

## Tested / not tested

- Tested: both palettes pass the contrast test; every Settings page and the first-run steps captured in
  both schemes at 100 % and 150 % (`docs/design/audit/theme/`); dark title bar, drop-down and text boxes
  verified in the screenshots on Windows 11 Pro.
- Not tested: switching Windows' app mode *while Settings is open* (would need changing the user's
  system setting); Windows 10 (no machine available); the native tooltip's dark style (hover tooltips
  were not captured).
