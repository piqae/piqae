using Piqae.Node;

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
    public void ConcurrentDisposeAndCommandsAreSerialized()
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
        Task.WaitAll(commands.Append(dispose).ToArray());
        Assert.Throws<ObjectDisposedException>(node.Snapshot);
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
    public void HostKeyCreationIsStableAcrossParallelProvidersAndRestart()
    {
        if (!OperatingSystem.IsWindows()) return;
        var installation = $"tests-{Guid.NewGuid():N}";
        var providers = Enumerable.Range(0, 16)
            .Select(_ => new WindowsCredentialHostKeyProvider(installation))
            .ToArray();
        var digests = providers
            .Select(provider => Task.Run(() => Convert.ToHexString(provider.HmacSha256("printer-identity", "fixture"u8))))
            .ToArray();
        Task.WaitAll(digests);
        Assert.Single(digests.Select(task => task.Result).Distinct());
        var restarted = new WindowsCredentialHostKeyProvider(installation);
        Assert.Equal(digests[0].Result, Convert.ToHexString(restarted.HmacSha256("printer-identity", "fixture"u8)));
    }
}
