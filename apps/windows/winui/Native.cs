using System;
using System.Runtime.InteropServices;

namespace AgentBar;

/// <summary>
/// P/Invoke to <c>ab_core.dll</c> — same C ABI as <c>agentbar.h</c>. Hosts the engine
/// in-process: start/stop + snapshot JSON (never secrets).
/// </summary>
internal static class Native
{
    private const string Dll = "ab_core.dll";

    [DllImport(Dll)] private static extern byte ab_engine_start();
    [DllImport(Dll)] private static extern byte ab_engine_stop();
    [DllImport(Dll)] private static extern byte ab_engine_running();
    [DllImport(Dll)] private static extern byte ab_engine_reload();
    [DllImport(Dll)] private static extern byte ab_refresh_now();
    [DllImport(Dll)] private static extern void ab_note_menu_opened();
    [DllImport(Dll)] private static extern IntPtr ab_snapshot_json();
    [DllImport(Dll)] private static extern IntPtr ab_snapshot_wait(ulong sinceSeq, uint timeoutMs);
    [DllImport(Dll)] private static extern IntPtr ab_version();
    [DllImport(Dll)] private static extern IntPtr ab_last_error_json();
    [DllImport(Dll)] private static extern IntPtr ab_config_path();
    [DllImport(Dll)] private static extern IntPtr ab_providers_catalog_json();
    [DllImport(Dll)] private static extern void ab_string_free(IntPtr s);

    public static bool EngineStart() => ab_engine_start() != 0;
    public static bool EngineStop() => ab_engine_stop() != 0;
    public static bool EngineRunning() => ab_engine_running() != 0;
    public static bool EngineReload() => ab_engine_reload() != 0;
    public static bool RefreshNow() => ab_refresh_now() != 0;
    public static void NoteMenuOpened() => ab_note_menu_opened();

    public static string SnapshotJson() => TakeString(ab_snapshot_json());

    /// <summary>BLOCKS until seq ≠ <paramref name="sinceSeq"/> or timeout. Background thread only.</summary>
    public static string SnapshotWait(ulong sinceSeq, uint timeoutMs) =>
        TakeString(ab_snapshot_wait(sinceSeq, timeoutMs));

    public static string Version() => _version ??= TakeString(ab_version());
    private static string? _version;

    public static string LastErrorJson() => TakeString(ab_last_error_json());
    public static string ConfigPath() => TakeString(ab_config_path());
    public static string ProvidersCatalogJson() => TakeString(ab_providers_catalog_json());

    /// <summary>Marshal Rust UTF-8 char* and free.</summary>
    internal static string TakeString(IntPtr ptr)
    {
        if (ptr == IntPtr.Zero) return "";
        try { return Marshal.PtrToStringUTF8(ptr) ?? ""; }
        finally { ab_string_free(ptr); }
    }
}
