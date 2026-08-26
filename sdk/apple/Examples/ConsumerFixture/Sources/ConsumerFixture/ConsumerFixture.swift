import PiqaeNodeKit

public enum ConsumerFixture {
    /// A minimal app-scoped node used to prove an independent package can
    /// consume the public Swift facade on every supported Apple platform.
    public static func makeNode() -> PiqaeNode {
        PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                availability: .backgroundOpportunistic
            )
        )
    }
}
