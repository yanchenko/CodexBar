using Microsoft.UI.Xaml;

namespace AgentBar;

/// <summary>Settings placeholder — shows engine version/config path/snapshot summary.</summary>
public sealed partial class SettingsWindow : Window
{
    public SettingsWindow()
    {
        InitializeComponent();
        try
        {
            AppWindow.Resize(new Windows.Graphics.SizeInt32(480, 360));
        }
        catch
        {
            // Resize is best-effort on older runtimes.
        }
    }

    internal void ApplySnapshot(UsageSnapshot snap)
    {
        VersionText.Text = $"Version: {Native.Version()}";
        ConfigPathText.Text = $"Config: {Native.ConfigPath()}";
        EngineText.Text = $"Engine: {(Native.EngineRunning() ? "running" : "stopped")}" +
                          (snap.Refreshing ? " · refreshing" : "");
        var lines = snap.TrayLines();
        SnapshotText.Text = lines.Count == 0
            ? "(empty snapshot)"
            : string.Join("\n", lines);
    }
}
