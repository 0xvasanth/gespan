Unicode True
!include "MUI2.nsh"
!include "FileFunc.nsh"
Name "Gespan Native"
OutFile "@OUTFILE@"
InstallDir "$LOCALAPPDATA\Programs\Gespan Native"
InstallDirRegKey HKCU "Software\Gespan Native" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
VIProductVersion "@WINDOWS_VERSION@"
VIAddVersionKey "ProductName" "Gespan Native"
VIAddVersionKey "FileDescription" "Gespan Native installer"
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
  Abort "Gespan Native did not close. Please try updating again."
  ready:
FunctionEnd
Section "Gespan Native"
  SetOutPath "$INSTDIR"
  ClearErrors
  File /oname=gespan-gpui.next.exe "@STAGE@\gespan-gpui.exe"
  IfErrors install_failed
  StrCpy $HadPrevious 0
  IfFileExists "$INSTDIR\gespan-gpui.exe" 0 replace_binary
  Delete "$INSTDIR\gespan-gpui.previous.exe"
  ClearErrors
  Rename "$INSTDIR\gespan-gpui.exe" "$INSTDIR\gespan-gpui.previous.exe"
  IfErrors install_failed
  StrCpy $HadPrevious 1
  replace_binary:
  ClearErrors
  Rename "$INSTDIR\gespan-gpui.next.exe" "$INSTDIR\gespan-gpui.exe"
  IfErrors rollback_binary
  File "@STAGE@\native-install.json"
  File /r "@STAGE@\licenses"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Gespan Native" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\GespanNative" "DisplayName" "Gespan Native"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\GespanNative" "DisplayVersion" "@VERSION@"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\GespanNative" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  CreateShortcut "$SMPROGRAMS\Gespan Native.lnk" "$INSTDIR\gespan-gpui.exe"
  CreateShortcut "$DESKTOP\Gespan Native.lnk" "$INSTDIR\gespan-gpui.exe"
  Delete "$INSTDIR\gespan-gpui.previous.exe"
  Goto install_done
  rollback_binary:
  StrCmp $HadPrevious 1 0 install_failed
  Rename "$INSTDIR\gespan-gpui.previous.exe" "$INSTDIR\gespan-gpui.exe"
  install_failed:
  Delete "$INSTDIR\gespan-gpui.next.exe"
  MessageBox MB_ICONSTOP "Gespan Native could not be updated. The previous executable was retained. Close other Gespan Native processes and try again."
  Abort
  install_done:
SectionEnd
Function .onInstFailed
  StrCmp $UpdatePID "" failed_done
  IfFileExists "$INSTDIR\gespan-gpui.exe" 0 failed_done
  Exec '"$INSTDIR\gespan-gpui.exe"'
  failed_done:
FunctionEnd
Function .onInstSuccess
  StrCmp $UpdatePID "" done
  Exec '"$INSTDIR\gespan-gpui.exe"'
  done:
FunctionEnd
Section "Uninstall"
  Delete "$SMPROGRAMS\Gespan Native.lnk"
  Delete "$DESKTOP\Gespan Native.lnk"
  Delete "$INSTDIR\gespan-gpui.exe"
  Delete "$INSTDIR\native-install.json"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir /r "$INSTDIR\licenses"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Gespan Native"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\GespanNative"
SectionEnd
