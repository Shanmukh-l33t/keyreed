; KeyReed standard installer. Does not embed speech or grammar models.
; The tray menu downloads the default speech model and verifies the compile-time pins.
; The full offline zip is a separate package.
; Uninstall removes the installed program files. It does not delete
; %LOCALAPPDATA%\Keyreed, which holds settings.
;
; Look: MUI2 wizard with KeyReed artwork (installer\welcome.bmp, header.bmp, made by
; installer\gen_art.py from assets\keyreed-icon.png) and a native progress page drawn by the
; KeyReedWave plugin (installer\kr_wave.c). Build with scripts\build-installer.sh.
;
; /DSANDBOX_TEST builds a copy that uses its own registry keys, install folder and Start menu
; folder and never kills running processes or touches the Run key, so the wizard can be exercised
; on a machine that has a real KeyReed installed. It is for testing only and is never released.

!include "MUI2.nsh"

!define APP_NAME "KeyReed"
!define APP_VERSION "0.5.0"

!ifndef PAYLOAD_DIR
  !define PAYLOAD_DIR "..\dist\keyreed-win-x64-slim"
!endif

!ifdef SANDBOX_TEST
  !define REG_APP "Software\KeyReedInstallerTest"
  !define REG_UNINST "Software\Microsoft\Windows\CurrentVersion\Uninstall\KeyReedInstallerTest"
  !define SM_DIR "$SMPROGRAMS\KeyReed (installer test)"
  !define DESKTOP_LNK "$DESKTOP\KeyReed (installer test).lnk"
  Name "${APP_NAME} ${APP_VERSION} [installer test]"
  OutFile "..\dist\KeyReed-Setup-TEST.exe"
  InstallDir "$LOCALAPPDATA\KeyReedInstallerTest"
!else
  !define REG_APP "Software\KeyReed"
  !define REG_UNINST "Software\Microsoft\Windows\CurrentVersion\Uninstall\KeyReed"
  !define SM_DIR "$SMPROGRAMS\KeyReed"
  !define DESKTOP_LNK "$DESKTOP\KeyReed.lnk"
  Name "${APP_NAME} ${APP_VERSION}"
  OutFile "..\dist\KeyReed-Setup-x64.exe"
  InstallDir "$LOCALAPPDATA\Programs\KeyReed"
!endif
InstallDirRegKey HKCU "${REG_APP}" "InstallDir"
RequestExecutionLevel user
Unicode True
SetCompressor /SOLID lzma
Icon "..\assets\keyreed.ico"
UninstallIcon "..\assets\keyreed.ico"
BrandingText "KeyReed ${APP_VERSION}"
ShowInstDetails hide
ShowUninstDetails hide

!addplugindir /x86-unicode "installer\plugins"

!define MUI_ABORTWARNING
!define MUI_ICON "..\assets\keyreed.ico"
!define MUI_UNICON "..\assets\keyreed.ico"
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_BITMAP "installer\header.bmp"
!define MUI_HEADERIMAGE_UNBITMAP "installer\header.bmp"
!define MUI_HEADERIMAGE_RIGHT
!define MUI_WELCOMEFINISHPAGE_BITMAP "installer\welcome.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "installer\welcome.bmp"

!define MUI_WELCOMEPAGE_TITLE "Welcome to KeyReed"
!define MUI_WELCOMEPAGE_TEXT "Voice it. Everywhere.$\r$\n$\r$\nKeyReed turns speech into polished text in any Windows application.$\r$\n$\r$\nSetup installs KeyReed ${APP_VERSION} for your Windows account only, so it does not need administrator rights. Speech models are not part of this installer; you download them later from inside KeyReed.$\r$\n$\r$\nClick Next to continue."
!define MUI_DIRECTORYPAGE_TEXT_TOP "Setup will install KeyReed in the folder below. To install in a different folder, click Browse and select another folder. Your KeyReed settings are kept separately and are not affected."
!define MUI_FINISHPAGE_TITLE "KeyReed is installed"
!define MUI_FINISHPAGE_TEXT "KeyReed ${APP_VERSION} has been installed.$\r$\n$\r$\nIt runs from the system tray. After you launch it, the Get started page in Settings walks you through your first dictation."
!ifndef SANDBOX_TEST
  !define MUI_FINISHPAGE_RUN "$INSTDIR\keyreed.exe"
  !define MUI_FINISHPAGE_RUN_TEXT "Launch KeyReed"
!endif
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
; The show callback belongs to the page inserted right after it, so it is defined here, not earlier.
!define MUI_PAGE_CUSTOMFUNCTION_SHOW InstFilesShow
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

VIProductVersion "0.5.0.0"
VIAddVersionKey "ProductName" "KeyReed"
VIAddVersionKey "FileDescription" "KeyReed — Voice it. Everywhere."
VIAddVersionKey "FileVersion" "${APP_VERSION}"
VIAddVersionKey "ProductVersion" "${APP_VERSION}"
VIAddVersionKey "LegalCopyright" "Copyright (c) 2026 KeyReed"
VIAddVersionKey "CompanyName" "KeyReed"

Function InstFilesShow
  KeyReedWave::Start
FunctionEnd

Function .onInstSuccess
  KeyReedWave::Success
FunctionEnd

Function .onInstFailed
  KeyReedWave::Failed
  SetDetailsView show
FunctionEnd

Section "KeyReed" SecApp
  SectionIn RO
  DetailPrint "Preparing installation"
!ifndef SANDBOX_TEST
  nsExec::Exec 'taskkill /F /IM keyreed.exe'
  nsExec::Exec 'taskkill /F /IM llama-server.exe'
!endif
  DetailPrint "Installing application files"
  ClearErrors
  SetDetailsPrint listonly
  SetOutPath "$INSTDIR"
  File /r "${PAYLOAD_DIR}\*.*"
  SetDetailsPrint both
  IfErrors 0 +3
    SetErrorLevel 2
    Abort "Could not write the application files into $INSTDIR (check folder permissions or disk space)."
  DetailPrint "Registering the uninstaller"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "${REG_APP}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${REG_APP}" "Version" "${APP_VERSION}"
  WriteRegStr HKCU "${REG_UNINST}" "DisplayName" "KeyReed"
  WriteRegStr HKCU "${REG_UNINST}" "DisplayIcon" "$INSTDIR\keyreed.exe"
  WriteRegStr HKCU "${REG_UNINST}" "UninstallString" "$\"$INSTDIR\Uninstall.exe$\""
  WriteRegStr HKCU "${REG_UNINST}" "QuietUninstallString" "$\"$INSTDIR\Uninstall.exe$\" /S"
  WriteRegStr HKCU "${REG_UNINST}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${REG_UNINST}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${REG_UNINST}" "Publisher" "KeyReed"
  WriteRegDWORD HKCU "${REG_UNINST}" "NoModify" 1
  WriteRegDWORD HKCU "${REG_UNINST}" "NoRepair" 1
  DetailPrint "Creating Start menu shortcuts"
  CreateDirectory "${SM_DIR}"
  CreateShortcut "${SM_DIR}\KeyReed.lnk" "$INSTDIR\keyreed.exe" "" "$INSTDIR\keyreed.exe" 0
  CreateShortcut "${SM_DIR}\KeyReed Settings.lnk" "$INSTDIR\keyreed.exe" "--settings" "$INSTDIR\keyreed.exe" 0
  CreateShortcut "${SM_DIR}\Uninstall KeyReed.lnk" "$INSTDIR\Uninstall.exe"
SectionEnd

Section "Desktop shortcut" SecDesktop
  DetailPrint "Creating the desktop shortcut"
  CreateShortcut "${DESKTOP_LNK}" "$INSTDIR\keyreed.exe"
SectionEnd

Section "Uninstall"
!ifndef SANDBOX_TEST
  nsExec::ExecToLog 'taskkill /F /IM keyreed.exe'
  nsExec::ExecToLog 'taskkill /F /IM llama-server.exe'
!endif
  Delete "${DESKTOP_LNK}"
  Delete "${SM_DIR}\KeyReed.lnk"
  Delete "${SM_DIR}\KeyReed Settings.lnk"
  Delete "${SM_DIR}\Uninstall KeyReed.lnk"
  RMDir "${SM_DIR}"
  RMDir /r "$INSTDIR\models"
  RMDir /r "$INSTDIR"
  Delete "$INSTDIR\keyreed.exe"
  Delete "$INSTDIR\Uninstall.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\README.txt"
  Delete "$INSTDIR\CHECKSUMS.sha256"
  RMDir "$INSTDIR"
!ifndef SANDBOX_TEST
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Keyreed"
!endif
  DeleteRegKey HKCU "${REG_UNINST}"
  DeleteRegValue HKCU "${REG_APP}" "InstallDir"
  DeleteRegValue HKCU "${REG_APP}" "Version"
  DeleteRegKey /ifempty HKCU "${REG_APP}"
SectionEnd

LangString DESC_App ${LANG_ENGLISH} "KeyReed application and native speech/grammar runtime (models download on demand in Settings)."
LangString DESC_Desktop ${LANG_ENGLISH} "Optional shortcut on the desktop."
!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecApp} $(DESC_App)
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} $(DESC_Desktop)
!insertmacro MUI_FUNCTION_DESCRIPTION_END
