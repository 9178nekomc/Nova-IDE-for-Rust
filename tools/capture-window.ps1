param(
    [Parameter(Mandatory=$true)][int]$ProcId,
    [Parameter(Mandatory=$true)][string]$OutFile,
    [int]$Frames = 1,
    [int]$IntervalMs = 0
)
Add-Type -AssemblyName System.Drawing
$env:LIB = $null
$env:INCLUDE = $null
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win32Cap {
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
[Win32Cap]::SetProcessDPIAware() | Out-Null
$proc = Get-Process -Id $ProcId -ErrorAction Stop
$hwnd = $proc.MainWindowHandle
if ($hwnd -eq [IntPtr]::Zero) { Write-Output 'no main window'; exit 1 }
for ($i = 0; $i -lt $Frames; $i++) {
    $rect = New-Object Win32Cap+RECT
    [Win32Cap]::GetWindowRect($hwnd, [ref]$rect) | Out-Null
    $w = $rect.Right - $rect.Left
    $h = $rect.Bottom - $rect.Top
    if ($w -le 0 -or $h -le 0) { Write-Output 'invalid window size'; exit 1 }
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    $ok = [Win32Cap]::PrintWindow($hwnd, $hdc, 2)
    $g.ReleaseHdc($hdc)
    $g.Dispose()
    $out = $OutFile -replace '\.png$', ("_{0:d2}.png" -f $i)
    $bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    if ($i -lt $Frames - 1) { Start-Sleep -Milliseconds $IntervalMs }
}
Write-Output ("captured {0} frame(s) -> {1}" -f $Frames, $OutFile)
