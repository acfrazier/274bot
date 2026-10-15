' Windows nightly host: start nightly-task.ps1 without a console window on the
' BotTest desktop (the same launcher pattern as the native scenario driver) and
' wait for it, so the scheduled task stays Running for the whole night.
' Usage (scheduled task action): wscript.exe nightly-launch.vbs night|catch-up
Dim shell, here, mode
Set shell = CreateObject("WScript.Shell")
here = CreateObject("Scripting.FileSystemObject").GetParentFolderName(WScript.ScriptFullName)
mode = "night"
If WScript.Arguments.Count > 0 Then mode = WScript.Arguments(0)
WScript.Quit shell.Run("powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File """ & _
    here & "\nightly-task.ps1"" -Mode " & mode, 0, True)
