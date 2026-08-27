using System.Text.Json;
using System.Text.Json.Serialization;

namespace Piqae.Node;

[JsonConverter(typeof(SnakeCaseEnumJsonConverter<NodeHostProduct>))]
public enum NodeHostProduct
{
    Standalone,
    Embedded
}

[JsonConverter(typeof(SnakeCaseEnumJsonConverter<InstalledHostPolicy>))]
public enum InstalledHostPolicy
{
    PreferInstalled,
    RequireInstalled,
    IsolatedApplication
}

[JsonConverter(typeof(SnakeCaseEnumJsonConverter<ConnectionManagement>))]
public enum ConnectionManagement
{
    UserManaged,
    HostManaged
}

public sealed class SnakeCaseEnumJsonConverter<T> : JsonConverter<T> where T : struct, Enum
{
    public override T Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
    {
        var wireValue = reader.GetString()
            ?? throw new JsonException("An enum string is required.");
        foreach (var candidate in Enum.GetValues<T>())
        {
            if (JsonNamingPolicy.SnakeCaseLower.ConvertName(candidate.ToString()) == wireValue)
                return candidate;
        }
        throw new JsonException($"Unsupported {typeof(T).Name} value.");
    }

    public override void Write(Utf8JsonWriter writer, T value, JsonSerializerOptions options) =>
        writer.WriteStringValue(JsonNamingPolicy.SnakeCaseLower.ConvertName(value.ToString()));
}

public sealed record NodeIdentityConfiguration
{
    [JsonPropertyName("display_name")]
    public string DisplayName { get; }
    [JsonPropertyName("site")]
    public string? Site { get; }
    [JsonPropertyName("location")]
    public string? Location { get; }
    [JsonPropertyName("labels")]
    public IReadOnlyList<string> Labels { get; }

    [JsonConstructor]
    public NodeIdentityConfiguration(
        string displayName,
        string? site = null,
        string? location = null,
        IReadOnlyList<string>? labels = null)
    {
        DisplayName = Required(displayName, "Node name", 120);
        Site = Optional(site, "Site", 120);
        Location = Optional(location, "Location", 120);
        var boundedLabels = labels?.Select(value => Required(value, "Label", 64)).ToArray()
            ?? Array.Empty<string>();
        if (boundedLabels.Length > 16)
            throw new ArgumentException("A node can have at most 16 labels.", nameof(labels));
        if (boundedLabels.Distinct(StringComparer.Ordinal).Count() != boundedLabels.Length)
            throw new ArgumentException("Node labels must be unique.", nameof(labels));
        Labels = boundedLabels;
    }

    private static string Required(string value, string field, int maximum)
    {
        ArgumentNullException.ThrowIfNull(value);
        var trimmed = value.Trim();
        if (trimmed.Length == 0 || System.Text.Encoding.UTF8.GetByteCount(trimmed) > maximum)
            throw new ArgumentException($"{field} must contain 1 to {maximum} UTF-8 bytes.");
        return trimmed;
    }

    private static string? Optional(string? value, string field, int maximum)
    {
        if (value is null) return null;
        var trimmed = value.Trim();
        if (trimmed.Length == 0) return null;
        if (System.Text.Encoding.UTF8.GetByteCount(trimmed) > maximum)
            throw new ArgumentException($"{field} must contain at most {maximum} UTF-8 bytes.");
        return trimmed;
    }
}

public sealed record ConnectionPolicy
{
    [JsonPropertyName("management")]
    public ConnectionManagement Management { get; }
    [JsonPropertyName("allows_multiple")]
    public bool AllowsMultiple { get; }
    [JsonPropertyName("allowed_authority_origins")]
    public IReadOnlyList<Uri> AllowedAuthorityOrigins { get; }

    [JsonConstructor]
    public ConnectionPolicy(
        ConnectionManagement management,
        bool allowsMultiple = true,
        IReadOnlyList<Uri>? allowedAuthorityOrigins = null)
    {
        if (!allowsMultiple)
            throw new ArgumentException(
                "Piqae hosts must not impose an artificial single-connection limit.",
                nameof(allowsMultiple));
        var normalized = (allowedAuthorityOrigins ?? Array.Empty<Uri>())
            .Select(ExactHttpsOrigin)
            .Distinct()
            .ToArray();
        if (normalized.Length > 32)
            throw new ArgumentException(
                "A connection policy can allow at most 32 authority origins.",
                nameof(allowedAuthorityOrigins));
        if (management == ConnectionManagement.HostManaged && normalized.Length == 0)
            throw new ArgumentException(
                "Host-managed connections require a pinned HTTPS authority origin.",
                nameof(allowedAuthorityOrigins));
        Management = management;
        AllowsMultiple = true;
        AllowedAuthorityOrigins = normalized;
    }

    public static ConnectionPolicy StandaloneUserManaged { get; } =
        new(ConnectionManagement.UserManaged, allowsMultiple: true);

    public void ValidateAuthority(Uri authority)
    {
        var origin = ExactHttpsOrigin(authority);
        if (AllowedAuthorityOrigins.Count > 0 && !AllowedAuthorityOrigins.Contains(origin))
            throw new ArgumentException(
                "The connection authority is outside this host's pinned policy.",
                nameof(authority));
    }

    private static Uri ExactHttpsOrigin(Uri authority)
    {
        ArgumentNullException.ThrowIfNull(authority);
        if (!authority.IsAbsoluteUri || authority.Scheme != Uri.UriSchemeHttps
            || !string.IsNullOrEmpty(authority.UserInfo)
            || (authority.AbsolutePath != "/" && authority.AbsolutePath.Length != 0)
            || !string.IsNullOrEmpty(authority.Query)
            || !string.IsNullOrEmpty(authority.Fragment))
        {
            throw new ArgumentException("Connection policies require an exact HTTPS origin.");
        }
        return new UriBuilder(Uri.UriSchemeHttps, authority.IdnHost, authority.IsDefaultPort ? -1 : authority.Port).Uri;
    }
}

public sealed record HostConfiguration
{
    [JsonPropertyName("contract")]
    public byte Contract { get; }
    [JsonPropertyName("product")]
    public NodeHostProduct Product { get; }
    [JsonPropertyName("application_id")]
    public string ApplicationId { get; }
    [JsonPropertyName("identity")]
    public NodeIdentityConfiguration Identity { get; }
    [JsonPropertyName("installed_host_policy")]
    public InstalledHostPolicy InstalledHostPolicy { get; }
    [JsonPropertyName("connection_policy")]
    public ConnectionPolicy ConnectionPolicy { get; }

    [JsonConstructor]
    public HostConfiguration(
        byte contract,
        NodeHostProduct product,
        string applicationId,
        NodeIdentityConfiguration identity,
        InstalledHostPolicy installedHostPolicy,
        ConnectionPolicy connectionPolicy)
    {
        if (contract != 1) throw new ArgumentOutOfRangeException(nameof(contract));
        ArgumentException.ThrowIfNullOrWhiteSpace(applicationId);
        if (applicationId.Length is < 3 or > 255
            || applicationId.Any(character => !char.IsLetterOrDigit(character)
                && character != '.' && character != '-'))
        {
            throw new ArgumentException(
                "Application IDs must be bounded reverse-DNS identifiers.",
                nameof(applicationId));
        }
        Contract = contract;
        Product = product;
        ApplicationId = applicationId;
        Identity = identity ?? throw new ArgumentNullException(nameof(identity));
        InstalledHostPolicy = installedHostPolicy;
        ConnectionPolicy = connectionPolicy ?? throw new ArgumentNullException(nameof(connectionPolicy));
    }

    public HostConfiguration(
        NodeHostProduct product,
        string applicationId,
        NodeIdentityConfiguration identity,
        InstalledHostPolicy installedHostPolicy,
        ConnectionPolicy connectionPolicy)
        : this(1, product, applicationId, identity, installedHostPolicy, connectionPolicy) { }
}

public static class LocalNodeNameSuggestion
{
    /// <summary>
    /// Uses only the machine's operator-visible computer name. It never reads
    /// the logged-in user, postal address, contacts, or advertising identity.
    /// </summary>
    public static string Make(string productName = "Piqae Node")
    {
        var machineName = Environment.MachineName.Trim();
        return machineName.Length == 0 ? productName : machineName[..Math.Min(machineName.Length, 120)];
    }
}
