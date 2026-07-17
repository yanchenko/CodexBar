using System;
using System.Globalization;
using System.Linq;
using System.Runtime.InteropServices;
using System.Threading;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Windows.System.Power;

namespace AgentBar;

/// <summary>
/// Process entry: in-process engine host + tray. Starts <c>ab_core</c>, shows
/// provider snapshot lines in a text MenuFlyout, Settings placeholder, Exit.
/// </summary>
[System.Diagnostics.CodeAnalysis.SuppressMessage(
    "Design", "CA1001:Types that own disposable fields should be disposable",
    Justification = "App lives for the whole process; tray is torn down by ExitApp.")]
public partial class App : Application
{
    private TrayIcon? _tray;
    private SettingsWindow? _settings;
    private bool _exiting;
    private bool _hostingEngine;
    private Thread? _pushThread;
    private Thread? _hostSignalsThread;
    private volatile bool _pushStop;
    private readonly object _statusDispatchLock = new();
    private UsageSnapshot? _pendingStatus;
    private bool _statusDispatchQueued;
    private static Mutex? _instanceMutex;
    private static EventWaitHandle? _activate;
    private const string ActivateEvent = "AgentBar.WinUI.Activate";
    private const string SingleInstanceMutex = "AgentBar.WinUI.SingleInstance";
    private const string AppUserModelId = "AgentBar";

    public App()
    {
        try { _ = Win32.SetCurrentProcessExplicitAppUserModelID(AppUserModelId); }
        catch { /* best-effort */ }
        InitializeComponent();
    }

    /// <summary>
    /// Hardcoded English load-failure dialog — NOT via engine strings (DLL may be missing).
    /// </summary>
    private static string DllLoadFailureMessage() =>
        CultureInfo.CurrentUICulture.TwoLetterISOLanguageName switch
        {
            _ => "ab_core.dll was not found next to the app, so the AgentBar engine cannot start.\n\n" +
                 "Build the Rust engine first:\n" +
                 "  cd rust && cargo build --profile release-ffi -p ab-core\n" +
                 "Then rebuild the WinUI host (see apps/windows/README.md).",
        };

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        if (!NativeLibrary.TryLoad("ab_core.dll", out _))
        {
            _ = Win32.MessageBoxW(IntPtr.Zero, DllLoadFailureMessage(),
                "AgentBar", Win32.MB_OK | Win32.MB_ICONERROR);
            Exit();
            return;
        }

        // Single instance: second launch signals the first to show Settings, then exits.
        _instanceMutex = new Mutex(true, SingleInstanceMutex, out bool createdNew);
        if (!createdNew)
        {
            if (EventWaitHandle.TryOpenExisting(ActivateEvent, out var ev))
            {
                ev.Set();
                ev.Dispose();
            }
            Exit();
            return;
        }
        _activate = new EventWaitHandle(false, EventResetMode.AutoReset, ActivateEvent);

        var cli = Environment.GetCommandLineArgs();
        bool hidden = cli.Any(a =>
            a.Equals("--hidden", StringComparison.OrdinalIgnoreCase) ||
            a.Equals("--tray", StringComparison.OrdinalIgnoreCase));

        _hostingEngine = true;
        if (!Native.EngineStart())
        {
            _ = Win32.MessageBoxW(IntPtr.Zero,
                "AgentBar engine failed to start. Check logs under %APPDATA%\\AgentBar\\logs.",
                "AgentBar", Win32.MB_OK | Win32.MB_ICONERROR);
            Exit();
            return;
        }

        _tray = new TrayIcon();
        _tray.OpenSettings += () => ShowSettings();
        _tray.Exit += ExitApp;
        _tray.RefreshRequested += () =>
        {
            Native.RefreshNow();
            ApplySnapshot(UsageSnapshot.Probe());
        };

        // One-shot paint before push thread.
        ApplySnapshot(UsageSnapshot.Probe());
        PushHostPowerSignals();
        StartHostSignalsPoll();

        if (!hidden)
            ShowSettings();
        else
            _tray.Balloon("AgentBar", "Running in the system tray.");

        StartSnapshotPush();

        // Single-instance reactivation.
        var uiq = DispatcherQueue.GetForCurrentThread();
        new Thread(() =>
        {
            try
            {
                while (!_exiting)
                {
                    _activate!.WaitOne();
                    if (_exiting) break;
                    uiq.TryEnqueue(() => { if (!_exiting) ShowSettings(); });
                }
            }
            catch { /* handle disposed during teardown */ }
        })
        { IsBackground = true, Name = "activate" }.Start();
    }

    private void ShowSettings()
    {
        if (_settings == null)
        {
            _settings = new SettingsWindow();
            _settings.AppWindow.Closing += (_, e) =>
            {
                if (_exiting) return;
                e.Cancel = true;
                _settings!.AppWindow.Hide();
            };
        }
        var snap = UsageSnapshot.Probe();
        _settings.ApplySnapshot(snap);
        // Re-bind toggles each time Settings is shown (CLI/other hosts may have patched).
        _settings.HydrateProviderToggles(snap);
        _settings.AppWindow.Show();
        var hwnd = WinRT.Interop.WindowNative.GetWindowHandle(_settings);
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
    }

    private const int SW_RESTORE = 9;

    [DllImport("user32.dll")]
    private static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);

    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(IntPtr hWnd);

    private void ApplySnapshot(UsageSnapshot s)
    {
        _tray?.UpdateSnapshot(s);
        _settings?.ApplySnapshot(s);
    }

    private void QueueLatestSnapshot(DispatcherQueue queue, UsageSnapshot snapshot)
    {
        lock (_statusDispatchLock)
        {
            _pendingStatus = snapshot;
            if (_statusDispatchQueued) return;
            _statusDispatchQueued = true;
        }
        if (!queue.TryEnqueue(DrainLatestSnapshot))
        {
            lock (_statusDispatchLock)
            {
                _statusDispatchQueued = false;
                _pendingStatus = null;
            }
        }
    }

    private void DrainLatestSnapshot()
    {
        while (true)
        {
            UsageSnapshot? s;
            lock (_statusDispatchLock)
            {
                s = _pendingStatus;
                _pendingStatus = null;
                if (s == null)
                {
                    _statusDispatchQueued = false;
                    return;
                }
            }
            ApplySnapshot(s);
        }
    }

    /// <summary>Background thread blocks in ab_snapshot_wait, marshals on change.</summary>
    private void StartSnapshotPush()
    {
        var uiQueue = DispatcherQueue.GetForCurrentThread();
        _pushThread = new Thread(() =>
        {
            ulong since = 0;
            bool delivered = false;
            while (!_pushStop)
            {
                string json;
                try { json = Native.SnapshotWait(since, 1000); }
                catch { Thread.Sleep(500); continue; }
                if (_pushStop) break;
                if (string.IsNullOrWhiteSpace(json) || json == "{}")
                {
                    Thread.Sleep(400);
                    continue;
                }
                var s = UsageSnapshot.FromJson(json);
                bool changed = !delivered || s.Seq != since;
                since = s.Seq;
                if (!changed) continue;
                delivered = true;
                QueueLatestSnapshot(uiQueue, s);
            }
        })
        { IsBackground = true, Name = "snapshot-push" };
        _pushThread.Start();
    }

    /// <summary>
    /// Feed WinRT power state into adaptive refresh (<c>ab_set_host_signals_json</c>).
    /// Low power when Energy Saver is on or discharging with &lt;20% remaining.
    /// Thermal is not exposed as a simple WinRT app signal — left false.
    /// </summary>
    private static void PushHostPowerSignals()
    {
        try
        {
            bool energySaver = PowerManager.EnergySaverStatus == EnergySaverStatus.On;
            bool lowBattery = PowerManager.BatteryStatus == BatteryStatus.Discharging
                              && PowerManager.RemainingChargePercent is >= 0 and < 20;
            bool lowPower = energySaver || lowBattery;
            var json = lowPower
                ? "{\"lowPower\":true,\"thermalSerious\":false}"
                : "{\"lowPower\":false,\"thermalSerious\":false}";
            _ = Native.SetHostSignalsJson(json);
        }
        catch
        {
            // PowerManager may be unavailable in some sandbox/VM contexts — degrade gracefully.
        }
    }

    private void StartHostSignalsPoll()
    {
        _hostSignalsThread = new Thread(() =>
        {
            while (!_pushStop)
            {
                PushHostPowerSignals();
                // Coarse poll; adaptive sleep re-plans on change via plan_epoch.
                for (var i = 0; i < 60 && !_pushStop; i++)
                    Thread.Sleep(1000);
            }
        })
        { IsBackground = true, Name = "host-signals" };
        _hostSignalsThread.Start();
    }

    private void ExitApp()
    {
        if (_exiting) return;
        _exiting = true;
        _pushStop = true;
        _activate?.Set();
        _tray?.Dispose();
        if (_hostingEngine)
        {
            var stop = System.Threading.Tasks.Task.Run(Native.EngineStop);
            stop.Wait(TimeSpan.FromSeconds(5));
        }
        // Join push thread so a hung native wait does not race process teardown.
        if (_pushThread is { IsAlive: true })
            _ = _pushThread.Join(TimeSpan.FromSeconds(2));
        if (_hostSignalsThread is { IsAlive: true })
            _ = _hostSignalsThread.Join(TimeSpan.FromSeconds(2));
        _activate?.Dispose();
        _activate = null;
        _instanceMutex?.Dispose();
        _instanceMutex = null;
        _settings?.Close();
        Exit();
    }
}
