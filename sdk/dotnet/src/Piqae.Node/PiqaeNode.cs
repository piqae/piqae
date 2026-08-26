using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Piqae.Node;

public enum HostMode { MachineService, UserAgent, EmbeddedApplication, AttachedClient }
public enum AvailabilityClass { ContinuousWhileAwake, ForegroundOnly, BackgroundOpportunistic, ManagedKiosk, WakeRelayCapable }
public enum LifecycleEvent { Started, EnteredForeground, EnteredBackground, SuspendImminent, Sleeping, Woke, NetworkAvailable, NetworkConstrained, NetworkUnavailable, ShutdownRequested }

public sealed record PiqaeNodeOptions(
    HostMode HostMode,
    AvailabilityClass Availability,
    bool LocalOnly,
    string ApplicationId,
    string DataDirectory);

public sealed record AdapterFingerprint(
    string Platform,
    string AdapterId,
    string AdapterVersion,
    string? DeviceFamily = null,
    string? FirmwareVersion = null);

public sealed record AdapterRegistration(AdapterFingerprint Fingerprint, JsonElement CapabilityContract);

public sealed record PrinterObservation(
    string NativeId,
    string Name,
    string State,
    bool IsDefault,
    JsonElement NativeOptions);

public sealed record PrinterProfileInput(string Name, bool IsDefault, string OptionsJson);

public enum AdapterOperationOutcomeKind
{
    RejectedBeforeHandoff,
    Accepted,
    CompletedReported,
    FailedTerminal,
    Ambiguous
}

public sealed class PiqaeNode : IDisposable
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower) }
    };

    private readonly object _gate = new();
    private ulong _handle;
    private bool _disposed;
    private GCHandle? _hostKeyHandle;
    private HmacCallback? _hostKeyCallback;

    public PiqaeNode(PiqaeNodeOptions options)
    {
        ArgumentNullException.ThrowIfNull(options);
        var descriptor = NativeMethods.piqae_node_abi_descriptor();
        if (descriptor.AbiVersion != 1 || descriptor.ContractMin > 1 || descriptor.ContractMax < 1)
            throw new PiqaeNodeException("unsupported_native_abi", "The native Piqae runtime ABI is not compatible with this SDK.");
        var payload = JsonSerializer.SerializeToUtf8Bytes(new
        {
            contract = 1,
            host_mode = options.HostMode,
            availability = options.Availability,
            local_only = options.LocalOnly,
            application_id = options.ApplicationId,
            data_directory = options.DataDirectory
        }, JsonOptions);
        using var response = NativeResponse.Call(payload, NativeMethods.piqae_node_create);
        _handle = response.Data.GetProperty("handle").GetUInt64();
    }

    public JsonElement Start() => CallHandle(NativeMethods.piqae_node_start);
    public JsonElement Stop() => CallHandle(NativeMethods.piqae_node_stop);
    public JsonElement Snapshot() => CallHandle(NativeMethods.piqae_node_snapshot);

    public JsonElement ApplyLifecycle(LifecycleEvent lifecycleEvent)
    {
        ThrowIfDisposed();
        var payload = JsonSerializer.SerializeToUtf8Bytes(new
        {
            type = "apply_lifecycle",
            @event = lifecycleEvent
        }, JsonOptions);
        using var response = NativeResponse.Call(_handle, payload, NativeMethods.piqae_node_command);
        return response.Data.Clone();
    }

    public JsonElement RegisterAdapter(AdapterRegistration registration) => Command(new
    {
        type = "register_adapter",
        registration
    });

    public JsonElement ObservePrinterInventory(string adapterId, IReadOnlyList<PrinterObservation> printers) => Command(new
    {
        type = "observe_printer_inventory",
        adapter_id = adapterId,
        printers
    });

    public JsonElement PrinterInventory() => Command(new { type = "printer_inventory" });

    public JsonElement EnqueueLocalJob(
        string adapterId,
        string idempotencyKey,
        string printerId,
        string title,
        string contentKind,
        ReadOnlySpan<byte> content,
        string optionsJson = "{}",
        long? expiresUnixMs = null) => Command(new
    {
        type = "enqueue_local_job",
        adapter_id = adapterId,
        idempotency_key = idempotencyKey,
        printer_id = printerId,
        title,
        content_kind = contentKind,
        content_base64 = Convert.ToBase64String(content),
        options_json = optionsJson,
        expires_unix_ms = expiresUnixMs
    });

    public JsonElement NextAdapterOperation(string adapterId) => Command(new
    {
        type = "next_adapter_operation",
        adapter_id = adapterId
    });

    public JsonElement BeginAdapterHandoff(string adapterId, string operationId, string fence) => Command(new
    {
        type = "begin_adapter_handoff",
        adapter_id = adapterId,
        operation_id = operationId,
        fence
    });

    public JsonElement CompleteAdapterOperation(
        string adapterId,
        string operationId,
        string fence,
        AdapterOperationOutcomeKind outcome,
        string? nativeJobId = null,
        string? code = null,
        bool retryable = false)
    {
        var result = outcome switch
        {
            AdapterOperationOutcomeKind.RejectedBeforeHandoff => new Dictionary<string, object?>
            { ["outcome"] = "rejected_before_handoff", ["code"] = code, ["retryable"] = retryable },
            AdapterOperationOutcomeKind.Accepted => new Dictionary<string, object?>
            { ["outcome"] = "accepted", ["native_job_id"] = nativeJobId },
            AdapterOperationOutcomeKind.CompletedReported => new Dictionary<string, object?>
            { ["outcome"] = "completed_reported", ["native_job_id"] = nativeJobId },
            AdapterOperationOutcomeKind.FailedTerminal => new Dictionary<string, object?>
            { ["outcome"] = "failed_terminal", ["native_job_id"] = nativeJobId, ["code"] = code },
            AdapterOperationOutcomeKind.Ambiguous => new Dictionary<string, object?>
            { ["outcome"] = "ambiguous", ["code"] = code },
            _ => throw new ArgumentOutOfRangeException(nameof(outcome))
        };
        return Command(new
        {
            type = "complete_adapter_operation",
            adapter_id = adapterId,
            operation_id = operationId,
            fence,
            result
        });
    }

    public JsonElement ProfileSnapshots(string printerId) => Command(new
    {
        type = "profile_snapshots",
        printer_id = printerId
    });

    public JsonElement CreateProfile(string printerId, PrinterProfileInput profile) => Command(new
    {
        type = "create_profile",
        printer_id = printerId,
        name = profile.Name,
        is_default = profile.IsDefault,
        options_json = profile.OptionsJson
    });

    public JsonElement UpdateProfile(
        string printerId,
        string profileId,
        ulong expectedRevision,
        PrinterProfileInput profile) => Command(new
    {
        type = "update_profile",
        printer_id = printerId,
        profile_id = profileId,
        expected_revision = expectedRevision,
        name = profile.Name,
        is_default = profile.IsDefault,
        options_json = profile.OptionsJson
    });

    public JsonElement DeleteProfile(string printerId, string profileId, ulong expectedRevision) => Command(new
    {
        type = "delete_profile",
        printer_id = printerId,
        profile_id = profileId,
        expected_revision = expectedRevision
    });

    public JsonElement ConnectorSnapshots() => Command(new { type = "connector_snapshots" });

    public JsonElement RevokeConnector(string connectorId) => Command(new
    {
        type = "revoke_connector",
        connector_id = connectorId
    });

    public void ConfigureHostKeyProvider(IPiqaeHostKeyProvider provider)
    {
        ArgumentNullException.ThrowIfNull(provider);
        lock (_gate)
        {
            ThrowIfDisposed();
            if (_hostKeyHandle.HasValue) throw new InvalidOperationException("A host key provider is already configured.");
            var handle = GCHandle.Alloc(provider);
            HmacCallback callback = HostHmacCallback;
            var native = new NativeHostKeyProvider
            {
                Context = GCHandle.ToIntPtr(handle),
                HmacSha256 = Marshal.GetFunctionPointerForDelegate(callback)
            };
            try
            {
                using var ignored = NativeResponse.Call(_handle, native, NativeMethods.piqae_node_set_host_key_provider);
                _hostKeyHandle = handle;
                _hostKeyCallback = callback;
            }
            catch { handle.Free(); throw; }
        }
    }

    public string DeriveOpaqueEvidence(string namespaceName, string canonicalIdentity)
    {
        lock (_gate)
        {
            ThrowIfDisposed();
            var payload = JsonSerializer.SerializeToUtf8Bytes(new
            {
                type = "derive_opaque_evidence",
                @namespace = namespaceName,
                canonical_identity = canonicalIdentity
            }, JsonOptions);
            using var response = NativeResponse.Call(_handle, payload, NativeMethods.piqae_node_command);
            return response.Data.GetProperty("opaque_evidence").GetString()
                ?? throw new PiqaeNodeException("invalid_native_response", "The native runtime returned incomplete evidence.");
        }
    }

    ~PiqaeNode() => Dispose(false);

    public void Dispose()
    {
        Dispose(true);
        GC.SuppressFinalize(this);
    }

    private void Dispose(bool disposing)
    {
        lock (_gate)
        {
            if (_disposed) return;
            _disposed = true;
            if (_handle != 0)
            {
                try
                {
                    using var ignored = NativeResponse.Call(_handle, NativeMethods.piqae_node_destroy, throwOnError: false);
                }
                catch when (!disposing) { }
                finally { _handle = 0; }
            }
            if (_hostKeyHandle is { } hostKeyHandle)
            {
                hostKeyHandle.Free();
                _hostKeyHandle = null;
                _hostKeyCallback = null;
            }
        }
    }

    private JsonElement CallHandle(Func<ulong, NativeBuffer> operation)
    {
        lock (_gate)
        {
            ThrowIfDisposed();
            using var response = NativeResponse.Call(_handle, operation);
            return response.Data.Clone();
        }
    }

    private JsonElement Command(object command)
    {
        lock (_gate)
        {
            ThrowIfDisposed();
            var payload = JsonSerializer.SerializeToUtf8Bytes(command, JsonOptions);
            using var response = NativeResponse.Call(_handle, payload, NativeMethods.piqae_node_command);
            return response.Data.Clone();
        }
    }

    private void ThrowIfDisposed() => ObjectDisposedException.ThrowIf(_disposed, this);

    private static int HostHmacCallback(
        IntPtr context,
        IntPtr keyScope,
        nuint keyScopeLength,
        IntPtr message,
        nuint messageLength,
        IntPtr output,
        nuint outputLength)
    {
        if (context == IntPtr.Zero || output == IntPtr.Zero || outputLength != 32
            || keyScopeLength > 128 || messageLength > 8192) return 1;
        try
        {
            var provider = (IPiqaeHostKeyProvider?)GCHandle.FromIntPtr(context).Target;
            if (provider is null) return 1;
            var scopeBytes = new byte[(int)keyScopeLength];
            var messageBytes = new byte[(int)messageLength];
            Marshal.Copy(keyScope, scopeBytes, 0, scopeBytes.Length);
            Marshal.Copy(message, messageBytes, 0, messageBytes.Length);
            var digest = provider.HmacSha256(Encoding.UTF8.GetString(scopeBytes), messageBytes);
            if (digest.Length != 32) return 1;
            Marshal.Copy(digest, 0, output, digest.Length);
            CryptographicOperations.ZeroMemory(digest);
            return 0;
        }
        catch { return 1; }
    }
}

public interface IPiqaeHostKeyProvider
{
    /// Returns only a 32-byte HMAC. Implementations must retain key material in
    /// a platform secure store and be safe for concurrent calls.
    byte[] HmacSha256(string keyScope, ReadOnlySpan<byte> message);
}

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate int HmacCallback(IntPtr context, IntPtr keyScope, nuint keyScopeLength, IntPtr message, nuint messageLength, IntPtr output, nuint outputLength);

[StructLayout(LayoutKind.Sequential)]
internal struct NativeHostKeyProvider
{
    internal IntPtr Context;
    internal IntPtr HmacSha256;
}

public sealed class PiqaeNodeException(string code, string message) : Exception(message)
{
    public string Code { get; } = code;
}

[StructLayout(LayoutKind.Sequential)]
internal readonly struct NativeBuffer
{
    public readonly IntPtr Data;
    public readonly nuint Length;
}

[StructLayout(LayoutKind.Sequential)]
internal readonly struct NativeAbiDescriptor
{
    public readonly ushort AbiVersion;
    public readonly ushort ContractMin;
    public readonly ushort ContractMax;
}

internal static class NativeMethods
{
    private const string Library = "piqae_node_ffi";

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeAbiDescriptor piqae_node_abi_descriptor();
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeBuffer piqae_node_create(byte[] data, nuint length);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeBuffer piqae_node_start(ulong handle);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeBuffer piqae_node_set_host_key_provider(ulong handle, NativeHostKeyProvider provider);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeBuffer piqae_node_stop(ulong handle);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeBuffer piqae_node_snapshot(ulong handle);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeBuffer piqae_node_command(ulong handle, byte[] data, nuint length);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeBuffer piqae_node_destroy(ulong handle);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern void piqae_node_free(NativeBuffer buffer);
}

internal sealed class NativeResponse : IDisposable
{
    private NativeBuffer _buffer;
    private JsonDocument _document = null!;

    private NativeResponse(NativeBuffer buffer, bool throwOnError)
    {
        _buffer = buffer;
        try
        {
            if (buffer.Data == IntPtr.Zero || buffer.Length == 0 || buffer.Length > 1024 * 1024)
                throw new PiqaeNodeException("invalid_native_response", "The native runtime returned an invalid response.");
            var bytes = new byte[(int)buffer.Length];
            Marshal.Copy(buffer.Data, bytes, 0, bytes.Length);
            _document = JsonDocument.Parse(bytes);
            var root = _document.RootElement;
            if (throwOnError && (!root.TryGetProperty("ok", out var ok) || !ok.GetBoolean()))
            {
                var error = root.GetProperty("error");
                throw new PiqaeNodeException(
                    error.GetProperty("code").GetString() ?? "native_error",
                    error.GetProperty("message").GetString() ?? "The native runtime operation failed.");
            }
        }
        catch
        {
            if (_buffer.Data != IntPtr.Zero) NativeMethods.piqae_node_free(_buffer);
            _buffer = default;
            _document?.Dispose();
            throw;
        }
    }

    internal JsonElement Data => _document.RootElement.GetProperty("data");

    internal static NativeResponse Call(byte[] payload, Func<byte[], nuint, NativeBuffer> operation) =>
        new(operation(payload, (nuint)payload.Length), true);
    internal static NativeResponse Call(ulong handle, Func<ulong, NativeBuffer> operation, bool throwOnError = true) =>
        new(operation(handle), throwOnError);
    internal static NativeResponse Call(ulong handle, NativeHostKeyProvider provider, Func<ulong, NativeHostKeyProvider, NativeBuffer> operation) =>
        new(operation(handle, provider), true);
    internal static NativeResponse Call(ulong handle, byte[] payload, Func<ulong, byte[], nuint, NativeBuffer> operation) =>
        new(operation(handle, payload, (nuint)payload.Length), true);

    public void Dispose()
    {
        _document.Dispose();
        if (_buffer.Data != IntPtr.Zero)
        {
            NativeMethods.piqae_node_free(_buffer);
            _buffer = default;
        }
    }
}
