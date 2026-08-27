using Piqae.Node;
using Org.BouncyCastle.Math.EC.Rfc8032;
using System.Security.Cryptography;
using System.Text.Json;
using Xunit;

namespace Piqae.Node.Tests;

public sealed class NodeTests
{
    [Fact]
    public void ApplicationIdentifiersMatchSharedContractFixture()
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        while (directory is not null && !Directory.Exists(Path.Combine(directory.FullName, "contracts")))
            directory = directory.Parent;
        Assert.NotNull(directory);
        var fixture = JsonSerializer.Deserialize<ApplicationIdFixture>(File.ReadAllText(Path.Combine(
            directory!.FullName,
            "contracts",
            "fixtures",
            "node-host-application-ids.json")));
        Assert.NotNull(fixture);

        foreach (var applicationId in fixture.Valid)
            _ = new HostConfiguration(
                NodeHostProduct.Standalone,
                applicationId,
                new NodeIdentityConfiguration("Fixture node"),
                InstalledHostPolicy.IsolatedApplication,
                new ConnectionPolicy(ConnectionManagement.UserManaged));
        foreach (var applicationId in fixture.Invalid)
            Assert.Throws<ArgumentException>(() => new HostConfiguration(
                NodeHostProduct.Standalone,
                applicationId,
                new NodeIdentityConfiguration("Fixture node"),
                InstalledHostPolicy.IsolatedApplication,
                new ConnectionPolicy(ConnectionManagement.UserManaged)));
    }

    [Fact]
    public void HostConfigurationMatchesPortableContractAndAllowsManyConnections()
    {
        var configuration = new HostConfiguration(
            NodeHostProduct.Embedded,
            "com.example.pos",
            new NodeIdentityConfiguration(
                "Dispatch PC",
                site: "Warehouse",
                location: "Desk 4",
                labels: ["shipping", "backup"]),
            InstalledHostPolicy.PreferInstalled,
            new ConnectionPolicy(
                ConnectionManagement.HostManaged,
                allowsMultiple: true,
                allowedAuthorityOrigins: [new Uri("https://api.piqae.com")]));

        var json = JsonSerializer.Serialize(configuration);
        var restored = JsonSerializer.Deserialize<HostConfiguration>(json);

        Assert.NotNull(restored);
        Assert.Equal((byte)1, restored.Contract);
        Assert.Equal(NodeHostProduct.Embedded, restored.Product);
        Assert.True(restored.ConnectionPolicy.AllowsMultiple);
        Assert.Contains("\"installed_host_policy\":\"prefer_installed\"", json);
        Assert.Contains("\"management\":\"host_managed\"", json);
        restored.ConnectionPolicy.ValidateAuthority(new Uri("https://api.piqae.com"));
        Assert.Throws<ArgumentException>(() =>
            restored.ConnectionPolicy.ValidateAuthority(new Uri("https://other.example")));
    }

    [Fact]
    public void HostIdentityIsBoundedAndNeverReadsUserIdentity()
    {
        var suggestion = LocalNodeNameSuggestion.Make();
        Assert.NotEmpty(suggestion);
        Assert.True(System.Text.Encoding.UTF8.GetByteCount(suggestion) <= 120);
        Assert.Throws<ArgumentException>(() => new NodeIdentityConfiguration(
            "Node", labels: Enumerable.Repeat("duplicate", 2).ToArray()));
        Assert.Throws<ArgumentException>(() => new ConnectionPolicy(
            ConnectionManagement.HostManaged,
            allowedAuthorityOrigins: []));
        Assert.Throws<ArgumentException>(() => new ConnectionPolicy(
            ConnectionManagement.UserManaged,
            allowsMultiple: false));
        Assert.Throws<ArgumentException>(() => new NodeIdentityConfiguration("Node\0hidden"));
        Assert.Throws<ArgumentException>(() => new ConnectionPolicy(
            ConnectionManagement.HostManaged,
            allowedAuthorityOrigins:
            [
                new Uri("https://api.piqae.com"),
                new Uri("https://api.piqae.com/"),
            ]));
        foreach (var invalid in new[] { ".com.example", "-com.example", "éxample.com" })
            Assert.Throws<ArgumentException>(() => new HostConfiguration(
                NodeHostProduct.Embedded,
                invalid,
                new NodeIdentityConfiguration("Node"),
                InstalledHostPolicy.PreferInstalled,
                new ConnectionPolicy(ConnectionManagement.UserManaged)));
        Assert.Throws<ArgumentException>(() => new HostConfiguration(
            NodeHostProduct.Standalone,
            "com.example.standalone",
            new NodeIdentityConfiguration("Node"),
            InstalledHostPolicy.IsolatedApplication,
            new ConnectionPolicy(
                ConnectionManagement.HostManaged,
                allowedAuthorityOrigins: [new Uri("https://api.piqae.com")])));
    }

    [Fact]
    public void EndpointIsDeterministicAndLocal()
    {
        var first = PiqaeBrokerClient.EndpointForDataDirectory(@"C:\Users\example\AppData\Local\Spool");
        Assert.Equal(first, PiqaeBrokerClient.EndpointForDataDirectory(@"C:\Users\example\AppData\Local\Spool"));
        Assert.StartsWith(@"\\.\pipe\piqae-node-", first);
    }

    [Fact]
    public void NativeInstanceLifecycleIsSafeAndInstanceScoped()
    {
        if (!OperatingSystem.IsWindows() || Environment.GetEnvironmentVariable("PIQAE_NODE_NATIVE_TEST") != "1") return;
        var unique = $"tests-{Guid.NewGuid():N}";
        var hostConfiguration = new HostConfiguration(
            NodeHostProduct.Embedded,
            "com.piqae.tests",
            new NodeIdentityConfiguration("Windows SDK node"),
            InstalledHostPolicy.IsolatedApplication,
            new ConnectionPolicy(ConnectionManagement.UserManaged));
        using var node = new PiqaeNode(new(
            HostMode.EmbeddedApplication,
            AvailabilityClass.ForegroundOnly,
            true,
            "com.piqae.tests",
            unique,
            hostConfiguration));
        Assert.True(node.Start().GetProperty("started").GetBoolean());
        var suspended = node.ApplyLifecycle(LifecycleEvent.SuspendImminent);
        Assert.False(suspended.GetProperty("lifecycle").GetProperty("accepting_cloud_leases").GetBoolean());
        Assert.True(node.ReconcileCloud(TimeSpan.FromSeconds(1)));
        var reconciliation = node.ReconcileCloudOutcome(TimeSpan.FromSeconds(1));
        Assert.False(reconciliation.CloudConfigured);
        Assert.True(reconciliation.LoopCompleted);
        Assert.Equal(CloudReconcileSuccessScope.All, reconciliation.SuccessScope);
        Assert.Throws<ArgumentOutOfRangeException>(() => node.ReconcileCloud(TimeSpan.Zero));
        var identity = node.UpdateNodeIdentity(
            1,
            new NodeIdentityConfiguration("Dispatch PC", site: "Warehouse", labels: ["sdk"]));
        Assert.Equal((ulong)2, identity.Revision);
        var conflict = Assert.Throws<PiqaeNodeException>(() => node.UpdateNodeIdentity(
            1,
            new NodeIdentityConfiguration("Stale PC")));
        Assert.Equal("node_identity_revision_conflict", conflict.Code);
        Assert.Equal((ulong)2, conflict.CurrentRevision);
        Assert.False(node.Stop().GetProperty("started").GetBoolean());
    }

    [Fact]
    public void PrintPacketDefaultsToPortablePdfAndOwnsItsJsonAndResources()
    {
        var bytes = new byte[] { 1, 2, 3 };
        var packet = PrintPacket.Parse(
            """{"format":"printpacket/v1","media":{"kind":"continuous","width_mm":80},"body":[]}"""u8,
            "{}"u8,
            new Dictionary<string, byte[]> { ["logo"] = bytes });
        bytes[0] = 99;

        Assert.IsType<PrintPacketOutputTarget.Pdf>(packet.OutputTarget);
        Assert.Equal(1, packet.Resources["logo"][0]);
        Assert.Equal("printpacket/v1", packet.Template.GetProperty("format").GetString());
        Assert.Throws<ArgumentException>(() => PrintPacket.Parse(
            """{"format":"printpacket/v0","media":{"kind":"continuous","width_mm":80},"body":[]}"""u8,
            "{}"u8));
    }

    [Fact]
    public void NativePrintPacketFacadeValidatesReceiptAndLabelAndSubmitsIdempotentlyOffline()
    {
        if (!OperatingSystem.IsWindows() || Environment.GetEnvironmentVariable("PIQAE_NODE_NATIVE_TEST") != "1") return;
        var unique = $"printpacket-{Guid.NewGuid():N}";
        using var node = new PiqaeNode(new(
            HostMode.EmbeddedApplication,
            AvailabilityClass.ForegroundOnly,
            true,
            "com.piqae.tests.printpacket",
            unique));
        node.Start();
        using var adapterCapabilities = JsonDocument.Parse("""{"document_kinds":["pdf"]}""");
        node.RegisterAdapter(new AdapterRegistration(
            new AdapterFingerprint(
                "windows",
                "com.piqae.tests.fake-printer",
                "1.0.0",
                "test",
                null),
            adapterCapabilities.RootElement.Clone()));
        using var nativeOptions = JsonDocument.Parse("{}");
        var inventory = node.ObservePrinterInventory(
            "com.piqae.tests.fake-printer",
            [new PrinterObservation(
                "virtual://printpacket",
                "Virtual PrintPacket printer",
                "available",
                true,
                nativeOptions.RootElement.Clone())]);
        var printerId = inventory.GetProperty("printers")[0].GetProperty("printer_id").GetString();
        Assert.False(string.IsNullOrEmpty(printerId));

        var receipt = ReadPrintPacketFixture("receipt-80mm");
        var label = ReadPrintPacketFixture("production-label-100x50");
        var capabilities = node.GetPrintPacketCapabilities();
        Assert.Equal("printpacket/v1", capabilities.Contract);
        Assert.Equal("printpacket.pdf-renderer/v1", capabilities.RendererAbi);
        Assert.Equal("printpacket.resources/v1", capabilities.ResourceAbi);
        Assert.Equal("printpacket.render-cache/v1", capabilities.CacheProfile);
        Assert.True(capabilities.DirectOfflineRendering);
        Assert.True(capabilities.HardLimits.MaxPages > 0);
        var receiptValidation = node.ValidatePrintPacket(receipt);
        var labelValidation = node.ValidatePrintPacket(label);
        Assert.Equal("printpacket/v1", receiptValidation.Manifest.SpecificationVersion);
        Assert.Equal("application/pdf", receiptValidation.Output.MediaType);
        Assert.Equal("application/pdf", labelValidation.Output.MediaType);
        Assert.NotEqual(receiptValidation.CacheKey, labelValidation.CacheKey);

        var first = node.EnqueuePrintPacket(
            "com.piqae.tests.fake-printer",
            "receipt-1042-copy-1",
            printerId!,
            "Receipt 1042",
            receipt);
        var second = node.EnqueuePrintPacket(
            "com.piqae.tests.fake-printer",
            "receipt-1042-copy-1",
            printerId!,
            "Receipt 1042",
            receipt);
        Assert.Equal(first.Job.JobId, second.Job.JobId);
        Assert.Equal(first.Output.Sha256, second.Output.Sha256);
        var operation = node.NextAdapterOperation("com.piqae.tests.fake-printer");
        Assert.Equal("pdf", operation.GetProperty("operation").GetProperty("content_kind").GetString());
        Assert.Equal(
            JsonValueKind.Null,
            node.NextAdapterOperation("com.piqae.tests.fake-printer")
                .GetProperty("operation")
                .ValueKind);

        var unsupported = new PrintPacket(
            receipt.Template,
            receipt.Data,
            outputTarget: new PrintPacketOutputTarget.PrinterNative(
                "zpl",
                "zpl-raster/v1",
                203,
                812));
        var exception = Assert.Throws<PiqaeNodeException>(() => node.ValidatePrintPacket(unsupported));
        Assert.Equal("printpacket_unsupported_target", exception.Code);
    }

    [Fact]
    public void NativeAndPortableApplicationIdentifiersMustMatch()
    {
        var hostConfiguration = new HostConfiguration(
            NodeHostProduct.Embedded,
            "com.example.one",
            new NodeIdentityConfiguration("Node"),
            InstalledHostPolicy.IsolatedApplication,
            new ConnectionPolicy(ConnectionManagement.UserManaged));
        Assert.Throws<ArgumentException>(() => new PiqaeNode(new(
            HostMode.EmbeddedApplication,
            AvailabilityClass.ForegroundOnly,
            true,
            "com.example.two",
            "state",
            hostConfiguration)));
    }

    [Fact]
    public void OnlyExplicitPrintPacketCoreErrorRequiresUpdate()
    {
        var exception = PiqaeNodeException.FromNative(
            "printpacket_core_update_required",
            "The renderer is too old.");

        Assert.Equal("native_core_update_required", exception.Code);
        Assert.Equal("printpacket_core_update_required", exception.NativeCode);
        Assert.Contains("must be updated", exception.Message, StringComparison.Ordinal);
        var invalid = PiqaeNodeException.FromNative(
            "invalid_command",
            "The command is not recognized.");
        Assert.Equal("invalid_command", invalid.Code);
    }

    private static PrintPacket ReadPrintPacketFixture(string name)
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        while (directory is not null && !Directory.Exists(Path.Combine(directory.FullName, "standards")))
            directory = directory.Parent;
        Assert.NotNull(directory);
        using var fixture = JsonDocument.Parse(File.ReadAllBytes(Path.Combine(
            directory!.FullName,
            "standards",
            "printpacket",
            "conformance",
            $"{name}.json")));
        return new PrintPacket(
            fixture.RootElement.GetProperty("template"),
            fixture.RootElement.GetProperty("data"));
    }

    [Fact]
    public async Task ConcurrentDisposeAndCommandsAreSerialized()
    {
        if (!OperatingSystem.IsWindows() || Environment.GetEnvironmentVariable("PIQAE_NODE_NATIVE_TEST") != "1") return;
        var node = new PiqaeNode(new(
            HostMode.EmbeddedApplication,
            AvailabilityClass.ForegroundOnly,
            true,
            "com.piqae.tests",
            $"tests-{Guid.NewGuid():N}"));
        node.Start();
        var commands = Enumerable.Range(0, 32)
            .Select(_ => Task.Run(() =>
            {
                try { node.Snapshot(); }
                catch (ObjectDisposedException) { }
            }))
            .ToArray();
        var dispose = Task.Run(node.Dispose);
        await Task.WhenAll(commands.Append(dispose));
        Assert.Throws<ObjectDisposedException>(() => { _ = node.Snapshot(); });
    }

    [Fact]
    public void ApprovedCapabilityCanRoundTripThroughWindowsCredentialManager()
    {
        if (!OperatingSystem.IsWindows()) return;
        var target = $"Piqae.Node/tests/{Guid.NewGuid():N}";
        try
        {
            WindowsCredentialStore.Write(target, "bounded-test-capability");
            Assert.Equal("bounded-test-capability", WindowsCredentialStore.Read(target));
        }
        finally { WindowsCredentialStore.Delete(target); }
    }

    [Fact]
    public async Task HostKeyCreationIsStableAcrossParallelProvidersAndRestart()
    {
        if (!OperatingSystem.IsWindows()) return;
        var installation = $"tests-{Guid.NewGuid():N}";
        var providers = Enumerable.Range(0, 16)
            .Select(_ => new WindowsCredentialHostKeyProvider(installation))
            .ToArray();
        try
        {
            var digests = await Task.WhenAll(providers.Select(provider => Task.Run(() =>
                Convert.ToHexString(provider.HmacSha256("printer-identity", "fixture"u8)))));
            Assert.Single(digests.Distinct());
            var restarted = new WindowsCredentialHostKeyProvider(installation);
            Assert.Equal(digests[0], Convert.ToHexString(restarted.HmacSha256("printer-identity", "fixture"u8)));
        }
        finally { providers[0].DeleteForTests("printer-identity"); }
    }

    [Fact]
    public void NativeAbiMismatchFailsClosed()
    {
        var exception = Assert.Throws<PiqaeNodeException>(() =>
            PiqaeNode.EnsureCompatibleAbi(new NativeAbiDescriptor(2, 1, 1)));
        Assert.Equal("unsupported_native_abi", exception.Code);
        Assert.Throws<PiqaeNodeException>(() =>
            PiqaeNode.EnsureCompatibleAbi(new NativeAbiDescriptor(1, 1, 1)));
        Assert.Throws<PiqaeNodeException>(() =>
            PiqaeNode.EnsureCompatibleAbi(new NativeAbiDescriptor(1, 3, 3)));
        PiqaeNode.EnsureCompatibleAbi(new NativeAbiDescriptor(1, 2, 2));
    }

    [Fact]
    public void SecretBearingSdkValuesAreRedactedFromDiagnosticStrings()
    {
        const string secret = "fixture-secret-must-not-escape";
        var credential = new BrokerCredential("com.piqae.tests", secret);
        var invitation = new PiqaeConnectorInvitation(
            new Uri("https://api.piqae.test"),
            secret,
            "wcred-ed25519-v1.connector.0123456789abcdef",
            PiqaePrinterGrant.AllLocalPrinters,
            Array.Empty<string>(),
            "Test node",
            "test-host");

        Assert.DoesNotContain(secret, credential.ToString(), StringComparison.Ordinal);
        Assert.DoesNotContain(secret, invitation.ToString(), StringComparison.Ordinal);
        Assert.Contains("[REDACTED]", credential.ToString(), StringComparison.Ordinal);
        Assert.Contains("[REDACTED]", invitation.ToString(), StringComparison.Ordinal);
        Assert.DoesNotContain(
            typeof(PiqaeNode).GetMethods(),
            method => method.Name.Contains("CompleteConnector", StringComparison.Ordinal));
    }

    [Fact]
    public void UnsafeInvitationOriginIsRejectedBeforeNativeExchange()
    {
        if (!OperatingSystem.IsWindows() || Environment.GetEnvironmentVariable("PIQAE_NODE_NATIVE_TEST") != "1") return;
        using var node = new PiqaeNode(new(
            HostMode.EmbeddedApplication,
            AvailabilityClass.ForegroundOnly,
            true,
            "com.piqae.tests",
            $"tests-{Guid.NewGuid():N}"));
        node.Start();
        var invitation = new PiqaeConnectorInvitation(
            new Uri("http://attacker.invalid"),
            "fixture-invitation-token",
            "wcred-ed25519-v1.connector.0123456789abcdef",
            PiqaePrinterGrant.SelectedPrinters,
            new[] { "printer-one" },
            "Test node",
            "test-host");

        Assert.Throws<ArgumentException>(() => { _ = node.Connect(invitation); });
    }

    [Fact]
    public void ConnectorProviderCallbackFailureIsRedactedAndFailsClosed()
    {
        if (!OperatingSystem.IsWindows() || Environment.GetEnvironmentVariable("PIQAE_NODE_NATIVE_TEST") != "1") return;
        const string secret = "provider-secret-must-not-escape";
        using var node = new PiqaeNode(new(
            HostMode.EmbeddedApplication,
            AvailabilityClass.ForegroundOnly,
            false,
            "com.piqae.tests",
            $"tests-{Guid.NewGuid():N}"), new ThrowingConnectorKeyProvider(secret));

        var failure = Assert.Throws<PiqaeNodeException>(() => { _ = node.Start(); });
        Assert.Equal("secure_connector_provider_required", failure.Code);
        Assert.DoesNotContain(secret, failure.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void PreparedConnectorKeyCanBeCancelledAcrossProviderRestart()
    {
        if (!OperatingSystem.IsWindows() || Environment.GetEnvironmentVariable("PIQAE_NODE_NATIVE_TEST") != "1") return;
        var installation = $"tests-{Guid.NewGuid():N}";
        var provider = new WindowsCredentialConnectorKeyProvider(installation);
        PreparedConnectorInvitation? prepared = null;
        try
        {
            using var node = new PiqaeNode(new(
                HostMode.EmbeddedApplication,
                AvailabilityClass.ForegroundOnly,
                false,
                "com.piqae.tests",
                $"tests-{Guid.NewGuid():N}"), provider);
            node.Start();
            prepared = node.PrepareConnectorInvitation();
            Assert.True(prepared.ExpiresUnixMs > DateTimeOffset.UtcNow.ToUnixTimeMilliseconds());
            Assert.Equal(43, prepared.PublicKeyBase64.Length);
            Assert.DoesNotContain('=', prepared.PublicKeyBase64);
            Assert.True(node.CancelPreparedConnectorInvitation(prepared.KeyHandle));
            Assert.False(node.CancelPreparedConnectorInvitation(prepared.KeyHandle));

            var restarted = new WindowsCredentialConnectorKeyProvider(installation);
            Assert.Throws<CryptographicException>(() => restarted.Sign(prepared.KeyHandle, "cancelled"u8));
        }
        finally
        {
            if (prepared is not null) provider.Delete(prepared.KeyHandle);
            provider.DeleteInstallationForTests("installation/com.piqae.tests");
        }
    }

    [Fact]
    public async Task ConnectorKeyPersistsAcrossProvidersAndDeleteIsIdempotent()
    {
        if (!OperatingSystem.IsWindows()) return;
        var installation = $"tests-{Guid.NewGuid():N}";
        var scope = $"connector/com.piqae.tests/{Guid.NewGuid():D}";
        var provider = new WindowsCredentialConnectorKeyProvider(installation);
        var generated = provider.Generate(scope);
        try
        {
            var message = "piqae-connector-bind-v1\0connector-test\0nonce-test"u8.ToArray();
            var restarted = new WindowsCredentialConnectorKeyProvider(installation);
            var signatures = await Task.WhenAll(Enumerable.Range(0, 16)
                .Select(_ => Task.Run(() => restarted.Sign(generated.Handle, message))));
            foreach (var signature in signatures)
                Assert.True(Ed25519.Verify(signature, 0, generated.PublicKey, 0, message, 0, message.Length));

            restarted.Delete(generated.Handle);
            restarted.Delete(generated.Handle);
            Assert.Throws<CryptographicException>(() => restarted.Sign(generated.Handle, message));
        }
        finally { provider.Delete(generated.Handle); }
    }

    [Fact]
    public void StableInstallationKeyIsDistinctAndCannotBeRevokedAsConnector()
    {
        if (!OperatingSystem.IsWindows()) return;
        var installation = $"tests-{Guid.NewGuid():N}";
        var scope = "installation/com.piqae.tests";
        var provider = new WindowsCredentialConnectorKeyProvider(installation);
        try
        {
            var first = provider.Generate(scope);
            var restarted = new WindowsCredentialConnectorKeyProvider(installation);
            var second = restarted.Generate(scope);
            Assert.Equal(first.Handle, second.Handle);
            Assert.Equal(first.PublicKey, second.PublicKey);
            Assert.Throws<InvalidOperationException>(() => restarted.Delete(first.Handle));

            var connector = restarted.Generate($"connector/com.piqae.tests/{Guid.NewGuid():D}");
            try
            {
                restarted.Delete(connector.Handle);
                var signature = restarted.Sign(first.Handle, "continuity"u8);
                Assert.True(Ed25519.Verify(signature, 0, first.PublicKey, 0, "continuity"u8.ToArray(), 0, "continuity"u8.Length));
            }
            finally { restarted.Delete(connector.Handle); }
        }
        finally { provider.DeleteInstallationForTests(scope); }
    }

    private sealed class ThrowingConnectorKeyProvider(string secret) : IPiqaeConnectorKeyProvider
    {
        public PiqaeGeneratedConnectorKey Generate(string scope) => throw new InvalidOperationException(secret);
        public byte[] Sign(string handle, ReadOnlySpan<byte> message) => throw new InvalidOperationException(secret);
        public void Delete(string handle) => throw new InvalidOperationException(secret);
    }
}

file sealed record ApplicationIdFixture(
    [property: System.Text.Json.Serialization.JsonPropertyName("valid")] string[] Valid,
    [property: System.Text.Json.Serialization.JsonPropertyName("invalid")] string[] Invalid);
