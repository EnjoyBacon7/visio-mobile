import AuthenticationServices
import Security
import SwiftUI
import UIKit

// MARK: - OidcAuthManager

class OidcAuthManager: NSObject, ObservableObject, ASWebAuthenticationPresentationContextProviding {

    /// Called when the OIDC flow completes (cookie or nil).
    var onComplete: ((String?) -> Void)?

    /// Tracks the active session so it isn't deallocated mid-flow.
    private var authSession: ASWebAuthenticationSession?

    /// The callback URL scheme registered in Info.plist.
    private static let callbackScheme = "visio"

    // MARK: - ASWebAuthenticationSession Flow

    /// Launches the OIDC authentication flow using ASWebAuthenticationSession.
    ///
    /// Flow:
    /// 1. Opens system Safari sheet with the authenticate endpoint
    /// 2. Server performs OIDC, sets sessionid cookie on `{instance}` domain
    /// 3. Server redirects to `visio://auth-callback`
    /// 4. ASWebAuthenticationSession intercepts the custom-scheme redirect and returns
    /// 5. App reads sessionid cookie from HTTPCookieStorage.shared
    /// 6. Validates session via /api/v1.0/users/me/
    func launchOidcFlow(meetInstance: String, completion: @escaping (String?) -> Void) {
        onComplete = completion

        let returnTo = "visio://auth-callback"
        let encodedReturnTo = returnTo.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) ?? returnTo
        guard let authURL = URL(string: "https://\(meetInstance)/api/v1.0/authenticate/?returnTo=\(encodedReturnTo)") else {
            completion(nil)
            return
        }

        let session = ASWebAuthenticationSession(
            url: authURL,
            callbackURLScheme: Self.callbackScheme
        ) { [weak self] callbackURL, error in
            guard let self else { return }
            self.authSession = nil

            if let error {
                // User cancelled or other error
                print("[OidcAuthManager] ASWebAuthenticationSession error: \(error.localizedDescription)")
                DispatchQueue.main.async { self.onComplete?(nil); self.onComplete = nil }
                return
            }

            // Session completed — the server set the sessionid cookie before redirecting.
            // Read it from the shared cookie storage.
            self.extractSessionCookie(meetInstance: meetInstance)
        }

        // Share cookies with Safari so existing sessions are reused
        // and the sessionid cookie lands in HTTPCookieStorage.shared.
        session.prefersEphemeralWebBrowserSession = false
        session.presentationContextProvider = self

        authSession = session
        session.start()
    }

    // MARK: - Cookie Extraction

    private func extractSessionCookie(meetInstance: String) {
        guard let instanceURL = URL(string: "https://\(meetInstance)/") else {
            DispatchQueue.main.async { self.onComplete?(nil); self.onComplete = nil }
            return
        }

        let cookies = HTTPCookieStorage.shared.cookies(for: instanceURL) ?? []
        if let sessionCookie = cookies.first(where: { $0.name == "sessionid" })?.value,
           !sessionCookie.isEmpty {
            DispatchQueue.main.async {
                self.onComplete?(sessionCookie)
                self.onComplete = nil
            }
        } else {
            // Cookie might not be immediately available; retry once after a short delay.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
                guard let self else { return }
                let retryURL = URL(string: "https://\(meetInstance)/")!
                let retryCookies = HTTPCookieStorage.shared.cookies(for: retryURL) ?? []
                let cookie = retryCookies.first(where: { $0.name == "sessionid" })?.value
                self.onComplete?(cookie?.isEmpty == false ? cookie : nil)
                self.onComplete = nil
            }
        }
    }

    // MARK: - ASWebAuthenticationPresentationContextProviding

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        // Return the key window for presentation
        guard let scene = UIApplication.shared.connectedScenes.first as? UIWindowScene,
              let window = scene.windows.first(where: { $0.isKeyWindow }) ?? scene.windows.first else {
            return ASPresentationAnchor()
        }
        return window
    }

    // MARK: - Keychain Storage

    func saveCookie(_ cookie: String) {
        let data = cookie.data(using: .utf8)!
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrAccount as String: "visio_sessionid",
            kSecAttrService as String: "io.visio.mobile",
        ]
        SecItemDelete(query as CFDictionary)
        var addQuery = query
        addQuery[kSecValueData as String] = data
        SecItemAdd(addQuery as CFDictionary, nil)
    }

    func getSavedCookie() -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrAccount as String: "visio_sessionid",
            kSecAttrService as String: "io.visio.mobile",
            kSecReturnData as String: true,
        ]
        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        guard status == errSecSuccess, let data = result as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    func clearCookie() {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrAccount as String: "visio_sessionid",
            kSecAttrService as String: "io.visio.mobile",
        ]
        SecItemDelete(query as CFDictionary)
    }
}
