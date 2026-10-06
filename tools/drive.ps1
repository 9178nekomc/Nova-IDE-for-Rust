param(
    [Parameter(Mandatory=$true)][int]$ProcId,
    [Parameter(Mandatory=$true)][string]$Cmd,
    [string]$Arg1 = '',
    [int]$X = 0,
    [int]$Y = 0
)
$env:LIB = $null
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Drive {
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref P p);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint data, UIntPtr extra);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Rt, B; }
    [StructLayout(LayoutKind.Sequential)] public struct P { public int X, Y; }
}
"@
$proc = Get-Process -Id $ProcId -ErrorAction Stop
$hwnd = $proc.MainWindowHandle
if ($hwnd -eq [IntPtr]::Zero) { Write-Error 'no main window'; exit 1 }

function Send-Key([IntPtr]$h, [int]$vk, [int]$scan) {
    $down = [IntPtr]::new(1) # lParam: repeat=1
    [Drive]::PostMessage($h, 0x0100, [IntPtr]$vk, $down) | Out-Null
    Start-Sleep -Milliseconds 40
    $upLParam = [IntPtr]([Int64]0xC0000001 -bor ($scan -shl 16))
    [Drive]::PostMessage($h, 0x0101, [IntPtr]$vk, $upLParam) | Out-Null
}

switch ($Cmd) {
    'char' {
        foreach ($ch in $Arg1.ToCharArray()) {
            $code = [int]$ch
            [Drive]::PostMessage($hwnd, 0x0102, [IntPtr]$code, [IntPtr]1) | Out-Null
            Start-Sleep -Milliseconds 25
        }
        Write-Output ("typed: " + $Arg1)
    }
    'key' {
        $map = @{
            'enter'=0x0D; 'esc'=0x1B; 'tab'=0x09; 'bs'=0x08; 'del'=0x2E;
            'home'=0x24; 'end'=0x23; 'left'=0x25; 'up'=0x26; 'right'=0x27; 'down'=0x28;
            'space'=0x20; 'f5'=0x74
        }
        $name = $Arg1.ToLower()
        if ($map.ContainsKey($name)) {
            Send-Key $hwnd $map[$name] 0
            Write-Output ("key: " + $name)
        } elseif ($name.Length -eq 1) {
            $vk = [int][char]$name.ToUpper()
            Send-Key $hwnd $vk 0
            Write-Output ("key: " + $name)
        } else {
            Write-Error ("unknown key: " + $name)
        }
    }
    'combo' {
        # 例: combo ctrl+s
        $parts = $Arg1.ToLower().Split('+')
        foreach ($p in $parts) {
            if ($p -eq 'ctrl') { [Drive]::PostMessage($hwnd, 0x0100, [IntPtr]0x11, [IntPtr]1) | Out-Null }
            elseif ($p -eq 'shift') { [Drive]::PostMessage($hwnd, 0x0100, [IntPtr]0x10, [IntPtr]1) | Out-Null }
        }
        Start-Sleep -Milliseconds 40
        $last = $parts[-1]
        $vk = [int][char]$last.ToUpper()
        Send-Key $hwnd $vk 0
        Start-Sleep -Milliseconds 40
        foreach ($p in $parts) {
            if ($p -eq 'ctrl') { [Drive]::PostMessage($hwnd, 0x0101, [IntPtr]0x11, [IntPtr]0xC0000001) | Out-Null }
            elseif ($p -eq 'shift') { [Drive]::PostMessage($hwnd, 0x0101, [IntPtr]0x10, [IntPtr]0xC0000001) | Out-Null }
        }
        Write-Output ("combo: " + $Arg1)
    }
    'lclick' {
        $lp = [IntPtr](($Y -shl 16) -bor ($X -band 0xFFFF))
        [Drive]::PostMessage($hwnd, 0x0200, [IntPtr]0, $lp) | Out-Null
        Start-Sleep -Milliseconds 60
        [Drive]::PostMessage($hwnd, 0x0201, [IntPtr]1, $lp) | Out-Null
        Start-Sleep -Milliseconds 80
        [Drive]::PostMessage($hwnd, 0x0202, [IntPtr]0, $lp) | Out-Null
        Write-Output ("lclick: " + $X + "," + $Y)
    }
    'rclick' {
        $lp = [IntPtr](($Y -shl 16) -bor ($X -band 0xFFFF))
        [Drive]::PostMessage($hwnd, 0x0200, [IntPtr]0, $lp) | Out-Null
        Start-Sleep -Milliseconds 60
        [Drive]::PostMessage($hwnd, 0x0204, [IntPtr]2, $lp) | Out-Null
        Start-Sleep -Milliseconds 80
        [Drive]::PostMessage($hwnd, 0x0205, [IntPtr]0, $lp) | Out-Null
        Write-Output ("rclick: " + $X + "," + $Y)
    }
    'realclick' {
        # X,Y 为屏幕坐标；真实鼠标事件（前台窗口）
        [Drive]::SetCursorPos($X, $Y) | Out-Null
        Start-Sleep -Milliseconds 80
        [Drive]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)   # LEFTDOWN
        Start-Sleep -Milliseconds 70
        [Drive]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)   # LEFTUP
        Write-Output ("realclick: " + $X + "," + $Y)
    }
    'realdclick' {
        [Drive]::SetCursorPos($X, $Y) | Out-Null
        Start-Sleep -Milliseconds 80
        foreach ($f in 2, 4, 2, 4) {
            [Drive]::mouse_event($f, 0, 0, 0, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds 60
        }
        Write-Output ("realdclick: " + $X + "," + $Y)
    }
    'realrclick' {
        [Drive]::SetCursorPos($X, $Y) | Out-Null
        Start-Sleep -Milliseconds 80
        [Drive]::mouse_event(8, 0, 0, 0, [UIntPtr]::Zero)   # RIGHTDOWN
        Start-Sleep -Milliseconds 70
        [Drive]::mouse_event(16, 0, 0, 0, [UIntPtr]::Zero)  # RIGHTUP
        Write-Output ("realrclick: " + $X + "," + $Y)
    }
    'realmotion' {
        [Drive]::SetCursorPos($X, $Y) | Out-Null
        Write-Output ("motion: " + $X + "," + $Y)
    }
    'realkey' {
        $map = @{
            'enter'=0x0D; 'esc'=0x1B; 'tab'=0x09; 'bs'=0x08; 'del'=0x2E;
            'home'=0x24; 'end'=0x23; 'left'=0x25; 'up'=0x26; 'right'=0x27; 'down'=0x28;
            'space'=0x20; 'f5'=0x74
        }
        $name = $Arg1.ToLower()
        if ($map.ContainsKey($name)) {
            $vk = [byte]$map[$name]
        } else {
            $vk = [byte][int][char]$name.ToUpper()
        }
        [Drive]::keybd_event([byte]$vk, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 50
        [Drive]::keybd_event([byte]$vk, 0, 2, [UIntPtr]::Zero)
        Write-Output ("realkey: " + $name)
    }
    'realcombo' {
        $parts = $Arg1.ToLower().Split('+')
        foreach ($p in $parts) {
            if ($p -eq 'ctrl') { [Drive]::keybd_event(0x11, 0, 0, [UIntPtr]::Zero) }
            elseif ($p -eq 'shift') { [Drive]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero) }
        }
        Start-Sleep -Milliseconds 350
        $last = $parts[-1]
        $vk = [byte][int][char]$last.ToUpper()
        [Drive]::keybd_event($vk, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 80
        [Drive]::keybd_event($vk, 0, 2, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 350
        foreach ($p in $parts) {
            if ($p -eq 'ctrl') { [Drive]::keybd_event(0x11, 0, 2, [UIntPtr]::Zero) }
            elseif ($p -eq 'shift') { [Drive]::keybd_event(0x10, 0, 2, [UIntPtr]::Zero) }
        }
        Write-Output ("realcombo: " + $Arg1)
    }
    'realtype' {
        # KEYEVENTF_UNICODE(VK_PACKET) 注入，绕过 IME 组合
        foreach ($ch in $Arg1.ToCharArray()) {
            $code = [int]$ch
            [Drive]::keybd_event(0, [byte]($code -band 0xFF), 4, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds 15
            [Drive]::keybd_event(0, [byte]($code -band 0xFF), 6, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds 25
        }
        Write-Output ("realtype(unicode): " + $Arg1)
    }
    'origin' {
        $r = New-Object Drive+R
        [Drive]::GetWindowRect($hwnd, [ref]$r) | Out-Null
        $p = New-Object Drive+P
        $p.X = 0; $p.Y = 0
        [Drive]::ClientToScreen($hwnd, [ref]$p) | Out-Null
        Write-Output ("client_origin: " + $p.X + "," + $p.Y + " window: " + $r.L + "," + $r.T)
    }
    default { Write-Error ("unknown cmd: " + $Cmd) }
}
