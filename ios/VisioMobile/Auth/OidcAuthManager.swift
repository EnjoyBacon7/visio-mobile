import Security
import SwiftUI
import UIKit
import WebKit

// MARK: - OidcAuthManager

class OidcAuthManager: NSObject, ObservableObject {

    /// Set to trigger the OIDC sheet presentation from SwiftUI.
    @Published var pendingInstance: String? = nil

    /// Called when the OIDC flow completes (cookie or nil).
    var onComplete: ((String?) -> Void)?

    func launchOidcFlow(meetInstance: String, completion: @escaping (String?) -> Void) {
        onComplete = completion
        DispatchQueue.main.async {
            self.pendingInstance = meetInstance
        }
    }

    func handleResult(_ cookie: String?) {
        pendingInstance = nil
        onComplete?(cookie)
        onComplete = nil
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

// MARK: - SwiftUI WKWebView Wrapper

struct OidcWebView: UIViewRepresentable {
    let meetInstance: String
    let onCookieExtracted: (String) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(meetInstance: meetInstance, onCookieExtracted: onCookieExtracted)
    }

    func makeUIView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        let webView = WKWebView(frame: .zero, configuration: config)
        webView.navigationDelegate = context.coordinator

        let returnTo = "https://\(meetInstance)/"
        let encodedReturnTo = returnTo.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) ?? returnTo
        if let url = URL(string: "https://\(meetInstance)/api/v1.0/authenticate/?returnTo=\(encodedReturnTo)") {
            webView.load(URLRequest(url: url))
        }
        return webView
    }

    func updateUIView(_ uiView: WKWebView, context: Context) {}

    class Coordinator: NSObject, WKNavigationDelegate {
        let meetInstance: String
        let onCookieExtracted: (String) -> Void
        private var extracted = false

        init(meetInstance: String, onCookieExtracted: @escaping (String) -> Void) {
            self.meetInstance = meetInstance
            self.onCookieExtracted = onCookieExtracted
        }

        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            guard let url = webView.url, !extracted else { return }
            let returnTo = "https://\(meetInstance)/"
            if url.absoluteString.hasPrefix(returnTo)
                && !url.absoluteString.contains("/api/v1.0/authenticate") {
                let store = webView.configuration.websiteDataStore.httpCookieStore
                store.getAllCookies { [weak self] cookies in
                    guard let self, !self.extracted else { return }
                    if let cookie = cookies.first(where: {
                        $0.name == "sessionid" && $0.domain.contains(self.meetInstance)
                    })?.value, !cookie.isEmpty {
                        self.extracted = true
                        DispatchQueue.main.async {
                            self.onCookieExtracted(cookie)
                        }
                    }
                }
            }
        }
    }
}

// MARK: - Sheet View

struct OidcLoginSheet: View {
    let meetInstance: String
    let onResult: (String?) -> Void

    var body: some View {
        NavigationStack {
            OidcWebView(meetInstance: meetInstance) { cookie in
                onResult(cookie)
            }
            .navigationTitle(meetInstance)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") {
                        onResult(nil)
                    }
                }
            }
        }
    }
}
