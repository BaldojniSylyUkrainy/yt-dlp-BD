; One-time migration for the published 0.7.0.0 installer, whose productName was
; accidentally changed from the permanent "yt-dlp BD" identity to
; "Baldojnyi Downloader". Keep every guard exact: this hook must never adopt or
; remove an unrelated installation.

Var LegacyInstallDir
Var LegacyCleanupExitCode

!macro MigrateLegacyShortcut LEGACY_SHORTCUT CANONICAL_SHORTCUT
  !insertmacro IsShortcutTarget "${LEGACY_SHORTCUT}" "$LegacyInstallDir\${MAINBINARYNAME}.exe"
  Pop $R6
  ${If} $R6 = 1
    CreateShortcut "${CANONICAL_SHORTCUT}" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "${CANONICAL_SHORTCUT}"
    !insertmacro UnpinShortcut "${LEGACY_SHORTCUT}"
    Delete "${LEGACY_SHORTCUT}"
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  StrCpy $LegacyInstallDir ""

  ; This app has always shipped with NSIS currentUser mode. Accept only the
  ; exact registry identity, internal version, standard install path,
  ; uninstaller path, and main binary written by the 0.7.0.0 release.
  ReadRegStr $R0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Baldojnyi Downloader" "DisplayName"
  ReadRegStr $R1 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Baldojnyi Downloader" "DisplayVersion"
  ReadRegStr $R2 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Baldojnyi Downloader" "InstallLocation"
  ReadRegStr $R3 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Baldojnyi Downloader" "UninstallString"
  ReadRegStr $R4 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Baldojnyi Downloader" "MainBinaryName"

  ${If} $R0 == "Baldojnyi Downloader"
  ${AndIf} $R1 == "0.7.0"
  ${AndIf} $R2 == "$\"$LOCALAPPDATA\Baldojnyi Downloader$\""
  ${AndIf} $R3 == "$\"$LOCALAPPDATA\Baldojnyi Downloader\uninstall.exe$\""
  ${AndIf} $R4 == "yt-dlp-desktop.exe"
  ${AndIf} ${FileExists} "$LOCALAPPDATA\Baldojnyi Downloader\uninstall.exe"
  ${AndIf} ${FileExists} "$LOCALAPPDATA\Baldojnyi Downloader\yt-dlp-desktop.exe"
    StrCpy $LegacyInstallDir "$LOCALAPPDATA\Baldojnyi Downloader"
  ${EndIf}

  ; If 0.7.0.0 is the only installed identity, update that exact directory in
  ; place. The installer will then write the canonical yt-dlp BD registry state.
  ReadRegStr $R5 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\yt-dlp BD" "InstallLocation"
  ${If} $R5 == ""
  ${AndIf} $LegacyInstallDir != ""
    StrCpy $INSTDIR $LegacyInstallDir
    SetOutPath $INSTDIR
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} $LegacyInstallDir != ""
    ; Preserve whichever shortcuts the user actually had, but retarget them to
    ; the canonical installation and remove only the exact legacy shortcut.
    !insertmacro MigrateLegacyShortcut "$SMPROGRAMS\Baldojnyi Downloader.lnk" "$SMPROGRAMS\yt-dlp BD.lnk"
    !insertmacro MigrateLegacyShortcut "$DESKTOP\Baldojnyi Downloader.lnk" "$DESKTOP\yt-dlp BD.lnk"

    ; If both identities existed, the canonical install directory won. Run the
    ; exact legacy uninstaller in update mode after Tauri has stopped the app;
    ; update mode deliberately preserves shared app data. Never recursively
    ; delete a path ourselves.
    ${If} $LegacyInstallDir != $INSTDIR
      ExecWait '$\"$LegacyInstallDir\uninstall.exe$\" /UPDATE /P _?=$LegacyInstallDir' $LegacyCleanupExitCode
      ${If} $LegacyCleanupExitCode != 0
        Abort "Could not remove the exact Baldojnyi Downloader 0.7.0 installation."
      ${EndIf}
      ${If} ${FileExists} "$LegacyInstallDir\yt-dlp-desktop.exe"
        Abort "The exact Baldojnyi Downloader 0.7.0 installation is still present."
      ${EndIf}
    ${EndIf}

    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Baldojnyi Downloader"
    DeleteRegKey HKCU "Software\ytdlp\Baldojnyi Downloader"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Baldojnyi Downloader"
  ${EndIf}
!macroend
