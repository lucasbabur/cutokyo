!macro NSIS_HOOK_PREINSTALL
  ; Automated artifact tests install into a disposable /D= directory. Never
  ; let a later interactive upgrade inherit that temporary registry location.
  ClearErrors
  ${GetOptions} $CMDLINE "/D=" $0
  ${If} ${Errors}
    StrCpy $INSTDIR "$LOCALAPPDATA\Cutokyo"
    SetOutPath $INSTDIR
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Installing the Cutokyo terminal command..."
  ExecWait '"$INSTDIR\cutokyo.exe" --install-cli' $0
  ${If} $0 != 0
    DetailPrint "Cutokyo CLI PATH setup failed with exit code $0"
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Restoring Cutokyo network and harness configuration..."
  ClearErrors
  StrCpy $2 0
  System::Call 'kernel32::SetEnvironmentVariable(t "CUTOKYO_RECOVERY_REPORT_PATH", t "$INSTDIR\cutokyo-recovery-report.txt")'
  ExecWait '"$INSTDIR\cutokyo.exe" --emergency-restore' $0
  System::Call 'kernel32::SetEnvironmentVariable(t "CUTOKYO_RECOVERY_REPORT_PATH", t "")'
  ${If} ${Errors}
    StrCpy $2 1
  ${EndIf}
  DetailPrint "Stopping active Cutokyo processes..."
  nsExec::ExecToLog '"$SYSDIR\taskkill.exe" /F /T /IM "cutokyo.exe"'
  Pop $1
  Sleep 500
  ${If} $2 = 1
    DetailPrint "Cutokyo emergency restore could not be started; uninstall aborted."
    SetErrorLevel 10
    Abort
  ${EndIf}
  ${If} $0 != 0
    DetailPrint "Cutokyo emergency restore failed with exit code $0; uninstall aborted."
    SetErrorLevel $0
    Abort
  ${EndIf}
  DetailPrint "Removing the Cutokyo terminal command..."
  ExecWait '"$INSTDIR\cutokyo.exe" --uninstall-cli' $1
  Delete "$INSTDIR\cutokyo-recovery-report.txt"
!macroend
