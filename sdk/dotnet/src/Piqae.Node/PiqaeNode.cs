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

public enum PiqaePrinterGrant { SelectedPrinters, AllLocalPrinters }

/// <summary>A prepared, short-lived connector identity. The key handle is opaque and non-secret.</summary>
public sealed record PreparedConnectorInvitation(
    string KeyHandle,
    string PublicKeyBase64,
    long ExpiresUnixMs);

/// <summary>
/// Inputs required to redeem an authority-issued invitation. Connector identity
/// and ownership metadata are returned only by the pinned control plane.
/// </summary>
public sealed record PiqaeConnectorInvitation(
    Uri ControlPlaneUrl,
    string InvitationToken,
    string ConnectorKeyHandle,
    PiqaePrinterGrant PrinterGrant,
    IReadOnlyList<string> AllowedPrinterIds,
    string NodeName,
    string Hostname)
{
    public override string ToString() =>
        $"PiqaeConnectorInvitation {{ ControlPlaneUrl = {ControlPlaneUrl}, InvitationToken = [REDACTED], ConnectorKeyHandle = {ConnectorKeyHandle}, PrinterGrant = {PrinterGrant}, NodeName = {NodeName}, Hostname = {Hostname} }}";
}

public sealed class PiqaeNode : IDisposable
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower) }
    };

    private readonly object _gate = new();
    private readonly string _applicationId;
    private ulong _handle;
    private bool _disposed;
    private GCHandle? _hostKeyHandle;
    private HmacCallback? _hostKeyCallback;
    private GCHandle? _connectorKeyHandle;
    private GenerateConnectorKeyCallback? _generateConnectorKeyCallback;
    private SignConnectorCallback? _signConnectorCallback;
    private DeleteConnectorKeyCallback? _deleteConnectorKeyCallback;

    public PiqaeNode(PiqaeNodeOptions options, IPiqaeConnectorKeyProvider? connectorKeyProvider = null)
    {
        ArgumentNullException.ThrowIfNull(options);
        if (string.IsNullOrWhiteSpace(options.ApplicationId))
            throw new ArgumentException("An application ID is required.", nameof(options));
        _applicationId = options.ApplicationId;
        var descriptor = NativeMethods.piqae_node_abi_descriptor();
        EnsureCompatibleAbi(descriptor);
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
        if (connectorKeyProvider is not null)
        {
            try { ConfigureConnectorKeyProvider(connectorKeyProvider); }
            catch
            {
                using var ignored = NativeResponse.Call(_handle, NativeMethods.piqae_node_destroy, throwOnError: false);
                _handle = 0;
                _disposed = true;
                throw;
            }
        }
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

    /// <summary>Creates a short-lived connector key using this node's application scope.</summary>
    public PreparedConnectorInvitation PrepareConnectorInvitation()
    {
        var result = Command(new
        {
            type = "prepare_connector_key",
            application_scope = _applicationId
        });
        return new(
            RequiredString(result, "key_handle"),
            RequiredString(result, "public_key_base64"),
            RequiredInt64(result, "expires_unix_ms"));
    }

    /// <summary>Cancels a prepared key. Durable native cleanup safely retries deletion.</summary>
    public bool CancelPreparedConnectorInvitation(string keyHandle)
    {
        ValidateOpaqueKeyHandle(keyHandle);
        var result = Command(new
        {
            type = "cancel_prepared_connector_key",
            key_handle = keyHandle
        });
        return RequiredBoolean(result, "cancelled");
    }

    /// <summary>
    /// Redeems an invitation through the native authority exchange. The caller
    /// cannot supply a connector record or override authority-owned metadata.
    /// </summary>
    public JsonElement Connect(PiqaeConnectorInvitation invitation)
    {
        ArgumentNullException.ThrowIfNull(invitation);
        if (!invitation.ControlPlaneUrl.IsAbsoluteUri
            || invitation.ControlPlaneUrl.Scheme != Uri.UriSchemeHttps
            || !string.IsNullOrEmpty(invitation.ControlPlaneUrl.UserInfo)
            || !string.IsNullOrEmpty(invitation.ControlPlaneUrl.Fragment))
            throw new ArgumentException("The connector control plane must be an absolute HTTPS URL.", nameof(invitation));
        if (string.IsNullOrWhiteSpace(invitation.InvitationToken))
            throw new ArgumentException("An invitation token is required.", nameof(invitation));
        ValidateOpaqueKeyHandle(invitation.ConnectorKeyHandle);
        if (string.IsNullOrWhiteSpace(invitation.NodeName) || string.IsNullOrWhiteSpace(invitation.Hostname))
            throw new ArgumentException("Node and host names are required.", nameof(invitation));
        ArgumentNullException.ThrowIfNull(invitation.AllowedPrinterIds);

        var result = CommandSensitive(new
        {
            type = "connect_invitation",
            control_plane_url = invitation.ControlPlaneUrl.AbsoluteUri,
            invitation_token = invitation.InvitationToken,
            connector_key_handle = invitation.ConnectorKeyHandle,
            printer_grant = invitation.PrinterGrant,
            allowed_printer_ids = invitation.AllowedPrinterIds,
            node_name = invitation.NodeName,
            hostname = invitation.Hostname
        });
        return RequiredObject(result, "connector");
    }

    public JsonElement RevokeConnector(string connectorId) => Command(new
    {
        type = "revoke_connector",
        connector_id = connectorId
    });

    public void ConfigureConnectorKeyProvider(IPiqaeConnectorKeyProvider provider)
    {
        ArgumentNullException.ThrowIfNull(provider);
        lock (_gate)
        {
            ThrowIfDisposed();
            if (_connectorKeyHandle.HasValue)
                throw new InvalidOperationException("A connector key provider is already configured.");
            var handle = GCHandle.Alloc(provider);
            GenerateConnectorKeyCallback generate = GenerateConnectorKey;
            SignConnectorCallback sign = SignConnector;
            DeleteConnectorKeyCallback delete = DeleteConnectorKey;
            var native = new NativeConnectorKeyProvider
            {
                Context = GCHandle.ToIntPtr(handle),
                Generate = Marshal.GetFunctionPointerForDelegate(generate),
                Sign = Marshal.GetFunctionPointerForDelegate(sign),
                Delete = Marshal.GetFunctionPointerForDelegate(delete)
            };
            try
            {
                using var ignored = NativeResponse.Call(
                    _handle,
                    native,
                    NativeMethods.piqae_node_set_connector_key_provider);
                _connectorKeyHandle = handle;
                _generateConnectorKeyCallback = generate;
                _signConnectorCallback = sign;
                _deleteConnectorKeyCallback = delete;
            }
            catch { handle.Free(); throw; }
        }
    }

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
            if (_connectorKeyHandle is { } connectorKeyHandle)
            {
                connectorKeyHandle.Free();
                _connectorKeyHandle = null;
                _generateConnectorKeyCallback = null;
                _signConnectorCallback = null;
                _deleteConnectorKeyCallback = null;
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

    private JsonElement CommandSensitive(object command)
    {
        lock (_gate)
        {
            ThrowIfDisposed();
            var payload = JsonSerializer.SerializeToUtf8Bytes(command, JsonOptions);
            try
            {
                using var response = NativeResponse.Call(_handle, payload, NativeMethods.piqae_node_command);
                return response.Data.Clone();
            }
            finally { CryptographicOperations.ZeroMemory(payload); }
        }
    }

    private static string RequiredString(JsonElement element, string propertyName)
    {
        if (!element.TryGetProperty(propertyName, out var value)
            || value.ValueKind != JsonValueKind.String
            || string.IsNullOrEmpty(value.GetString()))
            throw InvalidNativeResponse();
        return value.GetString()!;
    }

    private static long RequiredInt64(JsonElement element, string propertyName)
    {
        if (!element.TryGetProperty(propertyName, out var value)
            || value.ValueKind != JsonValueKind.Number
            || !value.TryGetInt64(out var number))
            throw InvalidNativeResponse();
        return number;
    }

    private static bool RequiredBoolean(JsonElement element, string propertyName)
    {
        if (!element.TryGetProperty(propertyName, out var value)
            || value.ValueKind is not (JsonValueKind.True or JsonValueKind.False))
            throw InvalidNativeResponse();
        return value.GetBoolean();
    }

    private static JsonElement RequiredObject(JsonElement element, string propertyName)
    {
        if (!element.TryGetProperty(propertyName, out var value)
            || value.ValueKind != JsonValueKind.Object)
            throw InvalidNativeResponse();
        return value.Clone();
    }

    private static PiqaeNodeException InvalidNativeResponse() => new(
        "invalid_native_response",
        "The native runtime response was incomplete.");

    private static void ValidateOpaqueKeyHandle(string keyHandle)
    {
        if (string.IsNullOrWhiteSpace(keyHandle)
            || keyHandle.Length > 256
            || keyHandle.Any(character => !char.IsAsciiLetterOrDigit(character) && character is not '.' and not '-' and not '_'))
            throw new ArgumentException("The connector key handle is invalid.", nameof(keyHandle));
    }

    private void ThrowIfDisposed() => ObjectDisposedException.ThrowIf(_disposed, this);

    internal static void EnsureCompatibleAbi(NativeAbiDescriptor descriptor)
    {
        if (descriptor.AbiVersion != 1 || descriptor.ContractMin > 1 || descriptor.ContractMax < 1)
            throw new PiqaeNodeException(
                "unsupported_native_abi",
                "The native Piqae runtime ABI is not compatible with this SDK.");
    }

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

    private static int GenerateConnectorKey(
        IntPtr context,
        IntPtr scope,
        nuint scopeLength,
        IntPtr handleOutput,
        nuint handleCapacity,
        IntPtr handleLengthOutput,
        IntPtr publicKeyOutput,
        nuint publicKeyLength)
    {
        if (context == IntPtr.Zero || scope == IntPtr.Zero || handleOutput == IntPtr.Zero
            || handleLengthOutput == IntPtr.Zero || publicKeyOutput == IntPtr.Zero
            || scopeLength is 0 or > 256 || handleCapacity is 0 or > 256 || publicKeyLength != 32)
            return 1;
        try
        {
            var provider = (IPiqaeConnectorKeyProvider?)GCHandle.FromIntPtr(context).Target;
            if (provider is null) return 1;
            var scopeBytes = new byte[(int)scopeLength];
            Marshal.Copy(scope, scopeBytes, 0, scopeBytes.Length);
            PiqaeGeneratedConnectorKey generated;
            string scopeValue;
            try
            {
                scopeValue = new UTF8Encoding(false, true).GetString(scopeBytes);
                generated = provider.Generate(scopeValue);
            }
            finally { CryptographicOperations.ZeroMemory(scopeBytes); }
            if (generated is null) return 1;
            var validHandle = !string.IsNullOrWhiteSpace(generated.Handle)
                && generated.Handle.Length <= 256
                && generated.Handle.All(character =>
                    char.IsAsciiLetterOrDigit(character) || character is '.' or '-' or '_');
            if (!validHandle || generated.PublicKey is null || generated.PublicKey.Length != 32)
            {
                CleanupRejectedGeneratedConnectorKey(provider, scopeValue, generated.Handle);
                if (generated.PublicKey is not null)
                    CryptographicOperations.ZeroMemory(generated.PublicKey);
                return 1;
            }
            var handleBytes = Encoding.UTF8.GetBytes(generated.Handle);
            try
            {
                if (handleBytes.Length == 0 || (nuint)handleBytes.Length > handleCapacity)
                {
                    CleanupRejectedGeneratedConnectorKey(provider, scopeValue, generated.Handle);
                    return 1;
                }
                Marshal.Copy(handleBytes, 0, handleOutput, handleBytes.Length);
                Marshal.WriteIntPtr(handleLengthOutput, (IntPtr)handleBytes.Length);
                Marshal.Copy(generated.PublicKey, 0, publicKeyOutput, generated.PublicKey.Length);
                return 0;
            }
            finally
            {
                CryptographicOperations.ZeroMemory(handleBytes);
                CryptographicOperations.ZeroMemory(generated.PublicKey);
            }
        }
        catch { return 1; }
    }

    private static void CleanupRejectedGeneratedConnectorKey(
        IPiqaeConnectorKeyProvider provider,
        string scope,
        string handle)
    {
        // Rust cannot durably schedule cleanup until it receives a valid
        // handle. Best-effort cleanup avoids orphaning a newly generated
        // invitation key while preserving installation keys.
        if (!scope.StartsWith("connector/", StringComparison.Ordinal)) return;
        try { provider.Delete(handle); }
        catch { /* Callback failures remain redacted at the ABI. */ }
    }

    private static int SignConnector(
        IntPtr context,
        IntPtr handle,
        nuint handleLength,
        IntPtr message,
        nuint messageLength,
        IntPtr signatureOutput,
        nuint signatureLength)
    {
        if (context == IntPtr.Zero || handle == IntPtr.Zero || signatureOutput == IntPtr.Zero
            || handleLength is 0 or > 256 || messageLength > 1024 * 1024 || signatureLength != 64
            || (messageLength != 0 && message == IntPtr.Zero)) return 1;
        try
        {
            var provider = (IPiqaeConnectorKeyProvider?)GCHandle.FromIntPtr(context).Target;
            if (provider is null) return 1;
            var handleBytes = new byte[(int)handleLength];
            var messageBytes = new byte[(int)messageLength];
            Marshal.Copy(handle, handleBytes, 0, handleBytes.Length);
            if (messageBytes.Length > 0) Marshal.Copy(message, messageBytes, 0, messageBytes.Length);
            try
            {
                var signature = provider.Sign(
                    new UTF8Encoding(false, true).GetString(handleBytes),
                    messageBytes);
                try
                {
                    if (signature.Length != 64) return 1;
                    Marshal.Copy(signature, 0, signatureOutput, signature.Length);
                    return 0;
                }
                finally { CryptographicOperations.ZeroMemory(signature); }
            }
            finally
            {
                CryptographicOperations.ZeroMemory(handleBytes);
                CryptographicOperations.ZeroMemory(messageBytes);
            }
        }
        catch { return 1; }
    }

    private static int DeleteConnectorKey(IntPtr context, IntPtr handle, nuint handleLength)
    {
        if (context == IntPtr.Zero || handle == IntPtr.Zero || handleLength is 0 or > 256) return 1;
        try
        {
            var provider = (IPiqaeConnectorKeyProvider?)GCHandle.FromIntPtr(context).Target;
            if (provider is null) return 1;
            var handleBytes = new byte[(int)handleLength];
            Marshal.Copy(handle, handleBytes, 0, handleBytes.Length);
            try { provider.Delete(new UTF8Encoding(false, true).GetString(handleBytes)); }
            finally { CryptographicOperations.ZeroMemory(handleBytes); }
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

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate int GenerateConnectorKeyCallback(
    IntPtr context, IntPtr scope, nuint scopeLength, IntPtr handleOutput, nuint handleCapacity,
    IntPtr handleLengthOutput, IntPtr publicKeyOutput, nuint publicKeyLength);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate int SignConnectorCallback(
    IntPtr context, IntPtr handle, nuint handleLength, IntPtr message, nuint messageLength,
    IntPtr signatureOutput, nuint signatureLength);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate int DeleteConnectorKeyCallback(IntPtr context, IntPtr handle, nuint handleLength);

[StructLayout(LayoutKind.Sequential)]
internal struct NativeConnectorKeyProvider
{
    internal IntPtr Context;
    internal IntPtr Generate;
    internal IntPtr Sign;
    internal IntPtr Delete;
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

    internal NativeAbiDescriptor(ushort abiVersion, ushort contractMin, ushort contractMax)
    {
        AbiVersion = abiVersion;
        ContractMin = contractMin;
        ContractMax = contractMax;
    }
}

internal static class NativeMethods
{
    private const string Library = "piqae_node_ffi";

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeAbiDescriptor piqae_node_abi_descriptor();
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeBuffer piqae_node_create(byte[] data, nuint length);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeBuffer piqae_node_start(ulong handle);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeBuffer piqae_node_set_host_key_provider(ulong handle, NativeHostKeyProvider provider);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeBuffer piqae_node_set_connector_key_provider(ulong handle, NativeConnectorKeyProvider provider);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeBuffer piqae_node_stop(ulong handle);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeBuffer piqae_node_snapshot(ulong handle);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeBuffer piqae_node_command(ulong handle, byte[] data, nuint length);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeBuffer piqae_node_destroy(ulong handle);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern NativeBuffer piqae_node_broker_execute(
        byte[] endpointData, nuint endpointLength,
        byte[] credentialJson, nuint credentialLength,
        byte[] capabilityJson, nuint capabilityLength,
        byte[] operationJson, nuint operationLength);
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)] internal static extern void piqae_node_free(NativeBuffer buffer);
}

internal sealed class NativeResponse : IDisposable
{
    private NativeBuffer _buffer;
    private JsonDocument _document = null!;
    private byte[]? _managedBytes;

    private NativeResponse(NativeBuffer buffer, bool throwOnError)
    {
        _buffer = buffer;
        try
        {
            if (buffer.Data == IntPtr.Zero || buffer.Length == 0 || buffer.Length > 1024 * 1024)
                throw new PiqaeNodeException("invalid_native_response", "The native runtime returned an invalid response.");
            var bytes = new byte[(int)buffer.Length];
            Marshal.Copy(buffer.Data, bytes, 0, bytes.Length);
            _managedBytes = bytes;
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
            if (_managedBytes is not null) CryptographicOperations.ZeroMemory(_managedBytes);
            _managedBytes = null;
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
    internal static NativeResponse Call(ulong handle, NativeConnectorKeyProvider provider, Func<ulong, NativeConnectorKeyProvider, NativeBuffer> operation) =>
        new(operation(handle, provider), true);
    internal static NativeResponse Call(ulong handle, byte[] payload, Func<ulong, byte[], nuint, NativeBuffer> operation) =>
        new(operation(handle, payload, (nuint)payload.Length), true);
    internal static NativeResponse CallBroker(
        byte[] endpoint,
        byte[] credentialJson,
        byte[] capabilityJson,
        byte[] operationJson) => new(
            NativeMethods.piqae_node_broker_execute(
                endpoint, (nuint)endpoint.Length,
                credentialJson, (nuint)credentialJson.Length,
                capabilityJson, (nuint)capabilityJson.Length,
                operationJson, (nuint)operationJson.Length),
            true);

    public void Dispose()
    {
        _document.Dispose();
        if (_managedBytes is not null)
        {
            CryptographicOperations.ZeroMemory(_managedBytes);
            _managedBytes = null;
        }
        if (_buffer.Data != IntPtr.Zero)
        {
            NativeMethods.piqae_node_free(_buffer);
            _buffer = default;
        }
    }
}
