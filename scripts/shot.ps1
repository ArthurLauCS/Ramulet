# Capture every visible window of class $Class in process $ProcessId to <Out>-<n>.png with PrintWindow
# (PW_RENDERFULLCONTENT): only that window's own pixels, works when it sits behind other windows on the desktop layer.
# Also prints each window's rect, owner and WS_EX_TOPMOST flag, to check z-order behavior.
param([int]$ProcessId, [string]$Out, [string]$Class = "RamuletMain")
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class Shot {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
    [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
    [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    public static List<IntPtr> Find(uint want, string cls) {
        var list = new List<IntPtr>();
        EnumWindows((h, l) => {
            var sb = new System.Text.StringBuilder(64);
            GetClassName(h, sb, 64);
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (sb.ToString() == cls && pid == want && IsWindowVisible(h)) list.Add(h);
            return true;
        }, IntPtr.Zero);
        return list;
    }
}
"@
[Shot]::SetProcessDPIAware() | Out-Null
$i = 0
foreach ($h in [Shot]::Find([uint32]$ProcessId, $Class)) {
    $r = New-Object Shot+RECT
    [Shot]::GetWindowRect($h, [ref]$r) | Out-Null
    $bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc()
    [Shot]::PrintWindow($h, $dc, 2) | Out-Null
    $g.ReleaseHdc($dc)
    $bmp.Save("$Out-$i.png"); $g.Dispose(); $bmp.Dispose()
    $owner = [Shot]::GetWindow($h, 4)
    $top = ([Shot]::GetWindowLong($h, -20) -band 8) -ne 0
    "$i hwnd=$h $($r.L),$($r.T) $($r.R - $r.L)x$($r.B - $r.T) owner=$owner topmost=$top"
    $i++
}
