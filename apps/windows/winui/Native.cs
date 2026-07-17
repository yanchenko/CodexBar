using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;

namespace AgentBar;

/// <summary>
/// P/Invoke to <c>ab_core.dll</c> — same C ABI as <c>agentbar.h</c>. Hosts the engine
/// in-process: start/stop + snapshot JSON (never secrets). Config mutation is path-only
/// via <see cref="ApplyPatchFile"/>.
/// </summary>
internal static class Native
{
    private const string Dll = "ab_core.dll";

    [DllImport(Dll)] private static extern byte ab_engine_start();
    [DllImport(Dll)] private static extern byte ab_engine_stop();
    [DllImport(Dll)] private static extern byte ab_engine_running();
    [DllImport(Dll)] private static extern byte ab_engine_reload();
    [DllImport(Dll)] private static extern byte ab_refresh_now();
    [DllImport(Dll)] private static extern byte ab_set_refresh_interval_secs(uint secs);
    [DllImport(Dll)] private static extern byte ab_set_adaptive_refresh(byte on);
    [DllImport(Dll)] private static extern void ab_note_menu_opened();
    [DllImport(Dll)] private static extern byte ab_set_host_signals_json(IntPtr utf8Json);
    [DllImport(Dll)] private static extern IntPtr ab_snapshot_json();
    [DllImport(Dll)] private static extern IntPtr ab_snapshot_wait(ulong sinceSeq, uint timeoutMs);
    [DllImport(Dll)] private static extern IntPtr ab_version();
    [DllImport(Dll)] private static extern IntPtr ab_last_error_json();
    [DllImport(Dll)] private static extern IntPtr ab_config_path();
    [DllImport(Dll)] private static extern IntPtr ab_log_dir();
    [DllImport(Dll)] private static extern IntPtr ab_providers_catalog_json();
    [DllImport(Dll)] private static extern byte ab_config_apply_patch_file(IntPtr absolutePathUtf8);
    [DllImport(Dll)] private static extern void ab_string_free(IntPtr s);

    public static bool EngineStart() => ab_engine_start() != 0;
    public static bool EngineStop() => ab_engine_stop() != 0;
    public static bool EngineRunning() => ab_engine_running() != 0;
    public static bool EngineReload() => ab_engine_reload() != 0;
    public static bool RefreshNow() => ab_refresh_now() != 0;
    public static bool SetRefreshIntervalSecs(uint secs) => ab_set_refresh_interval_secs(secs) != 0;
    public static bool SetAdaptiveRefresh(bool on) => ab_set_adaptive_refresh(on ? (byte)1 : (byte)0) != 0;
    public static void NoteMenuOpened() => ab_note_menu_opened();

    public static string SnapshotJson() => TakeString(ab_snapshot_json());

    /// <summary>BLOCKS until seq ≠ <paramref name="sinceSeq"/> or timeout. Background thread only.</summary>
    public static string SnapshotWait(ulong sinceSeq, uint timeoutMs) =>
        TakeString(ab_snapshot_wait(sinceSeq, timeoutMs));

    public static string Version() => _version ??= TakeString(ab_version());
    private static string? _version;

    public static string LastErrorJson() => TakeString(ab_last_error_json());
    public static string ConfigPath() => TakeString(ab_config_path());
    public static string LogDir() => TakeString(ab_log_dir());
    public static string ProvidersCatalogJson() => TakeString(ab_providers_catalog_json());

    /// <summary>
    /// Apply a merge-patch JSON document by writing a temp file and calling
    /// <c>ab_config_apply_patch_file</c>. Never full-file typed rewrite.
    /// </summary>
    public static bool ApplyConfigPatchJson(string patchJson)
    {
        if (string.IsNullOrWhiteSpace(patchJson)) return false;
        var dir = Path.Combine(Path.GetTempPath(), "AgentBar");
        Directory.CreateDirectory(dir);
        var path = Path.Combine(dir, $"patch-{Guid.NewGuid():N}.json");
        try
        {
            // Restrictive ACL best-effort is left to OS temp defaults; file deleted after apply.
            File.WriteAllText(path, patchJson, new UTF8Encoding(encoderShouldEmitUTF8Identifier: false));
            var full = Path.GetFullPath(path);
            return ApplyPatchFile(full);
        }
        finally
        {
            try { File.Delete(path); } catch { /* best-effort */ }
        }
    }

    /// <summary>Absolute path to host-written JSON patch; Rust merge-patches sticky config.</summary>
    public static bool ApplyPatchFile(string absolutePath)
    {
        if (string.IsNullOrWhiteSpace(absolutePath)) return false;
        var bytes = Encoding.UTF8.GetBytes(absolutePath + "\0");
        var ptr = Marshal.AllocHGlobal(bytes.Length);
        try
        {
            Marshal.Copy(bytes, 0, ptr, bytes.Length);
            return ab_config_apply_patch_file(ptr) != 0;
        }
        finally
        {
            Marshal.FreeHGlobal(ptr);
        }
    }

    /// <summary>Build a providers[] merge-patch for one provider id and apply.</summary>
    public static bool PatchProvider(
        string id,
        bool? enabled = null,
        string? apiKey = null,
        string? cookieHeader = null,
        string? sourceMode = null)
    {
        using var stream = new MemoryStream();
        using (var w = new Utf8JsonWriter(stream))
        {
            w.WriteStartObject();
            w.WritePropertyName("providers");
            w.WriteStartArray();
            w.WriteStartObject();
            w.WriteString("id", id);
            if (enabled is { } e) w.WriteBoolean("enabled", e);
            if (apiKey is { } k) w.WriteString("apiKey", k);
            if (cookieHeader is { } c) w.WriteString("cookieHeader", c);
            if (sourceMode is { } m) w.WriteString("sourceMode", m);
            w.WriteEndObject();
            w.WriteEndArray();
            w.WriteEndObject();
        }
        var json = Encoding.UTF8.GetString(stream.ToArray());
        if (!ApplyConfigPatchJson(json)) return false;
        return EngineReload();
    }

    /// <summary>Marshal Rust UTF-8 char* and free.</summary>
    internal static string TakeString(IntPtr ptr)
    {
        if (ptr == IntPtr.Zero) return "";
        try { return Marshal.PtrToStringUTF8(ptr) ?? ""; }
        finally { ab_string_free(ptr); }
    }
}
