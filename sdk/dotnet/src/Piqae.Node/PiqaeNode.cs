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

public sealed class PiqaeNode : IDisposable
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower) }
    };

    private ulong _handle;
    private bool _disposed;
    private GCHandle? _hostKeyHandle;
    private HmacCallback? _hostKeyCallback;

    public PiqaeNode(PiqaeNodeOptions options)
    {
        ArgumentNullException.ThrowIfNull(options);
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

    public void ConfigureHostKeyProvider(IPiqaeHostKeyProvider provider)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(provider);
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

    public string DeriveOpaqueEvidence(string namespaceName, string canonicalIdentity)
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

    public void Dispose()
    {
        if (_disposed) return;
        _disposed = true;
        if (_handle != 0)
        {
            using var ignored = NativeResponse.Call(_handle, NativeMethods.piqae_node_destroy, throwOnError: false);
            _handle = 0;
        }
        if (_hostKeyHandle is { } hostKeyHandle)
        {
            hostKeyHandle.Free();
            _hostKeyHandle = null;
            _hostKeyCallback = null;
        }
        GC.SuppressFinalize(this);
    }

    private JsonElement CallHandle(Func<ulong, NativeBuffer> operation)
    {
        ThrowIfDisposed();
        using var response = NativeResponse.Call(_handle, operation);
        return response.Data.Clone();
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

internal static class NativeMethods
{
    private const string Library = "piqae_node_ffi";

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
