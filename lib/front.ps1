# Bring a Windows window to the front by title prefix (used by corral from WSL).
# Windows only lets the foreground app hand over focus, so this taps Alt first,
# the usual way to lift that lock, then restores and focuses the window.
#
# -Place right   un-maximize and put the window on the right side of the work area
#                (Width/Height are fractions of it). Used only for new windows.
# -DryRun        print what would happen without touching any window.
param(
    [string]$Prefix = "Mirador",
    [double]$WaitSeconds = 8,
    [switch]$FindOnly,
    [string]$Place = "",
    [double]$Width = 0.42,
    [double]$Height = 0.82,
    [switch]$DryRun
)

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class CorralWin {
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int n);
    [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] static extern bool IsZoomed(IntPtr h);
    [DllImport("user32.dll")] static extern bool MoveWindow(IntPtr h, int x, int y, int w, int ht, bool repaint);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool SystemParametersInfo(uint action, uint param, ref RECT rect, uint ini);
    [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    public static string LastTitle = "";
    public static IntPtr Find(string prefix) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            if (!IsWindowVisible(h)) return true;
            var sb = new StringBuilder(512);
            GetWindowText(h, sb, 512);
            var t = sb.ToString();
            if (t.StartsWith(prefix)) { found = h; LastTitle = t; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static RECT WorkArea() {
        var r = new RECT();
        SystemParametersInfo(0x0030, 0, ref r, 0);  // SPI_GETWORKAREA
        return r;
    }
    public static string Right(IntPtr h, double fw, double fh, bool dry) {
        var a = WorkArea();
        int aw = a.Right - a.Left, ah = a.Bottom - a.Top;
        int w = (int)(aw * fw), ht = (int)(ah * fh);
        int margin = (int)(aw * 0.012);
        int x = a.Right - w - margin, y = a.Top + (ah - ht) / 2;
        string plan = string.Format("{0}x{1}+{2}+{3} (area {4}x{5}, maximizada: {6})", w, ht, x, y, aw, ah, h == IntPtr.Zero ? false : IsZoomed(h));
        if (dry || h == IntPtr.Zero) return plan;
        ShowWindow(h, 9);  // SW_RESTORE: leave maximized or full-screen size
        MoveWindow(h, x, y, w, ht, true);
        return plan;
    }
    public static bool Raise(IntPtr h) {
        if (IsIconic(h)) ShowWindow(h, 9);
        keybd_event(0x12, 0, 0, UIntPtr.Zero);
        keybd_event(0x12, 0, 2, UIntPtr.Zero);
        return SetForegroundWindow(h);
    }
}
'@
[CorralWin]::SetProcessDPIAware() | Out-Null

$deadline = (Get-Date).AddSeconds($WaitSeconds)
do {
    $h = [CorralWin]::Find($Prefix)
    if ($h -ne [IntPtr]::Zero) { break }
    Start-Sleep -Milliseconds 200
} while ((Get-Date) -lt $deadline)

if ($DryRun) {
    $title = if ($h -ne [IntPtr]::Zero) { [CorralWin]::LastTitle } else { "(no encontrada)" }
    Write-Output ("ventana: " + $title + " | right: " + [CorralWin]::Right($h, $Width, $Height, $true))
    exit 0
}
if ($h -eq [IntPtr]::Zero) { Write-Output "not-found"; exit 1 }
if ($FindOnly) { Write-Output ("found: " + [CorralWin]::LastTitle); exit 0 }
if ($Place -eq "right") {
    # The page may still be loading; give the browser a moment to finish sizing the window.
    Start-Sleep -Milliseconds 400
    Write-Output ("placed: " + [CorralWin]::Right($h, $Width, $Height, $false))
}
Write-Output ("raised: " + [CorralWin]::Raise($h))
