using System;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace AgentBar;

/// <summary>
/// Fluent settings: Dashboard ProgressBars, provider toggles/secrets via temp JSON
/// patch + <c>ab_config_apply_patch_file</c> (never full typed rewrite).
/// </summary>
public sealed partial class SettingsWindow : Window
{
    public SettingsWindow()
    {
        InitializeComponent();
        try
        {
            AppWindow.Resize(new Windows.Graphics.SizeInt32(720, 560));
        }
        catch
        {
            // Resize is best-effort on older runtimes.
        }
        VersionText.Text = $"Version: {Native.Version()}";
        ConfigPathText.Text = $"Config: {Native.ConfigPath()}";
        LogDirText.Text = $"Logs: {Native.LogDir()}";
    }

    private void Nav_SelectionChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        var tag = (args.SelectedItem as NavigationViewItem)?.Tag as string ?? "dashboard";
        DashboardPage.Visibility = tag == "dashboard" ? Visibility.Visible : Visibility.Collapsed;
        ProvidersPage.Visibility = tag == "providers" ? Visibility.Visible : Visibility.Collapsed;
        GeneralPage.Visibility = tag == "general" ? Visibility.Visible : Visibility.Collapsed;
        AdvancedPage.Visibility = tag == "advanced" ? Visibility.Visible : Visibility.Collapsed;
    }

    internal void ApplySnapshot(UsageSnapshot snap)
    {
        EngineText.Text = $"Engine: {(Native.EngineRunning() ? "running" : "stopped")}" +
                          (snap.Refreshing ? " · refreshing" : "");
        DashMeta.Text = $"Updated {snap.UpdatedAt} · seq {snap.Seq}" +
                        (snap.Refreshing ? " · refreshing…" : "");
        RebuildDashboard(snap);
    }

    private void RebuildDashboard(UsageSnapshot snap)
    {
        DashProviders.Children.Clear();
        if (snap.Providers.Count == 0)
        {
            DashProviders.Children.Add(new TextBlock
            {
                Text = "No providers in snapshot. Enable providers under Providers.",
                Opacity = 0.75,
            });
            return;
        }

        foreach (var p in snap.Providers)
        {
            var card = new Border
            {
                Padding = new Thickness(16),
                CornerRadius = new CornerRadius(8),
                Background = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["CardBackgroundFillColorDefaultBrush"],
                BorderBrush = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["CardStrokeColorDefaultBrush"],
                BorderThickness = new Thickness(1),
            };
            var stack = new StackPanel { Spacing = 8 };
            var title = char.ToUpperInvariant(p.Id[0]) + (p.Id.Length > 1 ? p.Id[1..] : "");
            stack.Children.Add(new TextBlock
            {
                Text = title,
                Style = (Style)Application.Current.Resources["SubtitleTextBlockStyle"],
            });

            if (!string.IsNullOrEmpty(p.Error))
            {
                stack.Children.Add(new TextBlock
                {
                    Text = p.Error!,
                    Foreground = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["SystemFillColorCriticalBrush"],
                    TextWrapping = TextWrapping.Wrap,
                });
            }
            else if (p.Primary is { } w)
            {
                var pct = Math.Clamp(w.UsedPercent, 0, 100);
                stack.Children.Add(new TextBlock
                {
                    Text = string.Format(CultureInfo.InvariantCulture, "Primary · {0:0.#}% used", pct),
                });
                stack.Children.Add(new ProgressBar
                {
                    Minimum = 0,
                    Maximum = 100,
                    Value = pct,
                    Height = 8,
                });
                if (!string.IsNullOrEmpty(w.ResetDescription))
                {
                    stack.Children.Add(new TextBlock
                    {
                        Text = w.ResetDescription!,
                        Opacity = 0.75,
                    });
                }
                if (p.Secondary is { } s)
                {
                    var sp = Math.Clamp(s.UsedPercent, 0, 100);
                    stack.Children.Add(new TextBlock
                    {
                        Text = string.Format(CultureInfo.InvariantCulture, "Secondary · {0:0.#}%", sp),
                        Opacity = 0.85,
                    });
                    stack.Children.Add(new ProgressBar
                    {
                        Minimum = 0,
                        Maximum = 100,
                        Value = sp,
                        Height = 6,
                        Opacity = 0.85,
                    });
                }
                if (p.CreditsRemaining is { } c)
                {
                    stack.Children.Add(new TextBlock
                    {
                        Text = string.Format(CultureInfo.InvariantCulture, "Credits · ${0:0.##}", c),
                    });
                }
            }
            else if (p.CursorRequests is { } cr)
            {
                var used = cr.Used ?? 0;
                var incl = cr.Included ?? 0;
                var pct = incl > 0 ? Math.Clamp(100.0 * used / incl, 0, 100) : 0;
                stack.Children.Add(new TextBlock
                {
                    Text = string.Format(CultureInfo.InvariantCulture, "Requests · {0:0}/{1:0}", used, incl),
                });
                stack.Children.Add(new ProgressBar
                {
                    Minimum = 0,
                    Maximum = 100,
                    Value = pct,
                    Height = 8,
                });
            }
            else
            {
                stack.Children.Add(new TextBlock { Text = "No usage windows yet", Opacity = 0.75 });
            }

            if (!string.IsNullOrEmpty(p.SourceLabel))
            {
                stack.Children.Add(new TextBlock
                {
                    Text = $"Source: {p.SourceLabel}",
                    Opacity = 0.6,
                    FontSize = 12,
                });
            }

            card.Child = stack;
            DashProviders.Children.Add(card);
        }
    }

    private void Refresh_Click(object sender, RoutedEventArgs e)
    {
        Native.RefreshNow();
        ApplySnapshot(UsageSnapshot.Probe());
    }

    private void ProviderToggle_Toggled(object sender, RoutedEventArgs e)
    {
        if (sender is not ToggleSwitch ts || ts.Tag is not string id) return;
        var ok = Native.PatchProvider(id, enabled: ts.IsOn);
        ProvidersStatus.Text = ok
            ? $"Updated {id} enabled={ts.IsOn} via patch-file ABI."
            : $"Failed to patch {id}: {Native.LastErrorJson()}";
        if (ok) ApplySnapshot(UsageSnapshot.Probe());
    }

    private void SaveCodex_Click(object sender, RoutedEventArgs e)
    {
        var key = CodexApiKey.Password?.Trim();
        if (string.IsNullOrEmpty(key))
        {
            ProvidersStatus.Text = "Enter an API key first (or leave empty to only toggle).";
            return;
        }
        var ok = Native.PatchProvider("codex", enabled: CodexEnabled.IsOn, apiKey: key);
        ProvidersStatus.Text = ok ? "Codex credentials saved (merge-patch)." : $"Save failed: {Native.LastErrorJson()}";
        if (ok)
        {
            CodexApiKey.Password = "";
            ApplySnapshot(UsageSnapshot.Probe());
        }
    }

    private void SaveClaude_Click(object sender, RoutedEventArgs e)
    {
        var key = ClaudeApiKey.Password?.Trim();
        if (string.IsNullOrEmpty(key))
        {
            ProvidersStatus.Text = "Enter an API key first.";
            return;
        }
        var ok = Native.PatchProvider("claude", enabled: ClaudeEnabled.IsOn, apiKey: key);
        ProvidersStatus.Text = ok ? "Claude credentials saved (merge-patch)." : $"Save failed: {Native.LastErrorJson()}";
        if (ok)
        {
            ClaudeApiKey.Password = "";
            ApplySnapshot(UsageSnapshot.Probe());
        }
    }

    private void SaveCursor_Click(object sender, RoutedEventArgs e)
    {
        var cookie = CursorCookie.Text?.Trim();
        if (string.IsNullOrEmpty(cookie))
        {
            ProvidersStatus.Text = "Paste a Cursor cookie header first.";
            return;
        }
        var ok = Native.PatchProvider("cursor", enabled: CursorEnabled.IsOn, cookieHeader: cookie);
        ProvidersStatus.Text = ok ? "Cursor cookie saved (merge-patch)." : $"Save failed: {Native.LastErrorJson()}";
        if (ok)
        {
            CursorCookie.Text = "";
            ApplySnapshot(UsageSnapshot.Probe());
        }
    }

    private void RefreshCombo_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (RefreshCombo.SelectedItem is not ComboBoxItem item) return;
        var tag = item.Tag as string ?? "300";
        if (tag == "adaptive")
        {
            _ = Native.SetAdaptiveRefresh(true);
            // Keep a non-zero floor interval; adaptive policy owns sleep length.
            _ = Native.SetRefreshIntervalSecs(300);
        }
        else if (uint.TryParse(tag, out var secs))
        {
            _ = Native.SetAdaptiveRefresh(false);
            _ = Native.SetRefreshIntervalSecs(secs);
        }
    }

    private void RevealConfig_Click(object sender, RoutedEventArgs e)
    {
        var path = Native.ConfigPath();
        ConfigPathText.Text = $"Config: {path}";
        AdvancedStatus.Text = string.IsNullOrEmpty(path) ? "No sticky path resolved." : path;
        try
        {
            if (!string.IsNullOrEmpty(path))
            {
                var dir = Path.GetDirectoryName(path);
                if (!string.IsNullOrEmpty(dir) && Directory.Exists(dir))
                    Process.Start(new ProcessStartInfo("explorer.exe", $"/select,\"{path}\"") { UseShellExecute = true });
            }
        }
        catch (Exception ex)
        {
            AdvancedStatus.Text = $"Reveal failed: {ex.Message}";
        }
    }

    private void OpenLogs_Click(object sender, RoutedEventArgs e)
    {
        var dir = Native.LogDir();
        LogDirText.Text = $"Logs: {dir}";
        try
        {
            if (!string.IsNullOrEmpty(dir))
            {
                Directory.CreateDirectory(dir);
                Process.Start(new ProcessStartInfo("explorer.exe", dir) { UseShellExecute = true });
            }
        }
        catch (Exception ex)
        {
            AdvancedStatus.Text = $"Open logs failed: {ex.Message}";
        }
    }

    private void Migrate_Click(object sender, RoutedEventArgs e)
    {
        // Optional migrate via CLI merge path: write a no-op-safe note; full migrate is CLI `agentbar config migrate`.
        // Hosts do not get a dedicated migrate ABI in v1 — shell out is avoided; instruct user.
        AdvancedStatus.Text =
            "Use `agentbar config migrate` to copy sticky config to ~/.config/agentbar and rebind. " +
            "Current sticky: " + Native.ConfigPath();
    }
}
