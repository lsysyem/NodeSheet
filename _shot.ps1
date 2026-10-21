param(
    [string]$Out = "shot.png",
    [string]$File = "",
    [int]$Style = 0,
    [int]$Theme = 2,
    [switch]$Menu,
    [switch]$Keep,
    [int]$ClickX = 0,
    [int]$ClickY = 0,
    [int]$Click2X = 0,
    [int]$Click2Y = 0
)

Add-Type -AssemblyName System.Drawing
$src = @"
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
public class W {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool ScreenToClient(IntPtr h, ref POINT p);
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, int flags);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    static W() { SetProcessDPIAware(); }

    static List<IntPtr> found = new List<IntPtr>();
    static uint wantPid;
    static bool Collect(IntPtr h, IntPtr l) {
        uint pid = 0;
        GetWindowThreadProcessId(h, out pid);
        if (pid == wantPid && IsWindowVisible(h)) found.Add(h);
        return true;
    }
    public static string[] CaptureProcess(uint pid, string prefix) {
        found.Clear(); wantPid = pid;
        EnumWindows(Collect, IntPtr.Zero);
        List<string> outp = new List<string>();
        int i = 0;
        foreach (IntPtr h in found) {
            RECT r;
            if (!GetWindowRect(h, out r)) continue;
            int w = r.R - r.L, t = r.B - r.T;
            if (w < 20 || t < 20) continue;
            StringBuilder sb = new StringBuilder(128);
            GetClassNameW(h, sb, 128);
            string path = prefix + "-win" + i + "-" + sb.ToString() + ".png";
            Bitmap b = new Bitmap(w, t);
            Graphics g = Graphics.FromImage(b);
            IntPtr dc = g.GetHdc();
            PrintWindow(h, dc, 2);
            g.ReleaseHdc(dc);
            g.Dispose();
            b.Save(path, ImageFormat.Png);
            b.Dispose();
            outp.Add(path + " " + w + "x" + t + " @" + r.L + "," + r.T);
            i++;
        }
        return outp.ToArray();
    }
    public static void SaveShot(IntPtr h, string path) {
        RECT r; GetWindowRect(h, out r);
        Bitmap b = new Bitmap(r.R - r.L, r.B - r.T);
        Graphics g = Graphics.FromImage(b);
        IntPtr dc = g.GetHdc();
        PrintWindow(h, dc, 2);
        g.ReleaseHdc(dc);
        g.Dispose();
        b.Save(path, ImageFormat.Png);
        b.Dispose();
    }
    public static void Click(IntPtr h, int x, int y) {
        IntPtr lp = (IntPtr)((y << 16) | (x & 0xffff));
        PostMessageW(h, 0x0200, IntPtr.Zero, lp);
        System.Threading.Thread.Sleep(80);
        PostMessageW(h, 0x0201, (IntPtr)1, lp);
        System.Threading.Thread.Sleep(80);
        PostMessageW(h, 0x0202, IntPtr.Zero, lp);
    }
    // click at coordinates in the full-window capture (see SaveShot)
    public static void ClickWindow(IntPtr h, int wx, int wy) {
        RECT wr; GetWindowRect(h, out wr);
        POINT p; p.X = wr.L + wx; p.Y = wr.T + wy;
        ScreenToClient(h, ref p);
        Click(h, p.X, p.Y);
    }
}
"@
Add-Type -TypeDefinition $src -ReferencedAssemblies "System.dll","System.Core.dll","System.Drawing.dll"

$root = $PSScriptRoot
$dir = Join-Path $env:TEMP ("ns-shot-" + $Style + "-" + $Theme)
Remove-Item $dir -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $dir | Out-Null
$settings = Join-Path $dir "settings.json"
[System.IO.File]::WriteAllText($settings, ('{"ui_style":' + $Style + ',"theme":' + $Theme + ',"window":{"width":1500,"height":950,"maximized":false}}'))

$exe = Join-Path $root "target\release\nodesheet.exe"
$env:RTV_SETTINGS = $settings
$env:RTV_NODES = $dir
$argv = @()
if ($File -ne "") { $argv += (Join-Path $root $File) }
$p = Start-Process $exe -ArgumentList $argv -PassThru

# winit: MainWindowHandle may be the event window right after start; retry
$h = [IntPtr]::Zero
for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 250
    $p.Refresh()
    $h = $p.MainWindowHandle
    if ($h -ne [IntPtr]::Zero) {
        $r = New-Object W+RECT
        if ([W]::GetClientRect($h, [ref]$r) -and ($r.R - $r.L) -gt 200) { break }
    }
}
if ($h -eq [IntPtr]::Zero) { Write-Error "window not found"; exit 1 }

[W]::ShowWindow($h, 9) | Out-Null
[W]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 900

[W]::SaveShot($h, $Out)
Write-Output ("dpi " + [W]::GetDpiForWindow($h))

if ($Menu) {
    # "settings" menu title in the menu bar (client physical px; window is on a 144dpi screen)
    if ($ClickX -gt 0) { [W]::ClickWindow($h, $ClickX, $ClickY) } else { [W]::ClickWindow($h, 258, 66) }
    Start-Sleep -Milliseconds 900
    if ($Click2X -gt 0) { [W]::ClickWindow($h, $Click2X, $Click2Y); Start-Sleep -Milliseconds 900 }
    $base = $Out.Substring(0, $Out.Length - 4)
    [W]::SaveShot($h, $base + "-after.png")
    foreach ($line in [W]::CaptureProcess($p.Id, $base)) { Write-Output $line }
}

if (-not $Keep) {
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    Remove-Item $dir -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Output ("ok " + $Out)
