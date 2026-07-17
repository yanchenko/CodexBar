using System;
using System.IO;
using System.Text.Json;
using Xunit;

namespace AgentBar.Tests;

/// <summary>
/// Snapshot schema v1 contract tests — C# DTOs must parse the same golden fixtures
/// as <c>ab-model</c> (<c>rust/crates/ab-model/tests/fixtures/</c>).
/// </summary>
public class SnapshotDtoTests
{
    private static string FixturePath(string name)
    {
        var candidates = new[]
        {
            Path.Combine(AppContext.BaseDirectory, "fixtures", name),
            Path.GetFullPath(Path.Combine(AppContext.BaseDirectory,
                "..", "..", "..", "..", "..", "..", "rust", "crates", "ab-model", "tests", "fixtures", name)),
        };
        foreach (var p in candidates)
        {
            if (File.Exists(p)) return p;
        }
        throw new FileNotFoundException($"Fixture not found: {name}. Looked in: {string.Join("; ", candidates)}");
    }

    private static string Load(string name) => File.ReadAllText(FixturePath(name));

    private static UsageSnapshot Parse(string json) => UsageSnapshot.FromJson(json);

    [Fact]
    public void SuccessCodexOmitsErrorParsesRateWindowAndCredits()
    {
        var raw = Load("success_codex.json");
        using (var doc = JsonDocument.Parse(raw))
        {
            var provider = doc.RootElement.GetProperty("providers")[0];
            Assert.False(provider.TryGetProperty("error", out _), "success fixture must omit error property");
            Assert.False(provider.TryGetProperty("errorCode", out _), "success fixture must omit errorCode property");
        }

        var snap = Parse(raw);
        Assert.Equal(1, snap.SchemaVersion);
        Assert.Equal(42UL, snap.Seq);
        Assert.False(snap.Refreshing);
        Assert.Single(snap.Providers);

        var p = snap.Providers[0];
        Assert.Equal("codex", p.Id);
        Assert.True(p.Enabled);
        Assert.Equal("oauth", p.SourceLabel);
        Assert.Null(p.Error);
        Assert.Null(p.ErrorCode);
        Assert.NotNull(p.Primary);
        Assert.Equal(42.5, p.Primary!.UsedPercent);
        Assert.Equal(300L, p.Primary.WindowMinutes);
        Assert.Equal("2026-07-17T17:00:00Z", p.Primary.ResetsAt);
        Assert.Equal("resets in 5h", p.Primary.ResetDescription);
        Assert.False(p.Primary.IsSyntheticPlaceholder);
        Assert.NotNull(p.Secondary);
        Assert.Equal(10.0, p.Secondary!.UsedPercent);
        Assert.Equal(10080L, p.Secondary.WindowMinutes);
        Assert.Equal(12.5, p.CreditsRemaining);
        Assert.Equal("user@example.com", p.AccountLabel);
        Assert.Equal("exact", p.DataConfidence);
        Assert.Null(p.CursorRequests);
    }

    [Fact]
    public void AllFixturesBanSecretKeys()
    {
        // Design security checklist: snapshot fixtures must never carry secret fields.
        string[] banned = ["apiKey", "cookieHeader", "accessToken", "refreshToken", "\"token\""];
        foreach (var name in new[]
                 {
                     "success_codex.json",
                     "failure_cursor_auth.json",
                     "rate_window_full.json",
                     "cursor_requests.json",
                 })
        {
            var raw = Load(name);
            foreach (var key in banned)
            {
                Assert.False(
                    raw.Contains(key, StringComparison.Ordinal),
                    $"fixture {name} must not contain banned key/substring '{key}'");
            }
        }
    }

    [Fact]
    public void FailureCursorIncludesStringErrorNoPrimary()
    {
        var raw = Load("failure_cursor_auth.json");
        using var doc = JsonDocument.Parse(raw);
        var err = doc.RootElement.GetProperty("providers")[0].GetProperty("error");
        Assert.Equal(JsonValueKind.String, err.ValueKind);
        Assert.NotEqual(JsonValueKind.Null, err.ValueKind);

        var snap = Parse(raw);
        var p = Assert.Single(snap.Providers);
        Assert.Equal("cursor", p.Id);
        Assert.Equal("Cursor cookie not configured (cookieSource=manual required)", p.Error);
        Assert.Equal("auth_missing", p.ErrorCode);
        Assert.Null(p.Primary);
        Assert.Null(p.CursorRequests);
    }

    [Fact]
    public void RateWindowFullAllFieldsAndExtraNamedWindows()
    {
        var snap = Parse(Load("rate_window_full.json"));
        var p = Assert.Single(snap.Providers);
        Assert.Equal("claude", p.Id);

        Assert.NotNull(p.Primary);
        Assert.Equal(88.0, p.Primary!.UsedPercent);
        Assert.Equal(300L, p.Primary.WindowMinutes);
        Assert.Equal("2026-07-17T14:00:00Z", p.Primary.ResetsAt);
        Assert.Equal("resets in 2h", p.Primary.ResetDescription);
        Assert.Equal(1.5, p.Primary.NextRegenPercent);
        Assert.True(p.Primary.IsSyntheticPlaceholder);

        Assert.NotNull(p.Secondary);
        Assert.Equal(50.0, p.Secondary!.UsedPercent);
        Assert.NotNull(p.Tertiary);
        Assert.Equal(5.0, p.Tertiary!.UsedPercent);

        Assert.NotNull(p.ExtraRateWindows);
        var extra = Assert.Single(p.ExtraRateWindows!);
        Assert.Equal("extra_session", extra.Id);
        Assert.Equal("Session", extra.Title);
        Assert.Equal(12.0, extra.Window.UsedPercent);
        Assert.Equal(60L, extra.Window.WindowMinutes);
        Assert.True(extra.UsageKnown);
    }

    [Fact]
    public void HostPatchJsonIsProvidersArrayMergeShape()
    {
        // Settings must only emit merge-patch objects with providers[] by id —
        // never a full typed config rewrite root. Shape mirrors Native.PatchProvider.
        using var stream = new System.IO.MemoryStream();
        using (var w = new Utf8JsonWriter(stream))
        {
            w.WriteStartObject();
            w.WritePropertyName("providers");
            w.WriteStartArray();
            w.WriteStartObject();
            w.WriteString("id", "codex");
            w.WriteBoolean("enabled", true);
            w.WriteString("apiKey", "secret-not-over-ffi");
            w.WriteEndObject();
            w.WriteEndArray();
            w.WriteEndObject();
        }
        var json = System.Text.Encoding.UTF8.GetString(stream.ToArray());
        using var doc = JsonDocument.Parse(json);
        Assert.Equal(JsonValueKind.Object, doc.RootElement.ValueKind);
        Assert.True(doc.RootElement.TryGetProperty("providers", out var providers));
        Assert.Equal(JsonValueKind.Array, providers.ValueKind);
        Assert.Equal("codex", providers[0].GetProperty("id").GetString());
        // Patch may contain secrets on disk; snapshot fixtures must never.
        Assert.True(json.Contains("apiKey", StringComparison.Ordinal));
        // Forbidden shapes for host save path.
        Assert.False(doc.RootElement.TryGetProperty("version", out _), "host patch must not rewrite whole tree version");
    }

    [Fact]
    public void CursorRequestsThinDto()
    {
        var snap = Parse(Load("cursor_requests.json"));
        var p = Assert.Single(snap.Providers);
        Assert.Equal("cursor", p.Id);
        Assert.Null(p.Error);
        Assert.NotNull(p.CursorRequests);
        Assert.Equal(500.0, p.CursorRequests!.Included);
        Assert.Equal(42.0, p.CursorRequests.Used);
        Assert.Equal(458.0, p.CursorRequests.Remaining);
        Assert.NotNull(p.Primary);
        Assert.Equal(8.4, p.Primary!.UsedPercent);
    }

    [Fact]
    public void EmptyOrMalformedYieldsEmptySnapshot()
    {
        Assert.Equal(0UL, Parse("").Seq);
        Assert.Equal(0UL, Parse("{}").Seq);
        Assert.Equal(0UL, Parse("not json").Seq);
        Assert.Empty(Parse("{}").Providers);
    }

    [Fact]
    public void TrayLinesTextOnlyNoProgressBarHints()
    {
        var ok = Parse(Load("success_codex.json"));
        var line = Assert.Single(ok.TrayLines());
        Assert.Contains("Codex", line, StringComparison.Ordinal);
        Assert.Contains("42.5%", line, StringComparison.Ordinal);
        Assert.DoesNotContain("ProgressBar", line, StringComparison.Ordinal);

        var fail = Parse(Load("failure_cursor_auth.json"));
        var failLine = Assert.Single(fail.TrayLines());
        Assert.Contains("Cursor", failLine, StringComparison.Ordinal);
        Assert.Contains("cookie", failLine, StringComparison.OrdinalIgnoreCase);
    }
}
