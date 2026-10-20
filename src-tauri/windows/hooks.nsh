; Moves the fconv CLI (bundled by Tauri next to the app exe) into <install>\cli and adds
; that folder to the user PATH. A separate folder keeps uninstall.exe and the bundled
; ffmpeg/yt-dlp off PATH.
; PATH is edited through the registry with DoNotExpandEnvironmentNames so existing
; %VAR% entries are preserved, then WM_SETTINGCHANGE is broadcast so newly opened
; terminals see the change.

!macro FCONV_PATH_SCRIPT ACTION
  nsExec::Exec `powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$$d='$INSTDIR\cli'; $$k=Get-Item 'HKCU:\Environment'; $$p=$$k.GetValue('Path','','DoNotExpandEnvironmentNames'); $$parts=@($$p -split ';' | Where-Object { $$_ -and $$_.TrimEnd('\') -ne $$d.TrimEnd('\') }); if ('${ACTION}' -eq 'add') { $$parts += $$d }; Set-ItemProperty -Path 'HKCU:\Environment' -Name 'Path' -Value ($$parts -join ';') -Type ExpandString"`
  Pop $0
  ; HWND_BROADCAST, WM_SETTINGCHANGE
  SendMessage 0xFFFF 0x1A 0 "STR:Environment" /TIMEOUT=500
!macroend

!macro NSIS_HOOK_POSTINSTALL
  CreateDirectory "$INSTDIR\cli"
  CopyFiles /SILENT "$INSTDIR\fconv.exe" "$INSTDIR\cli"
  Delete "$INSTDIR\fconv.exe"
  !insertmacro FCONV_PATH_SCRIPT "add"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro FCONV_PATH_SCRIPT "remove"
  Delete "$INSTDIR\cli\fconv.exe"
  RMDir "$INSTDIR\cli"
!macroend
