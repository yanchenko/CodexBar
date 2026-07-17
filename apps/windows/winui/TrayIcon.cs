using System;
using System.Collections.Generic;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Windows.Input;
using Microsoft.UI.Xaml.Controls;

namespace AgentBar;

/// <summary>
/// Tray icon via H.NotifyIcon — text-only <see cref="MenuFlyout"/> rows for providers
/// (no ProgressBars in the tray flyout; rich UI belongs in Settings).
/// </summary>
internal sealed class TrayIcon : IDisposable
{
    public event Action? OpenSettings;
    public event Action? Exit;
    public event Action? RefreshRequested;

    private readonly H.NotifyIcon.TaskbarIcon _icon;
    private readonly IntPtr _hicon;
    private readonly Icon _stateIcon;
    private MenuFlyout? _flyout;
    private readonly List<MenuFlyoutItem> _providerItems = new();
    private bool _disposed;

    public TrayIcon()
    {
        _hicon = MakeSolidIcon(16, Color.FromArgb(255, 37, 99, 235)); // blue "A" bar color
        _stateIcon = Icon.FromHandle(_hicon);

        _flyout = BuildMenu(Array.Empty<string>());
        _icon = new H.NotifyIcon.TaskbarIcon
        {
            ToolTipText = "AgentBar",
            ContextMenuMode = H.NotifyIcon.ContextMenuMode.SecondWindow,
            NoLeftClickDelay = true,
            ContextFlyout = _flyout,
        };
        _icon.LeftClickCommand = new RelayCommand(() => OpenSettings?.Invoke());
        _icon.UpdateIcon(_stateIcon);
        _icon.ForceCreate();
    }

    private MenuFlyout BuildMenu(IReadOnlyList<string> providerLines)
    {
        var flyout = new MenuFlyout();
        _providerItems.Clear();

        if (providerLines.Count == 0)
        {
            var empty = new MenuFlyoutItem { Text = "No usage data yet", IsEnabled = false };
            flyout.Items.Add(empty);
            _providerItems.Add(empty);
        }
        else
        {
            foreach (var line in providerLines)
            {
                var item = new MenuFlyoutItem { Text = line, IsEnabled = false };
                flyout.Items.Add(item);
                _providerItems.Add(item);
            }
        }

        flyout.Items.Add(new MenuFlyoutSeparator());

        var refresh = new MenuFlyoutItem { Text = "Refresh" };
        refresh.Click += (_, _) => RefreshRequested?.Invoke();
        flyout.Items.Add(refresh);

        var settings = new MenuFlyoutItem { Text = "Settings…" };
        settings.Click += (_, _) => OpenSettings?.Invoke();
        flyout.Items.Add(settings);

        flyout.Items.Add(new MenuFlyoutSeparator());

        var exit = new MenuFlyoutItem { Text = "Exit" };
        exit.Click += (_, _) => Exit?.Invoke();
        flyout.Items.Add(exit);

        flyout.Opening += (_, _) => Native.NoteMenuOpened();
        return flyout;
    }

    /// <summary>Replace provider text rows from a snapshot (UI thread only).</summary>
    public void UpdateSnapshot(UsageSnapshot snap)
    {
        if (_disposed) return;
        var lines = snap.TrayLines();
        var tip = lines.Count > 0 ? string.Join("\n", lines) : "AgentBar";
        if (tip.Length > 120) tip = tip[..117] + "…";
        _icon.ToolTipText = tip;

        // Mutate existing rows when membership count is unchanged to avoid flyout flicker
        // while open; rebuild only when provider row count changes.
        if (_flyout != null && _providerItems.Count == Math.Max(lines.Count, 1)
            && lines.Count > 0 && _providerItems.Count == lines.Count)
        {
            for (var i = 0; i < lines.Count; i++)
                _providerItems[i].Text = lines[i];
            return;
        }

        var next = BuildMenu(lines);
        _icon.ContextFlyout = next;
        _flyout = next;
    }

    public void Balloon(string title, string body)
    {
        if (_disposed) return;
        _icon.ShowNotification(title, body);
    }

    /// <summary>Premultiplied HICON solid circle (simple brand mark until real assets land).</summary>
    private static IntPtr MakeSolidIcon(int size, Color color)
    {
        int W = size, H = size;
        var bmi = new BITMAPINFO
        {
            bmiHeader = new BITMAPINFOHEADER
            {
                biSize = (uint)Marshal.SizeOf<BITMAPINFOHEADER>(),
                biWidth = W,
                biHeight = -H,
                biPlanes = 1,
                biBitCount = 32,
                biCompression = 0,
            },
        };

        IntPtr hdc = Win32.GetDC(IntPtr.Zero);
        IntPtr colorBmp = Win32.CreateDIBSection(hdc, ref bmi, 0, out IntPtr bits, IntPtr.Zero, 0);
        _ = Win32.ReleaseDC(IntPtr.Zero, hdc);
        if (colorBmp == IntPtr.Zero) return LoadIconW(IntPtr.Zero, IDI_APPLICATION);

        var buf = new byte[W * H * 4];
        double cx = (W - 1) / 2.0, cy = (H - 1) / 2.0, r = Math.Min(cx, cy) - 0.5;
        for (int y = 0; y < H; y++)
        {
            for (int x = 0; x < W; x++)
            {
                double dx = x - cx, dy = y - cy;
                int i = (y * W + x) * 4;
                if (dx * dx + dy * dy <= r * r)
                {
                    // Premultiplied BGRA
                    buf[i + 0] = color.B;
                    buf[i + 1] = color.G;
                    buf[i + 2] = color.R;
                    buf[i + 3] = 255;
                }
            }
        }
        Marshal.Copy(buf, 0, bits, buf.Length);

        int maskStride = (W + 15) / 16 * 2;
        var mask = new byte[maskStride * H];
        IntPtr hbmMask = CreateBitmap(W, H, 1, 1, mask);
        var ii = new ICONINFO { fIcon = true, hbmMask = hbmMask, hbmColor = colorBmp };
        IntPtr icon = CreateIconIndirect(ref ii);
        Win32.DeleteObject(colorBmp);
        DeleteObject(hbmMask);
        return icon != IntPtr.Zero ? icon : LoadIconW(IntPtr.Zero, IDI_APPLICATION);
    }

    public void Dispose()
    {
        if (_disposed) return;
        _disposed = true;
        _icon.Dispose();
        _stateIcon.Dispose();
        if (_hicon != IntPtr.Zero) DestroyIcon(_hicon);
    }

    private static readonly IntPtr IDI_APPLICATION = (IntPtr)32512;

    [StructLayout(LayoutKind.Sequential)]
    private struct ICONINFO
    {
        [MarshalAs(UnmanagedType.Bool)] public bool fIcon;
        public int xHotspot;
        public int yHotspot;
        public IntPtr hbmMask;
        public IntPtr hbmColor;
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr LoadIconW(IntPtr hinst, IntPtr name);

    [DllImport("user32.dll")]
    private static extern bool DestroyIcon(IntPtr icon);

    [DllImport("user32.dll")]
    private static extern IntPtr CreateIconIndirect(ref ICONINFO ii);

    [DllImport("gdi32.dll")]
    private static extern IntPtr CreateBitmap(int w, int h, uint planes, uint bitCount, byte[] bits);

    [DllImport("gdi32.dll")]
    private static extern bool DeleteObject(IntPtr hObject);

    private sealed class RelayCommand : ICommand
    {
        private readonly Action _run;
        public RelayCommand(Action run) => _run = run;
        public event EventHandler? CanExecuteChanged { add { } remove { } }
        public bool CanExecute(object? parameter) => true;
        public void Execute(object? parameter) => _run();
    }
}
