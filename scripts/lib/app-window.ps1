# Dot-source: find the launcher's own top-level window by process id.
#
# Do not use Process.MainWindowHandle: it returns the first visible, unowned window of the
# process, and winit's hidden helper window ("Winit Thread Event Target") reports visible. While
# the real window is still hidden (settle frames), MainWindowHandle returns the helper, and a
# WM_CLOSE posted to it destroys it without the app ever hearing of it.
Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class AppWindow {
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr h, uint cmd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    public static string Title(IntPtr h) {
        var t = new StringBuilder(512);
        GetWindowText(h, t, 512);
        return t.ToString();
    }
    /// The visible, unowned top-level window of the process that is not winit's helper; or 0.
    public static IntPtr Find(int pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint p;
            GetWindowThreadProcessId(h, out p);
            if (p != (uint)pid || !IsWindowVisible(h) || GetWindow(h, 4) != IntPtr.Zero) return true;
            var c = new StringBuilder(256);
            GetClassName(h, c, 256);
            if (c.ToString() == "Winit Thread Event Target") return true;
            found = h;
            return false;
        }, IntPtr.Zero);
        return found;
    }
}
"@

function Wait-AppWindow([System.Diagnostics.Process]$Process, [int]$TimeoutSeconds = 10) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ($true) {
        $h = [AppWindow]::Find($Process.Id)
        if ($h -ne [IntPtr]::Zero) { return $h }
        if ($Process.HasExited) { throw "the process exited (code $($Process.ExitCode)) before showing a window" }
        if ((Get-Date) -gt $deadline) { throw "no window appeared" }
        Start-Sleep -Milliseconds 50
    }
}
