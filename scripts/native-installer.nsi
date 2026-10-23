Unicode True
!include "MUI2.nsh"
!include "FileFunc.nsh"
Name "Rebase Native"
OutFile "@OUTFILE@"
InstallDir "$LOCALAPPDATA\Programs\Rebase Native"
InstallDirRegKey HKCU "Software\Rebase Native" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
VIProductVersion "@WINDOWS_VERSION@"
VIAddVersionKey "ProductName" "Rebase Native"
VIAddVersionKey "FileDescription" "Rebase Native preview installer"
VIAddVersionKey "FileVersion" "@VERSION@"
!define MUI_ICON "@ICON@"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
Var UpdatePID
Var HadPrevious
Function .onInit
  ${GetParameters} $R0
  ${GetOptions} $R0 "/UPDATEPID=" $UpdatePID
  StrCmp $UpdatePID "" ready
  System::Call 'kernel32::OpenProcess(i 0x00100000, i 0, i $UpdatePID) p .r1'
  StrCmp $1 0 ready
  System::Call 'kernel32::WaitForSingleObject(p r1, i 120000) i .r2'
  System::Call 'kernel32::CloseHandle(p r1)'
  StrCmp $2 0 ready
  Abort "Rebase Native did not close. Please try updating again."
  ready:
FunctionEnd
Section "Rebase Native"
  SetOutPath "$INSTDIR"
  ClearErrors
  File /oname=rebase-gpui.next.exe "@STAGE@\rebase-gpui.exe"
  IfErrors install_failed
  StrCpy $HadPrevious 0
  IfFileExists "$INSTDIR\rebase-gpui.exe" 0 replace_binary
  Delete "$INSTDIR\rebase-gpui.previous.exe"
  ClearErrors
  Rename "$INSTDIR\rebase-gpui.exe" "$INSTDIR\rebase-gpui.previous.exe"
  IfErrors install_failed
  StrCpy $HadPrevious 1
  replace_binary:
  ClearErrors
  Rename "$INSTDIR\rebase-gpui.next.exe" "$INSTDIR\rebase-gpui.exe"
  IfErrors rollback_binary
  File "@STAGE@\native-install.json"
  File /r "@STAGE@\licenses"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Rebase Native" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RebaseNative" "DisplayName" "Rebase Native"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RebaseNative" "DisplayVersion" "@VERSION@"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RebaseNative" "UninstallString" '$"$INSTDIR\Uninstall.exe$"'
  CreateShortcut "$SMPROGRAMS\Rebase Native.lnk" "$INSTDIR\rebase-gpui.exe"
  CreateShortcut "$DESKTOP\Rebase Native.lnk" "$INSTDIR\rebase-gpui.exe"
  Delete "$INSTDIR\rebase-gpui.previous.exe"
  Goto install_done
  rollback_binary:
  StrCmp $HadPrevious 1 0 install_failed
  Rename "$INSTDIR\rebase-gpui.previous.exe" "$INSTDIR\rebase-gpui.exe"
  install_failed:
  Delete "$INSTDIR\rebase-gpui.next.exe"
  MessageBox MB_ICONSTOP "Rebase Native could not be updated. The previous executable was retained. Close other Rebase Native processes and try again."
  Abort
  install_done:
SectionEnd
Function .onInstFailed
  StrCmp $UpdatePID "" failed_done
  IfFileExists "$INSTDIR\rebase-gpui.exe" 0 failed_done
  Exec '$"$INSTDIR\rebase-gpui.exe$"'
  failed_done:
FunctionEnd
Function .onInstSuccess
  StrCmp $UpdatePID "" done
  Exec '$"$INSTDIR\rebase-gpui.exe$"'
  done:
FunctionEnd
Section "Uninstall"
  Delete "$SMPROGRAMS\Rebase Native.lnk"
  Delete "$DESKTOP\Rebase Native.lnk"
  Delete "$INSTDIR\rebase-gpui.exe"
  Delete "$INSTDIR\native-install.json"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir /r "$INSTDIR\licenses"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Rebase Native"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\RebaseNative"
SectionEnd
