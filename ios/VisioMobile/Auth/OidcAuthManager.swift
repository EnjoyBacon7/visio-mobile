import AuthenticationServices
import Security
import UIKit

class OidcAuthManager: NSObject, ASWebAuthenticationPresentationContextProviding {

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        guard let scene = UIApplication.shared.connectedScenes.first as? UIWindowScene,
              let window = scene.windows.first else {
            return ASPresentationAnchor()
        }
        return window
    }

    func launchOidcFlow(meetInstance: String, completion: @escaping (String?) -> Void) {
        guard let authURL = URL(string: "https://\(meetInstance)/authenticate/?returnTo=https://\(meetInstance)/") else {
            completion(nil)
            return
        }

        let session = ASWebAuthenticationSession(url: authURL, callbackURLScheme: "visio") { callbackURL, error in
            guard error == nil else {
                completion(nil)
                return
            }
            // After OIDC flow, try to get the session cookie from shared cookie store
            let cookies = HTTPCookieStorage.shared.cookies(for: URL(string: "https://\(meetInstance)")!) ?? []
            let sessionCookie = cookies.first(where: { $0.name == "sessionid" })?.value
            completion(sessionCookie)
        }

        session.presentationContextProvider = self
        session.prefersEphemeralWebBrowserSession = false
        session.start()
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
