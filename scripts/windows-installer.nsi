; Windows installer for Clonk Rust.
;
; Wraps the staged payload that `cargo xtask package --no-archive` leaves in
; target/dist/clonk-rust/. Cross-built from Linux with makensis; see
; .github/workflows/release.yml.
;
; Required defines:
;   VERSION  release version, e.g. 0.2.0
;   PAYLOAD  absolute path to the staged clonk-rust directory
;   OUTFILE  absolute path of the installer to write
;
; Optional defines:
;   ICON     absolute path to the product .ico, which `cargo xtask package`
;            writes beside the staged payload. Optional so a stand-in payload
;            still compiles; without it the installer keeps the NSIS default
;            icon, which is the only thing here that is not the product mark.

!ifndef VERSION
  !error "VERSION must be defined (-DVERSION=x.y.z)"
!endif
!ifndef PAYLOAD
  !error "PAYLOAD must be defined (-DPAYLOAD=/path/to/target/dist/clonk-rust)"
!endif
!ifndef OUTFILE
  !error "OUTFILE must be defined (-DOUTFILE=/path/to/setup.exe)"
!endif

!include "MUI2.nsh"
!include "FileFunc.nsh"

Name "Clonk Rust ${VERSION}"
OutFile "${OUTFILE}"
Unicode true

; Per-user install: no UAC prompt, and the game keeps its own config and logs
; under the user profile anyway.
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\Clonk Rust"
InstallDirRegKey HKCU "Software\Clonk Rust" "InstallDir"

; The payload is ~326 MB of engine and game data; solid LZMA is worth the
; compression time here.
SetCompressor /SOLID lzma

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "Clonk Rust"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "Clonk Rust installer"
VIAddVersionKey "LegalCopyright" "See COPYING"

!define MUI_ABORTWARNING

; The installer and uninstaller are executables of their own, so they need the
; icon compiled in; the payload's own resources cannot reach them. Must precede
; the page macros — MUI2 reads these when the pages are inserted.
!ifdef ICON
  !define MUI_ICON "${ICON}"
  !define MUI_UNICON "${ICON}"
!endif

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\ClonkRust"

Section "Clonk Rust" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"

  ; Whole staged tree: bin\, planet\, content\ and the notices. The engine
  ; locates its data root by walking up from the executable looking for
  ; planet\System.c4g, so this layout must be preserved verbatim.
  File /r "${PAYLOAD}\*.*"

  WriteRegStr HKCU "Software\Clonk Rust" "InstallDir" "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  ; The launcher is the correct Windows entry point: it prepares the config,
  ; resolves the runtime and collects logs. (On macOS the runtime is launched
  ; directly instead, because the launcher writes next to a read-only image.)
  CreateDirectory "$SMPROGRAMS\Clonk Rust"
  CreateShortcut "$SMPROGRAMS\Clonk Rust\Clonk Rust.lnk" "$INSTDIR\bin\clonk-game.exe" "" "$INSTDIR\bin\clonk-game.exe" 0
  CreateShortcut "$SMPROGRAMS\Clonk Rust\Uninstall Clonk Rust.lnk" "$INSTDIR\Uninstall.exe"

  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "Clonk Rust"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "Clonk Rust"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\bin\clonk-game.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" "$\"$INSTDIR\Uninstall.exe$\""
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1

  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" "$0"
SectionEnd

Section "Uninstall"
  Delete "$SMPROGRAMS\Clonk Rust\Clonk Rust.lnk"
  Delete "$SMPROGRAMS\Clonk Rust\Uninstall Clonk Rust.lnk"
  RMDir "$SMPROGRAMS\Clonk Rust"

  ; Only what the installer laid down. Player profiles, configuration and logs
  ; live under the user profile and are deliberately left alone.
  RMDir /r "$INSTDIR\bin"
  RMDir /r "$INSTDIR\planet"
  RMDir /r "$INSTDIR\content"
  Delete "$INSTDIR\COPYING"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\credits.txt"
  Delete "$INSTDIR\THIRD_PARTY_GAME_CONTENT.md"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"

  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\Clonk Rust"
SectionEnd
