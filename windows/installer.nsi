; Still for Windows: installs Firefox ESR and Still's launcher for the current
; user (no administrator rights). Built by windows/package.py, which passes
; VERSION, ARCH, FIREFOX_VERSION, STAGE (the files to install), OUTFILE and ICON;
; FIREFOX_BUILDID only for Still's own Firefox build (windows/gecko/build.py).
;
; It replaces the earlier Tauri build in place: same folder, Start menu entry
; and uninstall entry. That build's updater runs this installer with
; "/P /UPDATE" and quits; Still's launcher does the same for later updates.
;   /P       progress only, no questions
;   /S       silent
;   /UPDATE  start Still when done

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma
SetCompressorDictSize 64
RequestExecutionLevel user

!include MUI2.nsh
!include FileFunc.nsh
!include WordFunc.nsh
!include LogicLib.nsh
!include x64.nsh

!define PRODUCT "Still"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT}"
; Where the Tauri build recorded its folder.
!define PRODUCTKEY "Software\${PRODUCT}\${PRODUCT}"
!define DATA "$LOCALAPPDATA\StillBrowser"
; Registers Still as a browser Windows can make the default (Settings ›
; Apps › Default apps). Windows lets only the user choose; still.exe
; --make-default opens that page.
!define CLIENTKEY "Software\Clients\StartMenuInternet\${PRODUCT}"
!define CAPABILITIES "${CLIENTKEY}\Capabilities"

Name "${PRODUCT}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\${PRODUCT}"
InstallDirRegKey HKCU "${PRODUCTKEY}" ""
BrandingText "${PRODUCT} ${VERSION} · Firefox ${FIREFOX_VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${PRODUCT}"
VIAddVersionKey "FileDescription" "${PRODUCT} Setup"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "© 2026 Still"

Var Passive
Var Update

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING

!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipWhenPassive
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipWhenPassive
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_FUNCTION StartStill
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "Japanese"

Function SkipWhenPassive
  ${If} $Passive == 1
    Abort
  ${EndIf}
FunctionEnd

Function StartStill
  Exec '"$INSTDIR\still.exe"'
FunctionEnd

Function .onInit
  StrCpy $Passive 0
  StrCpy $Update 0
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/P" $R1
  ${IfNot} ${Errors}
    StrCpy $Passive 1
  ${EndIf}
  ClearErrors
  ${GetOptions} $R0 "/UPDATE" $R1
  ${IfNot} ${Errors}
    StrCpy $Update 1
    StrCpy $Passive 1
  ${EndIf}
  ${If} ${Silent}
    StrCpy $Passive 1
  ${EndIf}
  !if "${ARCH}" == "arm64"
    ${IfNot} ${IsNativeARM64}
      MessageBox MB_ICONSTOP "このインストーラーは ARM64 版の Windows 用です。x64 版のインストーラーを使ってください。" /SD IDOK
      Abort
    ${EndIf}
  !endif
FunctionEnd

; A document type (StillHTML) or link type (StillURL) that still.exe opens.
!macro PROGID ID NAME
  WriteRegStr HKCU "Software\Classes\${ID}" "" "${NAME}"
  WriteRegStr HKCU "Software\Classes\${ID}" "FriendlyTypeName" "${NAME}"
  WriteRegStr HKCU "Software\Classes\${ID}\DefaultIcon" "" '"$INSTDIR\still.exe",0'
  WriteRegStr HKCU "Software\Classes\${ID}\Application" "ApplicationName" "${PRODUCT}"
  WriteRegStr HKCU "Software\Classes\${ID}\Application" "ApplicationIcon" '"$INSTDIR\still.exe",0'
  WriteRegStr HKCU "Software\Classes\${ID}\shell\open\command" "" '"$INSTDIR\still.exe" "%1"'
!macroend

!macro FILE_TYPE EXT
  WriteRegStr HKCU "${CAPABILITIES}\FileAssociations" "${EXT}" "StillHTML"
  WriteRegStr HKCU "Software\Classes\${EXT}\OpenWithProgids" "StillHTML" ""
!macroend

!macro UN_FILE_TYPE EXT
  DeleteRegValue HKCU "Software\Classes\${EXT}\OpenWithProgids" "StillHTML"
!macroend

; Pages Still offers to open: the same list Firefox registers.
!macro FILE_TYPES DO
  !insertmacro ${DO} ".htm"
  !insertmacro ${DO} ".html"
  !insertmacro ${DO} ".shtml"
  !insertmacro ${DO} ".xht"
  !insertmacro ${DO} ".xhtml"
  !insertmacro ${DO} ".svg"
  !insertmacro ${DO} ".webp"
  !insertmacro ${DO} ".avif"
  !insertmacro ${DO} ".pdf"
!macroend

Function RegisterBrowser
  !insertmacro PROGID "StillHTML" "Still HTML Document"
  !insertmacro PROGID "StillURL" "Still URL"
  WriteRegStr HKCU "Software\Classes\StillURL" "URL Protocol" ""
  WriteRegDWORD HKCU "Software\Classes\StillURL" "EditFlags" 2

  WriteRegStr HKCU "${CLIENTKEY}" "" "${PRODUCT}"
  WriteRegStr HKCU "${CLIENTKEY}\DefaultIcon" "" '"$INSTDIR\still.exe",0'
  WriteRegStr HKCU "${CLIENTKEY}\shell\open\command" "" '"$INSTDIR\still.exe"'
  WriteRegStr HKCU "${CAPABILITIES}" "ApplicationName" "${PRODUCT}"
  WriteRegStr HKCU "${CAPABILITIES}" "ApplicationDescription" "Firefox のエンジンで表示する、静かなブラウザ"
  WriteRegStr HKCU "${CAPABILITIES}" "ApplicationIcon" '"$INSTDIR\still.exe",0'
  WriteRegStr HKCU "${CAPABILITIES}\StartMenu" "StartMenuInternet" "${PRODUCT}"
  WriteRegStr HKCU "${CAPABILITIES}\URLAssociations" "http" "StillURL"
  WriteRegStr HKCU "${CAPABILITIES}\URLAssociations" "https" "StillURL"
  !insertmacro FILE_TYPES FILE_TYPE
  WriteRegStr HKCU "Software\RegisteredApplications" "${PRODUCT}" "${CAPABILITIES}"
  ; SHCNE_ASSOCCHANGED: Default apps and "Open with" pick up the change now.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
FunctionEnd

; Still's Firefox holds this file open, without sharing, while it runs.
!macro WAIT_FOR_STILL UN
Function ${UN}WaitForStill
  retry:
  ${IfNot} ${FileExists} "${DATA}\Profile\parent.lock"
    Return
  ${EndIf}
  ClearErrors
  FileOpen $0 "${DATA}\Profile\parent.lock" r
  ${IfNot} ${Errors}
    FileClose $0
    Return
  ${EndIf}
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "Still が起動しています。Still を閉じてから「再試行」を押してください。" /SD IDCANCEL IDRETRY retry
  Quit
FunctionEnd
!macroend
!insertmacro WAIT_FOR_STILL ""
!insertmacro WAIT_FOR_STILL "un."

Section "Install"
  SetShellVarContext current
  Call WaitForStill

  ; The earlier build quits right after starting this installer: wait up to
  ; ten seconds for it to let go of its program file.
  StrCpy $1 0
  old_build:
  ${If} ${FileExists} "$INSTDIR\still-browser.exe"
    ClearErrors
    Delete "$INSTDIR\still-browser.exe"
    ${If} ${Errors}
      IntOp $1 $1 + 1
      ${If} $1 > 40
        MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "以前の Still が起動しています。閉じてから「再試行」を押してください。" /SD IDCANCEL IDRETRY retry_old
        Quit
        retry_old:
        StrCpy $1 0
      ${EndIf}
      Sleep 250
      Goto old_build
    ${EndIf}
  ${EndIf}

  SetOutPath "$INSTDIR"
  File "${STAGE}\still.exe"

!ifdef FIREFOX_BUILDID
  ; Still's own Firefox does not update itself: install it unless this exact
  ; build is already there. A BuildID can also occur in Mozilla's build, so
  ; also require the absence of updater.exe (--disable-updater).
  ReadINIStr $2 "$INSTDIR\firefox\application.ini" "App" "BuildID"
  StrCpy $3 1
  ${If} ${FileExists} "$INSTDIR\firefox\firefox.exe"
  ${AndIfNot} ${FileExists} "$INSTDIR\firefox\updater.exe"
  ${AndIf} $2 == "${FIREFOX_BUILDID}"
    StrCpy $3 0
  ${EndIf}
!else
  ; Keep a newer Mozilla runtime only if it can update itself. Still's own
  ; --disable-updater build must be replaced even at the same or a newer version.
  ReadINIStr $2 "$INSTDIR\firefox\application.ini" "App" "Version"
  StrCpy $3 1
  ${If} $2 != ""
  ${AndIf} ${FileExists} "$INSTDIR\firefox\firefox.exe"
  ${AndIf} ${FileExists} "$INSTDIR\firefox\updater.exe"
    ${VersionCompare} "${FIREFOX_VERSION}" "$2" $3
  ${EndIf}
!endif
  ${If} $3 == 1
    DetailPrint "Firefox ${FIREFOX_VERSION} をインストールしています…"
    RMDir /r "$INSTDIR\firefox"
    SetOutPath "$INSTDIR\firefox"
    File /r "${STAGE}\firefox\*.*"
  ${Else}
    ; Refresh every Still-owned file even when Mozilla's newer copy stays.
    SetOutPath "$INSTDIR\firefox\distribution"
    File "${STAGE}\firefox\distribution\policies.json"
    SetOutPath "$INSTDIR\firefox\defaults\pref"
    File "${STAGE}\firefox\defaults\pref\still-autoconfig.js"
    SetOutPath "$INSTDIR\firefox"
    File "${STAGE}\firefox\still.cfg"
    SetOutPath "$INSTDIR\firefox\still"
    File /r "${STAGE}\firefox\still\*.*"
!ifndef FIREFOX_BUILDID
    SetOutPath "$INSTDIR\firefox\browser\chrome\icons\default"
    File "${STAGE}\firefox\browser\chrome\icons\default\main-window.ico"
    File "${STAGE}\firefox\browser\chrome\icons\default\default.ico"
!endif
  ${EndIf}

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "${PRODUCTKEY}" "" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "${PRODUCT}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" '"$INSTDIR\still.exe"'
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "${PRODUCT}"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" '"$INSTDIR"'
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
  DeleteRegValue HKCU "${UNINSTKEY}" "MainBinaryName"
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINSTKEY}" "EstimatedSize" "$0"
  Call RegisterBrowser

  CreateShortcut "$SMPROGRAMS\${PRODUCT}.lnk" "$INSTDIR\still.exe" "" "$INSTDIR\still.exe" 0
  ; Point a desktop shortcut the earlier build made at the launcher.
  ${If} ${FileExists} "$DESKTOP\${PRODUCT}.lnk"
    CreateShortcut "$DESKTOP\${PRODUCT}.lnk" "$INSTDIR\still.exe" "" "$INSTDIR\still.exe" 0
  ${EndIf}

  ${If} $Update == 1
    Exec '"$INSTDIR\still.exe"'
  ${EndIf}
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  Call un.WaitForStill
  RMDir /r "$INSTDIR\firefox"
  Delete "$INSTDIR\still.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${PRODUCT}.lnk"
  Delete "$DESKTOP\${PRODUCT}.lnk"
  DeleteRegKey HKCU "${UNINSTKEY}"
  DeleteRegKey HKCU "${PRODUCTKEY}"
  DeleteRegKey /ifempty HKCU "Software\${PRODUCT}"
  ; Windows falls back to another browser for links Still was handling.
  DeleteRegValue HKCU "Software\RegisteredApplications" "${PRODUCT}"
  DeleteRegKey HKCU "${CLIENTKEY}"
  DeleteRegKey HKCU "Software\Classes\StillHTML"
  DeleteRegKey HKCU "Software\Classes\StillURL"
  !insertmacro FILE_TYPES UN_FILE_TYPE
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  ; Bookmarks, history, sign-ins and settings stay unless the user says otherwise.
  MessageBox MB_YESNO|MB_DEFBUTTON2|MB_ICONQUESTION "ブックマーク・履歴・ログイン状態などの閲覧データも削除しますか？$\n（${DATA}）" /SD IDNO IDNO keep_data
  RMDir /r "${DATA}"
  keep_data:
SectionEnd
