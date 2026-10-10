import AuthenticationServices
import UIKit
import os

private let log = Logger(subsystem: "com.raspsoft.ramus", category: "web-auth")

/// Shows a sign-in page in an `ASWebAuthenticationSession` sheet over the
/// app, so signing in never leaves it.
///
/// The session is created without a callback scheme, so it never completes
/// on its own: the page links a PIN that the app polls for, and the app
/// closes the sheet with `dismiss()` once the token lands. The completion
/// handler therefore runs only when the user closes the sheet or the
/// session fails to start, and that is what `onClosed` reports.
///
/// Main queue only.
final class WebAuthSheet: NSObject, ASWebAuthenticationPresentationContextProviding {
    private var session: ASWebAuthenticationSession?
    private weak var anchor: UIWindow?

    func present(url: URL, anchor: UIWindow, onClosed: @escaping () -> Void) {
        dismiss()
        self.anchor = anchor
        var current: ASWebAuthenticationSession?
        let session = ASWebAuthenticationSession(url: url, callbackURLScheme: nil) {
            [weak self] _, error in
            // A session the app dismissed or replaced is already forgotten,
            // and whether `cancel()` runs this handler varies by iOS version.
            guard let self, let current, self.session === current else { return }
            self.session = nil
            if let error {
                log.info("sign-in sheet closed: \(error.localizedDescription, privacy: .public)")
            }
            onClosed()
        }
        current = session
        session.presentationContextProvider = self
        self.session = session
        if !session.start() {
            log.error("sign-in sheet failed to start")
            self.session = nil
            onClosed()
        }
    }

    func dismiss() {
        guard let session else { return }
        self.session = nil
        session.cancel()
    }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        anchor ?? ASPresentationAnchor()
    }
}
