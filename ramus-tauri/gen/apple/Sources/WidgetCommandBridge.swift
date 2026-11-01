import Foundation

/// Exported by the Rust core (`ramus_tauri::widget::ffi`). Blocks until the
/// command has run, waiting up to `timeoutMs` for the app to finish
/// launching first.
@_silgen_name("ramus_widget_command")
private func ramus_widget_command(_ json: UnsafePointer<CChar>, _ timeoutMs: UInt32) -> Int32

/// Hands widget button commands to the Rust core off the calling thread.
enum WidgetCommandBridge {
    /// Long enough for a cold launch to restore the session and queue.
    static let timeoutMs: UInt32 = 20_000

    static func send(_ json: String) async -> Int32 {
        await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let code = json.withCString { ramus_widget_command($0, timeoutMs) }
                continuation.resume(returning: code)
            }
        }
    }
}
