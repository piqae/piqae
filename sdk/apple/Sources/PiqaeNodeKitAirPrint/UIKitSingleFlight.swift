#if os(iOS)
import Foundation
import PiqaeNodeKit

/// UIKit exposes process-wide print controllers. This gate serializes every
/// picker and direct handoff across adapter instances.
@MainActor
final class PiqaeUIKitPrintGate {
    static let shared = PiqaeUIKitPrintGate()

    private var occupied = false
    private var waiters: [CheckedContinuation<Void, Never>] = []

    func acquire() async {
        if !occupied {
            occupied = true
            return
        }
        await withCheckedContinuation { continuation in
            waiters.append(continuation)
        }
    }

    func release() {
        guard !waiters.isEmpty else {
            occupied = false
            return
        }
        waiters.removeFirst().resume()
    }
}

/// UIKit may report `begin == false` and still race a completion callback.
/// This wrapper guarantees a checked continuation is resumed exactly once.
@MainActor
final class PiqaeUIKitOneShot<Value: Sendable> {
    private var continuation: CheckedContinuation<Value, Error>?

    init(_ continuation: CheckedContinuation<Value, Error>) {
        self.continuation = continuation
    }

    @discardableResult
    func resume(returning value: Value) -> Bool {
        guard let continuation else { return false }
        self.continuation = nil
        continuation.resume(returning: value)
        return true
    }

    @discardableResult
    func resume(throwing error: any Error) -> Bool {
        guard let continuation else { return false }
        self.continuation = nil
        continuation.resume(throwing: error)
        return true
    }
}
#endif
