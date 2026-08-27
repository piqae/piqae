using System.Text.Json;
using System.Text.Json.Serialization;

namespace Piqae.Node;

public abstract record PrintPacketOutputTarget
{
    private PrintPacketOutputTarget() { }

    public sealed record Pdf(string Profile = "printpacket.pdf-base14/v1")
        : PrintPacketOutputTarget;

    public sealed record PrinterNative(
        string Language,
        string Profile,
        ushort Dpi,
        uint PrintableWidthDots) : PrintPacketOutputTarget;

    internal object ToWireValue() => this switch
    {
        Pdf pdf => new { kind = "pdf", profile = pdf.Profile },
        PrinterNative native => new
        {
            kind = "printer_native",
            language = native.Language,
            profile = native.Profile,
            dpi = native.Dpi,
            printable_width_dots = native.PrintableWidthDots
        },
        _ => throw new ArgumentOutOfRangeException(nameof(PrintPacketOutputTarget))
    };
}

public sealed record PrintPacket
{
    public JsonElement Template { get; }
    public JsonElement Data { get; }
    public IReadOnlyDictionary<string, byte[]> Resources { get; }
    public PrintPacketOutputTarget OutputTarget { get; }

    public PrintPacket(
        JsonElement template,
        JsonElement data,
        IReadOnlyDictionary<string, byte[]>? resources = null,
        PrintPacketOutputTarget? outputTarget = null)
    {
        if (template.ValueKind != JsonValueKind.Object)
            throw new ArgumentException("A PrintPacket template must be a JSON object.", nameof(template));
        Template = template.Clone();
        Data = data.Clone();
        Resources = resources?.ToDictionary(
            pair => pair.Key,
            pair => pair.Value.ToArray(),
            StringComparer.Ordinal) ?? new Dictionary<string, byte[]>();
        OutputTarget = outputTarget ?? new PrintPacketOutputTarget.Pdf();
    }

    public static PrintPacket Parse(
        ReadOnlySpan<byte> templateJson,
        ReadOnlySpan<byte> dataJson,
        IReadOnlyDictionary<string, byte[]>? resources = null,
        PrintPacketOutputTarget? outputTarget = null)
    {
        using var template = JsonDocument.Parse(templateJson.ToArray());
        using var data = JsonDocument.Parse(dataJson.ToArray());
        return new PrintPacket(template.RootElement, data.RootElement, resources, outputTarget);
    }
}

public sealed record PrintPacketManifest
{
    [JsonPropertyName("standard")] public required string Standard { get; init; }
    [JsonPropertyName("specification_version")] public required string SpecificationVersion { get; init; }
    [JsonPropertyName("canonical_json")] public required string CanonicalJson { get; init; }
    [JsonPropertyName("canonical_sha256")] public required string CanonicalSha256 { get; init; }
    [JsonPropertyName("canonical_bytes")] public ulong CanonicalBytes { get; init; }
    [JsonPropertyName("required_features")] public required IReadOnlyList<string> RequiredFeatures { get; init; }
    [JsonPropertyName("resource_count")] public uint ResourceCount { get; init; }
    [JsonPropertyName("resource_bytes")] public ulong ResourceBytes { get; init; }
}

public sealed record PrintPacketOutput
{
    [JsonPropertyName("media_type")] public required string MediaType { get; init; }
    [JsonPropertyName("profile")] public required string Profile { get; init; }
    [JsonPropertyName("sha256")] public required string Sha256 { get; init; }
    [JsonPropertyName("bytes")] public ulong Bytes { get; init; }
    [JsonPropertyName("pages")] public uint Pages { get; init; }
}

public sealed record PrintPacketValidation
{
    [JsonPropertyName("manifest")] public required PrintPacketManifest Manifest { get; init; }
    [JsonPropertyName("cache_key")] public required string CacheKey { get; init; }
    [JsonPropertyName("output")] public required PrintPacketOutput Output { get; init; }
}

public sealed record PrintPacketJob
{
    [JsonPropertyName("job_id")] public required string JobId { get; init; }
    [JsonPropertyName("state")] public required string State { get; init; }
}

public sealed record PrintPacketSubmission
{
    [JsonPropertyName("job")] public required PrintPacketJob Job { get; init; }
    [JsonPropertyName("manifest")] public required PrintPacketManifest Manifest { get; init; }
    [JsonPropertyName("cache_key")] public required string CacheKey { get; init; }
    [JsonPropertyName("output")] public required PrintPacketOutput Output { get; init; }
}
