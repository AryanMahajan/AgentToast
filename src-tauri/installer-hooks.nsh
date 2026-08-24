; Installer hooks for AgentToast.
;
; NSIS cannot overwrite a running executable, and it does not treat that as an
; error: it skips the file and reports success. An update installed while the
; tray app was running therefore appeared to work while leaving the old binaries
; in place — the user ends up running the previous version with no indication
; anything went wrong.
;
; Closing the app before copying files is what makes an update actually apply.

!macro NSIS_HOOK_PREINSTALL
  DetailPrint "Closing AgentToast if it is running..."
  ; /T ends the tray app and the bridge processes it spawned.
  nsExec::Exec 'taskkill /F /T /IM agenttoast.exe'
  Pop $0
  nsExec::Exec 'taskkill /F /IM agenttoast-bridge-claude.exe'
  Pop $0
  ; Windows releases the file lock a moment after the process ends.
  Sleep 1200
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Closing AgentToast if it is running..."
  nsExec::Exec 'taskkill /F /T /IM agenttoast.exe'
  Pop $0
  nsExec::Exec 'taskkill /F /IM agenttoast-bridge-claude.exe'
  Pop $0
  Sleep 1200
!macroend
