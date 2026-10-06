param(
    [Parameter(Mandatory=$true)][int]$ProcId,
    [Parameter(Mandatory=$true)][string]$Cmd,
    [int]$X = 0,
    [int]$Y = 0,
    [int]$TimeoutMs = 60000
)
$env:LIB = $null
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class GW {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
}
"@
$deadline = (Get-Date).AddMilliseconds($TimeoutMs)
$ready = $false
while ((Get-Date) -lt $deadline) {
    $h = [GW]::GetForegroundWindow()
    $fgpid = [uint32]0
    if ($h -ne [IntPtr]::Zero) { [GW]::GetWindowThreadProcessId($h, [ref]$fgpid) | Out-Null }
    if ($fgpid -eq 0 -or $fgpid -eq $ProcId) { $ready = $true; break }
    Start-Sleep -Milliseconds 400
}
if (-not $ready) {
    Write-Output 'TIMEOUT: foreground still held by elevated process'
    exit 2
}
Write-Output ('foreground ready (pid=' + $fgpid + '), dispatching input')
& (Join-Path $PSScriptRoot 'drive.ps1') -ProcId $ProcId -Cmd $Cmd -X $X -Y $Y
