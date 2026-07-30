import Foundation
import PiqaePrintCoreReplayCore

@main
struct ReplayMain {
    @MainActor
    static func main() {
        let response: PrintCoreReplayResponse
        let exitCode: Int32

        do {
            let input = try readBoundedInput()
            guard !input.isEmpty else {
                throw PrintCoreReplayError.failure(
                    code: "invalid_request",
                    message: "expected one JSON request on stdin"
                )
            }
            guard input.count <= PrintCoreReplayValidator.maximumRequestBytes else {
                throw PrintCoreReplayError.failure(
                    code: "request_too_large",
                    message: "request exceeds the 2 MiB input limit"
                )
            }
            let request: PrintCoreReplayRequest
            do {
                request = try JSONDecoder().decode(PrintCoreReplayRequest.self, from: input)
            } catch {
                throw PrintCoreReplayError.failure(
                    code: "invalid_request",
                    message: "stdin is not a valid PrintCore replay request"
                )
            }
            response = try PrintCoreReplayer.replay(request)
            exitCode = 0
        } catch let error as PrintCoreReplayError {
            response = error.response
            exitCode = 1
        } catch {
            response = PrintCoreReplayResponse(
                ok: false,
                code: "internal",
                message: "unexpected PrintCore replay failure",
                retryable: false,
                handoffMayHaveSucceeded: false
            )
            exitCode = 1
        }

        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        if let encoded = try? encoder.encode(response) {
            FileHandle.standardOutput.write(encoded)
            FileHandle.standardOutput.write(Data([0x0A]))
        } else {
            FileHandle.standardOutput.write(
                Data(
                    #"{"code":"internal","handoff_may_have_succeeded":false,"message":"response encoding failed","ok":false,"retryable":false}"#
                        .utf8
                )
            )
            FileHandle.standardOutput.write(Data([0x0A]))
        }
        exit(exitCode)
    }

    private static func readBoundedInput() throws -> Data {
        let limit = PrintCoreReplayValidator.maximumRequestBytes
        var input = Data()
        while input.count <= limit {
            let remaining = limit + 1 - input.count
            let chunk = try FileHandle.standardInput.read(
                upToCount: min(64 * 1024, remaining)
            ) ?? Data()
            guard !chunk.isEmpty else { break }
            input.append(chunk)
        }
        return input
    }
}
