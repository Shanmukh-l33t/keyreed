; KeyReed standard installer. Does not embed speech or grammar models.
; The tray menu downloads the default speech model and verifies the compile-time pins.
; The full offline zip is a separate package.
; Uninstall removes the installed program files. It does not delete
; %LOCALAPPDATA%\Keyreed, which holds settings.
;
; Look: every screen (welcome, install animation, finish, and the same for uninstall) is drawn by
; the KeyReedSetup plugin (installer\kr_setup.c) over the wizard window; NSIS still does all of the
; installing. Install progress and status shown on screen are read from NSIS's own progress bar and
; status text. Build with scripts\build-installer.sh.
;
; /DSANDBOX_TEST builds a copy that uses its own registry keys, install folder and Start menu
; folder and never kills running processes or touches the Run key, so the wizard can be exercised
; on a machine that has a real KeyReed installed. It is for testing only and is never released.

!include "MUI2.nsh"
!include "LogicLib.nsh"

!define APP_NAME "KeyReed"
!define APP_VERSION "2.0.0-beta.1"

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
ManifestDPIAware true
ShowInstDetails hide
ShowUninstDetails hide

!addplugindir /x86-unicode "installer\plugins"

!define MUI_ABORTWARNING
!define MUI_UNABORTWARNING
!define MUI_ICON "..\assets\keyreed.ico"
!define MUI_UNICON "..\assets\keyreed.ico"
!define MUI_CUSTOMFUNCTION_GUIINIT GuiInit
!define MUI_CUSTOMFUNCTION_UNGUIINIT un.GuiInit

Page custom WelcomeShow WelcomeLeave
; The show callback belongs to the page inserted right after it.
!define MUI_PAGE_CUSTOMFUNCTION_SHOW InstFilesShow
!insertmacro MUI_PAGE_INSTFILES
Page custom FinishShow FinishLeave

UninstPage custom un.WelcomeShow
!define MUI_PAGE_CUSTOMFUNCTION_SHOW un.InstFilesShow
!insertmacro MUI_UNPAGE_INSTFILES
UninstPage custom un.FinishShow
!insertmacro MUI_LANGUAGE "English"

VIProductVersion "2.0.0.0"
VIAddVersionKey "ProductName" "KeyReed"
VIAddVersionKey "FileDescription" "KeyReed — Voice it. Everywhere."
VIAddVersionKey "FileVersion" "${APP_VERSION}"
VIAddVersionKey "ProductVersion" "${APP_VERSION}"
VIAddVersionKey "LegalCopyright" "Copyright (c) 2026 KeyReed"
VIAddVersionKey "CompanyName" "KeyReed"

Function GuiInit
  KeyReedSetup::Init "install"
FunctionEnd

Function un.GuiInit
  KeyReedSetup::Init "uninstall"
FunctionEnd

Function WelcomeShow
  nsDialogs::Create 1018
  Pop $0
  KeyReedSetup::Welcome "install" "${APP_VERSION}"
  nsDialogs::Show
FunctionEnd

Function InstFilesShow
  KeyReedSetup::Progress "install"
FunctionEnd

Function .onInstSuccess
  KeyReedSetup::Success
FunctionEnd

Function .onInstFailed
  KeyReedSetup::Failed
FunctionEnd

Function FinishShow
  nsDialogs::Create 1018
  Pop $0
  KeyReedSetup::Finish "install"
  nsDialogs::Show
FunctionEnd

Function FinishLeave
  KeyReedSetup::Option "launch"
  Pop $0
  ${If} $0 == "1"
!ifdef SANDBOX_TEST
    ; The test build never starts a second KeyReed next to the user's real one.
    Exec '"$INSTDIR\keyreed.exe" --version'
!else
    Exec '"$INSTDIR\keyreed.exe"'
!endif
  ${EndIf}
FunctionEnd

Function un.WelcomeShow
  nsDialogs::Create 1018
  Pop $0
  KeyReedSetup::Welcome "uninstall" "${APP_VERSION}"
  nsDialogs::Show
FunctionEnd

Function un.InstFilesShow
  KeyReedSetup::Progress "uninstall"
FunctionEnd

Function un.onUninstSuccess
  KeyReedSetup::Success
FunctionEnd

Function un.onUninstFailed
  KeyReedSetup::Failed
FunctionEnd

Function un.FinishShow
  nsDialogs::Create 1018
  Pop $0
  KeyReedSetup::Finish "uninstall"
  nsDialogs::Show
FunctionEnd

; Show one human stage name in the status line; NSIS's per-file log lines go only to the hidden list.
!macro STAGE TEXT
  SetDetailsPrint both
  DetailPrint "${TEXT}"
  SetDetailsPrint listonly
!macroend

Section "KeyReed" SecApp
  SectionIn RO
  !insertmacro STAGE "Preparing KeyReed"
!ifndef SANDBOX_TEST
  nsExec::Exec 'taskkill /F /IM keyreed.exe'
  ; Only KeyReed's own llama-server (under the install folder), never another app's.
  nsExec::Exec `powershell -NoProfile -NonInteractive -Command "Get-Process llama-server -ErrorAction SilentlyContinue | Where-Object { $$_.Path -and $$_.Path.StartsWith('$INSTDIR\runtime\llama\', 'OrdinalIgnoreCase') } | Stop-Process -Force"`
!endif
  !insertmacro STAGE "Installing application files"
  ClearErrors
  SetOutPath "$INSTDIR"
  File /r "${PAYLOAD_DIR}\*.*"
  SetDetailsPrint both
  IfErrors 0 +3
    SetErrorLevel 2
    Abort "Could not write the application files into $INSTDIR (check folder permissions or disk space)."
  !insertmacro STAGE "Registering KeyReed with Windows"
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
  !insertmacro STAGE "Creating shortcuts"
  CreateDirectory "${SM_DIR}"
  CreateShortcut "${SM_DIR}\KeyReed.lnk" "$INSTDIR\keyreed.exe" "" "$INSTDIR\keyreed.exe" 0
  CreateShortcut "${SM_DIR}\KeyReed Settings.lnk" "$INSTDIR\keyreed.exe" "--settings" "$INSTDIR\keyreed.exe" 0
  CreateShortcut "${SM_DIR}\Uninstall KeyReed.lnk" "$INSTDIR\Uninstall.exe"
  SetAutoClose true
SectionEnd

Section "Desktop shortcut" SecDesktop
  !insertmacro STAGE "Creating the desktop shortcut"
  CreateShortcut "${DESKTOP_LNK}" "$INSTDIR\keyreed.exe"
SectionEnd

Section "Uninstall"
  !insertmacro STAGE "Closing KeyReed"
!ifndef SANDBOX_TEST
  nsExec::ExecToLog 'taskkill /F /IM keyreed.exe'
  nsExec::ExecToLog `powershell -NoProfile -NonInteractive -Command "Get-Process llama-server -ErrorAction SilentlyContinue | Where-Object { $$_.Path -and $$_.Path.StartsWith('$INSTDIR\runtime\llama\', 'OrdinalIgnoreCase') } | Stop-Process -Force"`
!endif
  !insertmacro STAGE "Removing shortcuts"
  Delete "${DESKTOP_LNK}"
  Delete "${SM_DIR}\KeyReed.lnk"
  Delete "${SM_DIR}\KeyReed Settings.lnk"
  Delete "${SM_DIR}\Uninstall KeyReed.lnk"
  RMDir "${SM_DIR}"
  !insertmacro STAGE "Removing program files"
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
  !insertmacro STAGE "Unregistering KeyReed"
  DeleteRegKey HKCU "${REG_UNINST}"
  DeleteRegValue HKCU "${REG_APP}" "InstallDir"
  DeleteRegValue HKCU "${REG_APP}" "Version"
  DeleteRegKey /ifempty HKCU "${REG_APP}"
  SetAutoClose true
SectionEnd

Function WelcomeLeave
  KeyReedSetup::Option "desktop"
  Pop $0
  ${If} $0 == "0"
    SectionSetFlags ${SecDesktop} 0
  ${Else}
    SectionSetFlags ${SecDesktop} ${SF_SELECTED}
  ${EndIf}
FunctionEnd
