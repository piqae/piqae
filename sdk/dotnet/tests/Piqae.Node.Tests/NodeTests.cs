using Piqae.Node;
using Org.BouncyCastle.Math.EC.Rfc8032;
using System.Security.Cryptography;
using Xunit;

namespace Piqae.Node.Tests;

public sealed class NodeTests
{
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
        using var node = new PiqaeNode(new(
            HostMode.EmbeddedApplication,
            AvailabilityClass.ForegroundOnly,
            true,
            "com.piqae.tests",
            unique));
        Assert.True(node.Start().GetProperty("started").GetBoolean());
        var suspended = node.ApplyLifecycle(LifecycleEvent.SuspendImminent);
        Assert.False(suspended.GetProperty("lifecycle").GetProperty("accepting_cloud_leases").GetBoolean());
        Assert.False(node.Stop().GetProperty("started").GetBoolean());
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
            PiqaeNode.EnsureCompatibleAbi(new NativeAbiDescriptor(1, 2, 2)));
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
}
