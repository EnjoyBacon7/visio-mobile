import SwiftUI

struct HomeView: View {
    @EnvironmentObject private var manager: VisioManager

    @State private var roomURL: String = ""
    @State private var displayName: String = ""
    @State private var navigateToCall: Bool = false
    @State private var showSettings: Bool = false
    @State private var roomStatus: String = "idle"
    @State private var meetInstances: [String] = []

    private var lang: String { manager.currentLang }
    private var isDark: Bool { manager.currentTheme == "dark" }

    private static let slugPattern = /^[a-z]{3}-[a-z]{4}-[a-z]{3}$/

    private func extractSlug(_ input: String) -> String? {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
            .trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        let candidate = trimmed.contains("/")
            ? String(trimmed.split(separator: "/").last ?? "")
            : trimmed
        return candidate.wholeMatch(of: Self.slugPattern) != nil ? candidate : nil
    }

    /// If input is just a slug, prefix with first configured server
    private func resolveRoomURL(_ input: String) -> String {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.wholeMatch(of: Self.slugPattern) != nil, let server = meetInstances.first {
            return "https://\(server)/\(trimmed)"
        }
        return trimmed
    }

    var body: some View {
        ZStack {
            VisioColors.background(dark: isDark).ignoresSafeArea()

            VStack(spacing: 32) {
                Spacer()

                // App branding with tricolore logo
                VStack(spacing: 8) {
                    VisioLogo(size: 96)
                    Text(Strings.t("app.title", lang: lang))
                        .font(.largeTitle)
                        .fontWeight(.bold)
                        .foregroundStyle(VisioColors.onBackground(dark: isDark))
                }

                Text(Strings.t("home.subtitle", lang: lang))
                    .font(.subheadline)
                    .foregroundStyle(VisioColors.secondaryText(dark: isDark))

                // Authentication section
                if manager.isAuthenticated {
                    VStack(spacing: 4) {
                        Text("\(Strings.t("home.loggedAs", lang: lang)) \(manager.authenticatedDisplayName)")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                        Button(Strings.t("home.logout", lang: lang)) {
                            manager.logoutSession()
                        }
                        .font(.subheadline)
                    }
                } else {
                    Button(action: {
                        guard let meetInstance = meetInstances.first else { return }
                        manager.authManager.launchOidcFlow(meetInstance: meetInstance) { cookie in
                            if let cookie = cookie {
                                manager.onAuthCookieReceived(cookie)
                            }
                        }
                    }) {
                        Label(Strings.t("home.connect", lang: lang), systemImage: "person.circle")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                    .padding(.horizontal, 32)
                }

                // Input fields
                VStack(spacing: 16) {
                    TextField(Strings.t("home.meetUrl.placeholder", lang: lang), text: $roomURL)
                        .textFieldStyle(.roundedBorder)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.URL)

                    if roomStatus == "checking" {
                        Text(Strings.t("home.room.checking", lang: lang))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    } else if roomStatus == "valid" {
                        Text(Strings.t("home.room.valid", lang: lang))
                            .font(.caption)
                            .foregroundStyle(.green)
                    } else if roomStatus == "not_found" {
                        Text(Strings.t("home.room.notFound", lang: lang))
                            .font(.caption)
                            .foregroundStyle(.red)
                    }

                    TextField(Strings.t("home.displayName", lang: lang), text: $displayName)
                        .textFieldStyle(.roundedBorder)
                        .textInputAutocapitalization(.words)
                }
                .padding(.horizontal, 32)
                .task(id: roomURL) {
                    let resolved = resolveRoomURL(roomURL)
                    guard let _ = extractSlug(resolved) else {
                        roomStatus = "idle"
                        return
                    }
                    roomStatus = "checking"
                    try? await Task.sleep(for: .milliseconds(500))
                    guard !Task.isCancelled else { return }
                    let result = manager.client.validateRoom(
                        url: resolved,
                        username: displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            ? nil : displayName.trimmingCharacters(in: .whitespacesAndNewlines)
                    )
                    guard !Task.isCancelled else { return }
                    switch result {
                    case .valid: roomStatus = "valid"
                    case .notFound: roomStatus = "not_found"
                    case .invalidFormat: roomStatus = "idle"
                    case .networkError: roomStatus = "error"
                    }
                }

                // Join button
                Button {
                    navigateToCall = true
                } label: {
                    Label(Strings.t("home.join", lang: lang), systemImage: "phone.fill")
                        .font(.headline)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 12)
                }
                .buttonStyle(.borderedProminent)
                .tint(VisioColors.primary500)
                .disabled(roomStatus != "valid")
                .padding(.horizontal, 32)

                Spacer()
                Spacer()
            }
        }
        .navigationTitle(Strings.t("app.title", lang: lang))
        .navigationBarTitleDisplayMode(.inline)
        .toolbarColorScheme(isDark ? .dark : .light, for: .navigationBar)
        .toolbarBackground(VisioColors.surface(dark: isDark), for: .navigationBar)
        .toolbarBackground(.visible, for: .navigationBar)
        .appToolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    showSettings = true
                } label: {
                    Image(systemName: "gearshape.fill")
                        .foregroundStyle(VisioColors.secondaryText(dark: isDark))
                }
            }
        }
        .navigationDestination(isPresented: $navigateToCall) {
            CallView(
                roomURL: resolveRoomURL(roomURL),
                displayName: displayName.trimmingCharacters(in: .whitespacesAndNewlines)
            )
        }
        .sheet(isPresented: $showSettings) {
            SettingsView()
                .environmentObject(manager)
        }
        .onAppear {
            // Pre-fill display name from manager (includes OIDC identity)
            let name = manager.displayName
            if !name.isEmpty && displayName.isEmpty {
                displayName = name
            }
            // Load meet instances
            meetInstances = manager.client.getMeetInstances()
        }
        .onChange(of: manager.authenticatedDisplayName) { newValue in
            if !newValue.isEmpty && displayName.isEmpty {
                displayName = newValue
            }
        }
        .onChange(of: manager.pendingDeepLink) { newValue in
            if let link = newValue {
                roomURL = link
                manager.pendingDeepLink = nil
            }
        }
    }
}

#Preview {
    NavigationStack {
        HomeView()
            .environmentObject(VisioManager())
    }
}
