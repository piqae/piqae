using System.Buffers.Binary;
using System.IO.Pipes;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using Microsoft.Win32.SafeHandles;
using System.Runtime.InteropServices;

namespace Piqae.Node;

public enum BrokerCapability { ObserveStatus, ObservePrinters, ObserveJobHistory, ManageProfiles, SubmitLocalJobs, ManageConnectors }

public sealed record BrokerAuthorizationHandle(Guid AuthorizationId, string Nonce, long ExpiresUnixMs);
public sealed record BrokerCredential(string ApplicationId, string Token)
{
    public override string ToString() => $"BrokerCredential {{ ApplicationId = {ApplicationId}, Token = [REDACTED] }}";
}

public sealed class PiqaeBrokerClient
{
    private const int MaximumMessageBytes = 2 * 1024 * 1024;
    private static readonly TimeSpan RequestTimeout = TimeSpan.FromSeconds(5);
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower) }
    };
    private readonly string _endpoint;
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
        _endpoint = endpoint;
        _pipeName = endpoint[prefix.Length..];
    }

    public static string EndpointForDataDirectory(string dataDirectory)
    {
        var digest = SHA256.HashData(Encoding.UTF8.GetBytes(dataDirectory));
        return $@"\\.\pipe\piqae-node-{Convert.ToHexString(digest.AsSpan(0, 12)).ToLowerInvariant()}";
    }

    public async Task<BrokerAuthorizationHandle> RequestAuthorizationAsync(
        IReadOnlyCollection<BrokerCapability> capabilities,
        CancellationToken cancellationToken = default)
    {
        var result = await RequestAsync("request_authorization", new
        {
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
        var credential = result.Deserialize<BrokerCredential>(JsonOptions)
            ?? throw new PiqaeNodeException("invalid_broker_response", "The broker response was incomplete.");
        var credentialBytes = JsonSerializer.SerializeToUtf8Bytes(credential, JsonOptions);
        try
        {
            WindowsCredentialStore.WriteBytes(CredentialTarget(), credentialBytes);
            WindowsCredentialStore.DeleteRequired(LegacyCredentialTarget(credential.ApplicationId));
        }
        finally { CryptographicOperations.ZeroMemory(credentialBytes); }
        return credential;
    }

    public BrokerCredential? LoadStoredCredential()
    {
        var stored = WindowsCredentialStore.ReadBytes(CredentialTarget());
        if (stored is null) return null;
        try { return JsonSerializer.Deserialize<BrokerCredential>(stored, JsonOptions); }
        finally { CryptographicOperations.ZeroMemory(stored); }
    }

    public async Task<JsonElement> ExecuteSdkAsync(
        BrokerCredential credential,
        BrokerCapability capability,
        object sdkOperation,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(credential);
        ArgumentNullException.ThrowIfNull(sdkOperation);
        cancellationToken.ThrowIfCancellationRequested();

        // Authentication, canonicalization, request/response proof verification,
        // replay protection, and the bounded timeout all remain in the Rust v4
        // client. In particular, this SDK never sends the bearer token to a pipe.
        return await Task.Run(() =>
        {
            cancellationToken.ThrowIfCancellationRequested();
            var endpoint = Encoding.UTF8.GetBytes(_endpoint);
            var credentialJson = JsonSerializer.SerializeToUtf8Bytes(credential, JsonOptions);
            var capabilityJson = JsonSerializer.SerializeToUtf8Bytes(capability, JsonOptions);
            var operationJson = JsonSerializer.SerializeToUtf8Bytes(new
            {
                type = "sdk",
                operation = sdkOperation
            }, JsonOptions);
            try
            {
                using var response = NativeResponse.CallBroker(
                    endpoint,
                    credentialJson,
                    capabilityJson,
                    operationJson);
                if (!response.Data.TryGetProperty("result", out var result)
                    || result.ValueKind != JsonValueKind.Object
                    || !result.TryGetProperty("type", out var type)
                    || type.ValueKind != JsonValueKind.String
                    || type.GetString() != "sdk"
                    || !result.TryGetProperty("data", out var data)
                    || data.ValueKind == JsonValueKind.Undefined)
                    throw new PiqaeNodeException(
                        "invalid_broker_response",
                        "The authenticated node broker returned an unexpected result.");
                return data.Clone();
            }
            finally
            {
                CryptographicOperations.ZeroMemory(endpoint);
                CryptographicOperations.ZeroMemory(credentialJson);
                CryptographicOperations.ZeroMemory(capabilityJson);
                CryptographicOperations.ZeroMemory(operationJson);
            }
        }, cancellationToken).ConfigureAwait(false);
    }

    private string CredentialTarget()
    {
        var executable = Path.GetFullPath(Environment.ProcessPath
            ?? throw new InvalidOperationException("The current executable path is unavailable."))
            .Replace('/', '\\').ToLowerInvariant();
        var processSlot = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(executable))
            .AsSpan(0, 16)).ToLowerInvariant();
        return $"Piqae.Node/{_pipeName}/{processSlot}";
    }

    private string LegacyCredentialTarget(string applicationId) => $"Piqae.Node/{_pipeName}/{applicationId}";

    private async Task<JsonElement> RequestAsync(string operationType, object fields, CancellationToken cancellationToken)
    {
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        deadline.CancelAfter(RequestTimeout);
        await using var pipe = new NamedPipeClientStream(".", _pipeName, PipeDirection.InOut, PipeOptions.Asynchronous);
        await pipe.ConnectAsync(deadline.Token).ConfigureAwait(false);
        var requestId = Guid.NewGuid();
        var fieldsJson = JsonSerializer.SerializeToElement(fields, JsonOptions);
        var body = JsonSerializer.SerializeToUtf8Bytes(new
        {
            protocol = 4,
            request_id = requestId,
            operation = MergeOperation(operationType, fieldsJson)
        }, JsonOptions);
        var prefix = new byte[4];
        try
        {
            if (body.Length > MaximumMessageBytes) throw new PiqaeNodeException("message_too_large", "The broker request is too large.");
            BinaryPrimitives.WriteUInt32BigEndian(prefix, (uint)body.Length);
            await pipe.WriteAsync(prefix, deadline.Token).ConfigureAwait(false);
            await pipe.WriteAsync(body, deadline.Token).ConfigureAwait(false);
            await pipe.FlushAsync(deadline.Token).ConfigureAwait(false);
        }
        finally { CryptographicOperations.ZeroMemory(body); }
        await ReadExactlyAsync(pipe, prefix, deadline.Token).ConfigureAwait(false);
        var length = BinaryPrimitives.ReadUInt32BigEndian(prefix);
        if (length == 0 || length > MaximumMessageBytes) throw new PiqaeNodeException("invalid_broker_response", "The broker response length is invalid.");
        body = new byte[length];
        try
        {
            await ReadExactlyAsync(pipe, body, deadline.Token).ConfigureAwait(false);
            using var response = JsonDocument.Parse(body);
            var root = response.RootElement;
            if (!root.TryGetProperty("protocol", out var protocol) || protocol.GetUInt16() != 4)
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
        finally { CryptographicOperations.ZeroMemory(body); }
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
        var bytes = Encoding.UTF8.GetBytes(secret);
        try { WriteBytes(target, bytes); }
        finally { CryptographicOperations.ZeroMemory(bytes); }
    }

    internal static void WriteBytes(string target, ReadOnlySpan<byte> secret)
    {
        if (!OperatingSystem.IsWindows()) throw new PlatformNotSupportedException();
        if (secret.Length > 5120) throw new ArgumentOutOfRangeException(nameof(secret));
        var bytes = secret.ToArray();
        var blob = IntPtr.Zero;
        try
        {
            blob = Marshal.AllocHGlobal(bytes.Length);
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
        finally
        {
            CryptographicOperations.ZeroMemory(bytes);
            if (blob != IntPtr.Zero && secret.Length > 0)
            {
                var zeros = new byte[secret.Length];
                Marshal.Copy(zeros, 0, blob, zeros.Length);
            }
            if (blob != IntPtr.Zero) Marshal.FreeHGlobal(blob);
        }
    }

    internal static string? Read(string target)
    {
        var bytes = ReadBytes(target);
        if (bytes is null) return null;
        try { return Encoding.UTF8.GetString(bytes); }
        finally { CryptographicOperations.ZeroMemory(bytes); }
    }

    internal static byte[]? ReadBytes(string target)
    {
        if (!OperatingSystem.IsWindows()) throw new PlatformNotSupportedException();
        if (!CredRead(target, CredTypeGeneric, 0, out var pointer))
        {
            const int errorNotFound = 1168;
            var error = Marshal.GetLastWin32Error();
            if (error == errorNotFound) return null;
            throw new System.ComponentModel.Win32Exception(error);
        }
        try
        {
            var credential = Marshal.PtrToStructure<NativeCredential>(pointer);
            var bytes = new byte[checked((int)credential.CredentialBlobSize)];
            Marshal.Copy(credential.CredentialBlob, bytes, 0, bytes.Length);
            return bytes;
        }
        finally { CredFree(pointer); }
    }

    internal static void Delete(string target)
    {
        if (OperatingSystem.IsWindows()) _ = CredDelete(target, CredTypeGeneric, 0);
    }

    internal static void DeleteRequired(string target)
    {
        if (!OperatingSystem.IsWindows()) throw new PlatformNotSupportedException();
        if (CredDelete(target, CredTypeGeneric, 0)) return;
        const int errorNotFound = 1168;
        var error = Marshal.GetLastWin32Error();
        if (error != errorNotFound) throw new System.ComponentModel.Win32Exception(error);
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
        var mutexName = $@"Local\Piqae.Node.opaque-key.{Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(target)))}";
        using var mutex = new Mutex(false, mutexName);
        var ownsMutex = false;
        try
        {
            try { ownsMutex = mutex.WaitOne(TimeSpan.FromSeconds(5)); }
            catch (AbandonedMutexException) { ownsMutex = true; }
            if (!ownsMutex) throw new TimeoutException("Timed out waiting for the installation host-key lock.");
            // Re-read only after acquiring the installation/scope mutex so two
            // app processes can never both create different keys and race the
            // Credential Manager replacement.
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
        finally { if (ownsMutex) mutex.ReleaseMutex(); }
    }

    internal void DeleteForTests(string keyScope) =>
        WindowsCredentialStore.DeleteRequired($"{_targetPrefix}/{keyScope}");
}
