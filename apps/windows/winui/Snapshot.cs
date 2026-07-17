using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace AgentBar;

// Hand mirror of ab-model UsageSnapshot schema v1. No codegen — kept honest by
// shared JSON fixtures under rust/crates/ab-model/tests/fixtures/ + C# DTO tests.

/// <summary>Parsed usage snapshot (engine → host JSON). Never contains secrets.</summary>
internal sealed class UsageSnapshot
{
    public int SchemaVersion { get; init; }
    public ulong Seq { get; init; }
    public string UpdatedAt { get; init; } = "";
    public bool Refreshing { get; init; }
    public IReadOnlyList<ProviderSnap> Providers { get; init; } = Array.Empty<ProviderSnap>();

    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNameCaseInsensitive = true,
    };

    public static UsageSnapshot Empty { get; } = new();

    public static UsageSnapshot Probe() => FromJson(Native.SnapshotJson());

    public static UsageSnapshot FromJson(string json)
    {
        if (string.IsNullOrWhiteSpace(json) || json == "{}")
            return Empty;
        try
        {
            var dto = JsonSerializer.Deserialize<SnapshotDto>(json, JsonOptions);
            if (dto is null) return Empty;
            return new UsageSnapshot
            {
                SchemaVersion = dto.SchemaVersion,
                Seq = dto.Seq,
                UpdatedAt = dto.UpdatedAt ?? "",
                Refreshing = dto.Refreshing,
                Providers = (dto.Providers ?? Array.Empty<ProviderSnapDto>())
                    .Select(ProviderSnap.FromDto)
                    .ToArray(),
            };
        }
        catch
        {
            return Empty;
        }
    }

    /// <summary>One human-readable tray line per provider (text only — no progress bars).</summary>
    public IReadOnlyList<string> TrayLines()
    {
        if (Providers.Count == 0)
            return new[] { "No providers enabled" };

        var lines = new List<string>(Providers.Count);
        foreach (var p in Providers)
            lines.Add(p.FormatTrayLine());
        return lines;
    }
}

internal sealed class ProviderSnap
{
    public string Id { get; init; } = "";
    public bool Enabled { get; init; }
    public string? SourceLabel { get; init; }
    public string? UpdatedAt { get; init; }
    public string? Error { get; init; }
    public string? ErrorCode { get; init; }
    public RateWindow? Primary { get; init; }
    public RateWindow? Secondary { get; init; }
    public RateWindow? Tertiary { get; init; }
    public IReadOnlyList<NamedRateWindow>? ExtraRateWindows { get; init; }
    public double? CreditsRemaining { get; init; }
    public string? AccountLabel { get; init; }
    public string? DataConfidence { get; init; }
    public CursorRequests? CursorRequests { get; init; }

    internal static ProviderSnap FromDto(ProviderSnapDto d) => new()
    {
        Id = d.Id ?? "",
        Enabled = d.Enabled,
        SourceLabel = d.SourceLabel,
        UpdatedAt = d.UpdatedAt,
        Error = d.Error,
        ErrorCode = d.ErrorCode,
        Primary = RateWindow.FromDto(d.Primary),
        Secondary = RateWindow.FromDto(d.Secondary),
        Tertiary = RateWindow.FromDto(d.Tertiary),
        ExtraRateWindows = d.ExtraRateWindows?.Select(NamedRateWindow.FromDto).ToArray(),
        CreditsRemaining = d.CreditsRemaining,
        AccountLabel = d.AccountLabel,
        DataConfidence = d.DataConfidence,
        CursorRequests = CursorRequests.FromDto(d.CursorRequests),
    };

    /// <summary>Compact text row for the tray flyout (no ProgressBar).</summary>
    public string FormatTrayLine()
    {
        var inv = CultureInfo.InvariantCulture;
        var name = string.IsNullOrEmpty(Id) ? "?" : char.ToUpperInvariant(Id[0]) + Id[1..];
        if (!string.IsNullOrEmpty(Error))
            return $"{name}: {Error}";
        if (Primary is { } w)
        {
            var pct = string.Format(inv, "{0:0.#}%", w.UsedPercent);
            if (CreditsRemaining is { } c)
                return string.Format(inv, "{0}: {1} · ${2:0.##} left", name, pct, c);
            if (!string.IsNullOrEmpty(w.ResetDescription))
                return $"{name}: {pct} · {w.ResetDescription}";
            return $"{name}: {pct}";
        }
        if (CursorRequests is { Used: { } used, Included: { } incl })
            return string.Format(inv, "{0}: {1:0.#}/{2:0.#} req", name, used, incl);
        return $"{name}: —";
    }
}

internal sealed class RateWindow
{
    public double UsedPercent { get; init; }
    public long? WindowMinutes { get; init; }
    public string? ResetsAt { get; init; }
    public string? ResetDescription { get; init; }
    public double? NextRegenPercent { get; init; }
    public bool IsSyntheticPlaceholder { get; init; }

    internal static RateWindow? FromDto(RateWindowDto? d) =>
        d is null ? null : new RateWindow
        {
            UsedPercent = d.UsedPercent,
            WindowMinutes = d.WindowMinutes,
            ResetsAt = d.ResetsAt,
            ResetDescription = d.ResetDescription,
            NextRegenPercent = d.NextRegenPercent,
            IsSyntheticPlaceholder = d.IsSyntheticPlaceholder ?? false,
        };
}

internal sealed class NamedRateWindow
{
    public string Id { get; init; } = "";
    public string Title { get; init; } = "";
    public RateWindow Window { get; init; } = new();
    public bool UsageKnown { get; init; } = true;

    internal static NamedRateWindow FromDto(NamedRateWindowDto d) => new()
    {
        Id = d.Id ?? "",
        Title = d.Title ?? "",
        Window = RateWindow.FromDto(d.Window) ?? new RateWindow(),
        UsageKnown = d.UsageKnown ?? true,
    };
}

internal sealed class CursorRequests
{
    public double? Included { get; init; }
    public double? Used { get; init; }
    public double? Remaining { get; init; }

    internal static CursorRequests? FromDto(CursorRequestsDto? d) =>
        d is null ? null : new CursorRequests
        {
            Included = d.Included,
            Used = d.Used,
            Remaining = d.Remaining,
        };
}

// Wire DTOs (schema v1 camelCase) — public-ish for unit tests via InternalsVisibleTo.

internal sealed record SnapshotDto(
    [property: JsonPropertyName("schemaVersion")] int SchemaVersion,
    [property: JsonPropertyName("seq")] ulong Seq,
    [property: JsonPropertyName("updatedAt")] string? UpdatedAt,
    [property: JsonPropertyName("refreshing")] bool Refreshing,
    [property: JsonPropertyName("providers")] ProviderSnapDto[]? Providers);

internal sealed record ProviderSnapDto(
    [property: JsonPropertyName("id")] string? Id,
    [property: JsonPropertyName("enabled")] bool Enabled,
    [property: JsonPropertyName("sourceLabel")] string? SourceLabel,
    [property: JsonPropertyName("updatedAt")] string? UpdatedAt,
    [property: JsonPropertyName("error")] string? Error,
    [property: JsonPropertyName("errorCode")] string? ErrorCode,
    [property: JsonPropertyName("primary")] RateWindowDto? Primary,
    [property: JsonPropertyName("secondary")] RateWindowDto? Secondary,
    [property: JsonPropertyName("tertiary")] RateWindowDto? Tertiary,
    [property: JsonPropertyName("extraRateWindows")] NamedRateWindowDto[]? ExtraRateWindows,
    [property: JsonPropertyName("creditsRemaining")] double? CreditsRemaining,
    [property: JsonPropertyName("accountLabel")] string? AccountLabel,
    [property: JsonPropertyName("dataConfidence")] string? DataConfidence,
    [property: JsonPropertyName("cursorRequests")] CursorRequestsDto? CursorRequests);

internal sealed record CursorRequestsDto(
    [property: JsonPropertyName("included")] double? Included,
    [property: JsonPropertyName("used")] double? Used,
    [property: JsonPropertyName("remaining")] double? Remaining);

internal sealed record RateWindowDto(
    [property: JsonPropertyName("usedPercent")] double UsedPercent,
    [property: JsonPropertyName("windowMinutes")] long? WindowMinutes,
    [property: JsonPropertyName("resetsAt")] string? ResetsAt,
    [property: JsonPropertyName("resetDescription")] string? ResetDescription,
    [property: JsonPropertyName("nextRegenPercent")] double? NextRegenPercent,
    [property: JsonPropertyName("isSyntheticPlaceholder")] bool? IsSyntheticPlaceholder);

internal sealed record NamedRateWindowDto(
    [property: JsonPropertyName("id")] string? Id,
    [property: JsonPropertyName("title")] string? Title,
    [property: JsonPropertyName("window")] RateWindowDto? Window,
    [property: JsonPropertyName("usageKnown")] bool? UsageKnown);
