Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public class DDTest {
  [DllImport("user32.dll")]
  public static extern bool PostMessageW(IntPtr hWnd, uint msg, IntPtr wp, IntPtr lp);
  [DllImport("kernel32.dll")]
  public static extern IntPtr GlobalAlloc(uint flags, UIntPtr bytes);
  [DllImport("kernel32.dll")]
  public static extern IntPtr GlobalLock(IntPtr h);
  [DllImport("kernel32.dll")]
  public static extern bool GlobalUnlock(IntPtr h);
}
"@

$proc = Get-Process nodesheet -ErrorAction Stop
$hwnd = $proc.MainWindowHandle
if ($hwnd -eq [IntPtr]::Zero) { Write-Host "no window"; exit 1 }
Write-Host ("hwnd: " + $hwnd)

$path = $null
# Build path without non-ASCII issues: use the 8.3 short name of the (Chinese-named) folder
# Try short path via Scripting.FileSystemObject
$fso = New-Object -ComObject Scripting.FileSystemObject
$folder = $fso.GetFolder((Get-Location).Path)
$shortPath = $folder.ShortPath
Write-Host ("short cwd: " + $shortPath)
$full = $shortPath + "\testdata\sales.xlsx"
Write-Host ("drop path: " + $full)

$bytes = [System.Text.Encoding]::Unicode.GetByteCount($full) + 2 + 22
$h = [DDTest]::GlobalAlloc(2, [UIntPtr][UInt64]$bytes)
$p = [DDTest]::GlobalLock($h)
$buf = New-Object byte[] $bytes
[BitConverter]::GetBytes([UInt32]20).CopyTo($buf, 0)
[BitConverter]::GetBytes([UInt32]1).CopyTo($buf, 16)
[System.Text.Encoding]::Unicode.GetBytes($full).CopyTo($buf, 20)
[System.Runtime.InteropServices.Marshal]::Copy($buf, 0, $p, $bytes)
[DDTest]::GlobalUnlock($h) | Out-Null
$ok = [DDTest]::PostMessageW($hwnd, 0x233, $h, [IntPtr]::Zero)
Write-Host ("PostMessage: " + $ok)
