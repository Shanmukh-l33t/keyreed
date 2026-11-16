; KeyReed standard installer. Does not embed speech or grammar models.
; The tray menu downloads the default speech model and verifies the compile-time pins.
; The full offline zip is a separate package.
; Uninstall removes the installed program files. It does not delete
; %LOCALAPPDATA%\Keyreed, which holds settings.

!include "MUI2.nsh"

!define APP_NAME "KeyReed"
!define APP_VERSION "0.3.0"

Name "${APP_NAME} ${APP_VERSION}"
OutFile "..\dist\KeyReed-Setup-x64.exe"
InstallDir "$LOCALAPPDATA\Programs\KeyReed"
InstallDirRegKey HKCU "Software\KeyReed" "InstallDir"
RequestExecutionLevel user
Unicode True
SetCompressor /SOLID lzma
Icon "..\assets\keyreed.ico"
UninstallIcon "..\assets\keyreed.ico"

!define MUI_ABORTWARNING
!define MUI_ICON "..\assets\keyreed.ico"
!define MUI_UNICON "..\assets\keyreed.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\keyreed.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Launch KeyReed"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

VIProductVersion "0.3.0.0"
VIAddVersionKey "ProductName" "KeyReed"
VIAddVersionKey "FileDescription" "KeyReed — Voice it. Everywhere."
VIAddVersionKey "FileVersion" "${APP_VERSION}"
VIAddVersionKey "ProductVersion" "${APP_VERSION}"
VIAddVersionKey "LegalCopyright" "Copyright (c) 2026 KeyReed"
VIAddVersionKey "CompanyName" "KeyReed"

Section "KeyReed" SecApp
  SectionIn RO
  nsExec::ExecToLog 'taskkill /F /IM keyreed.exe'
  nsExec::ExecToLog 'taskkill /F /IM llama-server.exe'
  SetOutPath "$INSTDIR"
  File /r "..\dist\keyreed-win-x64-slim\*.*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\KeyReed" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\KeyReed" "Version" "${APP_VERSION}"
  CreateDirectory "$SMPROGRAMS\KeyReed"
  CreateShortcut "$SMPROGRAMS\KeyReed\KeyReed.lnk" "$INSTDIR\keyreed.exe" "" "$INSTDIR\keyreed.exe" 0
  CreateShortcut "$SMPROGRAMS\KeyReed\KeyReed Settings.lnk" "$INSTDIR\keyreed.exe" "--settings" "$INSTDIR\keyreed.exe" 0
  CreateShortcut "$SMPROGRAMS\KeyReed\Uninstall KeyReed.lnk" "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\KeyReed" "DisplayName" "KeyReed"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\KeyReed" "DisplayIcon" "$INSTDIR\keyreed.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\KeyReed" "UninstallString" "$\"$INSTDIR\Uninstall.exe$\""
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\KeyReed" "QuietUninstallString" "$\"$INSTDIR\Uninstall.exe$\" /S"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\KeyReed" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\KeyReed" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\KeyReed" "Publisher" "KeyReed"
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\KeyReed" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\KeyReed" "NoRepair" 1
SectionEnd

Section "Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\KeyReed.lnk" "$INSTDIR\keyreed.exe"
SectionEnd

Section "Uninstall"
  nsExec::ExecToLog 'taskkill /F /IM keyreed.exe'
  nsExec::ExecToLog 'taskkill /F /IM llama-server.exe'
  Delete "$DESKTOP\KeyReed.lnk"
  Delete "$SMPROGRAMS\KeyReed\KeyReed.lnk"
  Delete "$SMPROGRAMS\KeyReed\KeyReed Settings.lnk"
  Delete "$SMPROGRAMS\KeyReed\Uninstall KeyReed.lnk"
  RMDir "$SMPROGRAMS\KeyReed"
  RMDir /r "$INSTDIR\models"
  RMDir /r "$INSTDIR"
  Delete "$INSTDIR\keyreed.exe"
  Delete "$INSTDIR\Uninstall.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\README.txt"
  Delete "$INSTDIR\CHECKSUMS.sha256"
  RMDir "$INSTDIR"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Keyreed"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\KeyReed"
  DeleteRegValue HKCU "Software\KeyReed" "InstallDir"
  DeleteRegValue HKCU "Software\KeyReed" "Version"
  DeleteRegKey /ifempty HKCU "Software\KeyReed"
SectionEnd

LangString DESC_App ${LANG_ENGLISH} "KeyReed application and native speech/grammar runtime (models download on demand in Settings)."
LangString DESC_Desktop ${LANG_ENGLISH} "Optional shortcut on the desktop."
!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecApp} $(DESC_App)
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} $(DESC_Desktop)
!insertmacro MUI_FUNCTION_DESCRIPTION_END
