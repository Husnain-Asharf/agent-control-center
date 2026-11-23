; Hooks for the NSIS setup.exe, set in tauri.conf.json as
; bundle.windows.nsis.installerHooks.
;
; The daemon, accd.exe, keeps running after the app's window closes, and
; Windows cannot replace or remove the files of a running program. So the
; installer and the uninstaller end it first, and its sessions end with it.
; An upgrade then also starts the new daemon, not the old one.

; Ends accd.exe. taskkill exits with 0 when it ended one, and with another
; code when none runs or one cannot be ended, which is no error here. It does
; not wait for the process to exit, so it runs again until none is left, for
; at most 5 seconds.
!macro ACC_STOP_DAEMON
  Push $R0
  Push $R1
  StrCpy $R1 0
  ${Do}
    nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM accd.exe'
    Pop $R0
    ${If} $R0 != 0
      ${ExitDo}
    ${EndIf}
    Sleep 100
    IntOp $R1 $R1 + 1
  ${LoopUntil} $R1 >= 50
  Pop $R1
  Pop $R0
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro ACC_STOP_DAEMON
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro ACC_STOP_DAEMON
!macroend
