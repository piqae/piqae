using System.Buffers.Binary;
using System.IO.Pipes;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using Microsoft.Win32.SafeHandles;
using System.Runtime.InteropServices;

namespace Piqae.Node;

public enum BrokerCapability { ObserveStatus, ObservePrinters, ManageProfiles, SubmitLocalJobs, ManageConnectors }

public sealed record BrokerApplication(string ApplicationId, string DisplayName, string? SigningIdentitySha256 = null);
public sealed record BrokerAuthorizationHandle(Guid AuthorizationId, string Nonce, long ExpiresUnixMs);
public sealed record BrokerCredential(string ApplicationId, string Token);

public sealed class PiqaeBrokerClient
{
    private const int MaximumMessageBytes = 2 * 1024 * 1024;
    private static readonly TimeSpan RequestTimeout = TimeSpan.FromSeconds(5);
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower) }
    };
    private readonly string _pipeName;

    public PiqaeBrokerClient(string endpoint)
    {
        const string prefix = @"\\.\pipe\";
        if (!OperatingSystem.IsWindows()
            || !endpoint.StartsWith(prefix + "piqae-node-", StringComparison.Ordinal)
            || endpoint.Length <= prefix.Length
            || endpoint.Length > 240
            || endpoint[prefix.Length..].Any(character => !char.IsAsciiLetterOrDigit(character) && character != '-'))
            throw new ArgumentException("A local Windows Piqae pipe endpoint is required.", nameof(endpoint));
        _pipeName = endpoint[prefix.Length..];
    }

    public static string EndpointForDataDirectory(string dataDirectory)
    {
        var digest = SHA256.HashData(Encoding.UTF8.GetBytes(dataDirectory));
        return $@"\\.\pipe\piqae-node-{Convert.ToHexString(digest.AsSpan(0, 12)).ToLowerInvariant()}";
    }

    public async Task<BrokerAuthorizationHandle> RequestAuthorizationAsync(
        BrokerApplication application,
        IReadOnlyCollection<BrokerCapability> capabilities,
        CancellationToken cancellationToken = default)
    {
        var result = await RequestAsync("request_authorization", new
        {
            application,
            requested_capabilities = capabilities
        }, cancellationToken).ConfigureAwait(false);
        return JsonSerializer.Deserialize<BrokerAuthorizationHandle>(result.GetRawText(), JsonOptions)
            ?? throw new PiqaeNodeException("invalid_broker_response", "The broker response was incomplete.");
    }

    public async Task<string> AuthorizationStatusAsync(BrokerAuthorizationHandle handle, CancellationToken cancellationToken = default)
    {
        var result = await RequestAsync("authorization_status", new { handle }, cancellationToken).ConfigureAwait(false);
        return result.GetProperty("state").GetString()
            ?? throw new PiqaeNodeException("invalid_broker_response", "The broker response was incomplete.");
    }

    public async Task<BrokerCredential> ExchangeAndStoreAsync(BrokerAuthorizationHandle handle, CancellationToken cancellationToken = default)
    {
        var result = await RequestAsync("exchange_authorization", new { handle }, cancellationToken).ConfigureAwait(false);
        var credential = JsonSerializer.Deserialize<BrokerCredential>(result.GetRawText(), JsonOptions)
            ?? throw new PiqaeNodeException("invalid_broker_response", "The broker response was incomplete.");
        WindowsCredentialStore.Write(CredentialTarget(credential.ApplicationId), JsonSerializer.Serialize(credential, JsonOptions));
        return credential;
    }

    public BrokerCredential? LoadStoredCredential(string applicationId)
    {
        var stored = WindowsCredentialStore.Read(CredentialTarget(applicationId));
        return stored is null ? null : JsonSerializer.Deserialize<BrokerCredential>(stored, JsonOptions);
    }

    public Task<JsonElement> ExecuteSdkAsync(
        BrokerCredential credential,
        BrokerCapability capability,
        object sdkOperation,
        CancellationToken cancellationToken = default) => RequestAsync("execute", new
    {
        credential,
        capability,
        operation = new { type = "sdk", operation = sdkOperation }
    }, cancellationToken);

    private string CredentialTarget(string applicationId) => $"Piqae.Node/{_pipeName}/{applicationId}";

    private async Task<JsonElement> RequestAsync(string operationType, object fields, CancellationToken cancellationToken)
    {
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        deadline.CancelAfter(RequestTimeout);
        await using var pipe = new NamedPipeClientStream(".", _pipeName, PipeDirection.InOut, PipeOptions.Asynchronous);
        await pipe.ConnectAsync(deadline.Token).ConfigureAwait(false);
        var requestId = Guid.NewGuid();
        var fieldsJson = JsonSerializer.SerializeToElement(fields, JsonOptions);
        using var document = JsonDocument.Parse(JsonSerializer.SerializeToUtf8Bytes(new
        {
            protocol = 3,
            request_id = requestId,
            operation = MergeOperation(operationType, fieldsJson)
        }, JsonOptions));
        var body = JsonSerializer.SerializeToUtf8Bytes(document.RootElement, JsonOptions);
        if (body.Length > MaximumMessageBytes) throw new PiqaeNodeException("message_too_large", "The broker request is too large.");
        var prefix = new byte[4];
        BinaryPrimitives.WriteUInt32BigEndian(prefix, (uint)body.Length);
        await pipe.WriteAsync(prefix, deadline.Token).ConfigureAwait(false);
        await pipe.WriteAsync(body, deadline.Token).ConfigureAwait(false);
        await pipe.FlushAsync(deadline.Token).ConfigureAwait(false);
        await ReadExactlyAsync(pipe, prefix, deadline.Token).ConfigureAwait(false);
        var length = BinaryPrimitives.ReadUInt32BigEndian(prefix);
        if (length == 0 || length > MaximumMessageBytes) throw new PiqaeNodeException("invalid_broker_response", "The broker response length is invalid.");
        body = new byte[length];
        await ReadExactlyAsync(pipe, body, deadline.Token).ConfigureAwait(false);
        using var response = JsonDocument.Parse(body);
        var root = response.RootElement;
        if (!root.TryGetProperty("protocol", out var protocol) || protocol.GetUInt16() is < 1 or > 3)
            throw new PiqaeNodeException("unsupported_broker_protocol", "The node broker protocol is incompatible.");
        if (root.GetProperty("request_id").GetGuid() != requestId)
            throw new PiqaeNodeException("response_id_mismatch", "The broker response did not match the request.");
        var result = root.GetProperty("result");
        if (result.TryGetProperty("Err", out var failure))
            throw new PiqaeNodeException(failure.GetProperty("code").GetString() ?? "broker_error", failure.GetProperty("message").GetString() ?? "The broker rejected the request.");
        if (!result.TryGetProperty("Ok", out var success))
            throw new PiqaeNodeException("invalid_broker_response", "The broker response was incomplete.");
        return success.Clone();
    }

    private static JsonElement MergeOperation(string type, JsonElement fields)
    {
        var json = new Dictionary<string, object?> { ["type"] = type };
        foreach (var property in fields.EnumerateObject()) json[property.Name] = property.Value.Clone();
        return JsonSerializer.SerializeToElement(json, JsonOptions);
    }

    private static async Task ReadExactlyAsync(Stream stream, Memory<byte> buffer, CancellationToken cancellationToken)
    {
        var read = 0;
        while (read < buffer.Length)
        {
            var count = await stream.ReadAsync(buffer[read..], cancellationToken).ConfigureAwait(false);
            if (count == 0) throw new EndOfStreamException("The broker closed the pipe early.");
            read += count;
        }
    }
}

internal static class WindowsCredentialStore
{
    private const uint CredTypeGeneric = 1;
    private const uint CredPersistLocalMachine = 2;

    internal static void Write(string target, string secret)
    {
        if (!OperatingSystem.IsWindows()) throw new PlatformNotSupportedException();
        var bytes = Encoding.UTF8.GetBytes(secret);
        if (bytes.Length > 5120) throw new ArgumentOutOfRangeException(nameof(secret));
        var blob = Marshal.AllocHGlobal(bytes.Length);
        try
        {
            Marshal.Copy(bytes, 0, blob, bytes.Length);
            var credential = new NativeCredential
            {
                Type = CredTypeGeneric,
                TargetName = target,
                CredentialBlobSize = (uint)bytes.Length,
                CredentialBlob = blob,
                Persist = CredPersistLocalMachine,
                UserName = Environment.UserName
            };
            if (!CredWrite(ref credential, 0)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        }
        finally { CryptographicOperations.ZeroMemory(bytes); Marshal.FreeHGlobal(blob); }
    }

    internal static string? Read(string target)
    {
        if (!OperatingSystem.IsWindows()) throw new PlatformNotSupportedException();
        if (!CredRead(target, CredTypeGeneric, 0, out var pointer)) return null;
        try
        {
            var credential = Marshal.PtrToStructure<NativeCredential>(pointer);
            var bytes = new byte[checked((int)credential.CredentialBlobSize)];
            Marshal.Copy(credential.CredentialBlob, bytes, 0, bytes.Length);
            try { return Encoding.UTF8.GetString(bytes); }
            finally { CryptographicOperations.ZeroMemory(bytes); }
        }
        finally { CredFree(pointer); }
    }

    internal static void Delete(string target)
    {
        if (OperatingSystem.IsWindows()) _ = CredDelete(target, CredTypeGeneric, 0);
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct NativeCredential
    {
        public uint Flags, Type;
        public string TargetName;
        public string? Comment;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
        public uint CredentialBlobSize;
        public IntPtr CredentialBlob;
        public uint Persist, AttributeCount;
        public IntPtr Attributes;
        public string? TargetAlias;
        public string UserName;
    }

    [DllImport("advapi32.dll", EntryPoint = "CredWriteW", CharSet = CharSet.Unicode, SetLastError = true)] private static extern bool CredWrite(ref NativeCredential credential, uint flags);
    [DllImport("advapi32.dll", EntryPoint = "CredReadW", CharSet = CharSet.Unicode, SetLastError = true)] private static extern bool CredRead(string target, uint type, uint flags, out IntPtr credential);
    [DllImport("advapi32.dll", EntryPoint = "CredDeleteW", CharSet = CharSet.Unicode, SetLastError = true)] private static extern bool CredDelete(string target, uint type, uint flags);
    [DllImport("advapi32.dll")] private static extern void CredFree(IntPtr credential);
}

public sealed class WindowsCredentialHostKeyProvider(string installationScope) : IPiqaeHostKeyProvider
{
    private readonly string _targetPrefix = $"Piqae.Node/opaque-key/{installationScope}";

    public byte[] HmacSha256(string keyScope, ReadOnlySpan<byte> message)
    {
        if (!OperatingSystem.IsWindows()) throw new PlatformNotSupportedException();
        if (string.IsNullOrWhiteSpace(keyScope) || keyScope.Length > 64) throw new ArgumentException("Invalid key scope.", nameof(keyScope));
        var target = $"{_targetPrefix}/{keyScope}";
        var stored = WindowsCredentialStore.Read(target);
        byte[] key;
        if (stored is null)
        {
            key = RandomNumberGenerator.GetBytes(32);
            WindowsCredentialStore.Write(target, Convert.ToBase64String(key));
            var verified = WindowsCredentialStore.Read(target);
            byte[]? verifiedKey = null;
            try
            {
                verifiedKey = verified is null ? null : Convert.FromBase64String(verified);
                if (verifiedKey is null || !CryptographicOperations.FixedTimeEquals(key, verifiedKey))
                    throw new CryptographicException("The host key could not be verified in Windows Credential Manager.");
            }
            catch { CryptographicOperations.ZeroMemory(key); throw; }
            finally { if (verifiedKey is not null) CryptographicOperations.ZeroMemory(verifiedKey); }
        }
        else
        {
            key = Convert.FromBase64String(stored);
            if (key.Length != 32)
            {
                CryptographicOperations.ZeroMemory(key);
                throw new CryptographicException("Windows Credential Manager returned invalid host key material.");
            }
        }
        try { return HMACSHA256.HashData(key, message); }
        finally { CryptographicOperations.ZeroMemory(key); }
    }
}
