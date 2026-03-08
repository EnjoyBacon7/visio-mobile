import SwiftUI
import visioFFI

struct HomeView: View {
    @EnvironmentObject private var manager: VisioManager

    @State private var roomURL: String = ""
    @State private var displayName: String = ""
    @State private var navigateToCall: Bool = false
    @State private var showSettings: Bool = false
    @State private var roomStatus: String = "idle"
    @State private var meetInstances: [String] = []
    @State private var showServerPicker: Bool = false
    @State private var customServer: String = ""
    @State private var showCreateRoom: Bool = false
    @State private var pendingOidcInstance: String? = nil

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
                    AuthenticatedCard(
                        displayName: manager.authenticatedDisplayName,
                        email: manager.authenticatedEmail,
                        isDark: isDark,
                        lang: lang,
                        onLogout: { manager.logoutSession() }
                    )
                    .padding(.horizontal, 32)
                } else {
                    Button(action: {
                        if meetInstances.count <= 1 {
                            guard let meetInstance = meetInstances.first else { return }
                            launchOidc(meetInstance: meetInstance)
                        } else {
                            customServer = ""
                            showServerPicker = true
                        }
                    }) {
                        Label(Strings.t("home.connect", lang: lang), systemImage: "person.circle")
                            .font(.headline)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 12)
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(VisioColors.primary500)
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

                if manager.isAuthenticated {
                    Button {
                        showCreateRoom = true
                    } label: {
                        Label(Strings.t("home.createRoom", lang: lang), systemImage: "plus.rectangle")
                            .font(.headline)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 12)
                    }
                    .buttonStyle(.bordered)
                    .tint(VisioColors.primary500)
                    .padding(.horizontal, 32)
                }

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
        .sheet(isPresented: $showCreateRoom) {
            CreateRoomSheet(
                lang: lang,
                onCreated: { roomUrl in
                    showCreateRoom = false
                    roomURL = roomUrl
                    navigateToCall = true
                },
                onCancel: { showCreateRoom = false }
            )
            .environmentObject(manager)
        }
        .sheet(isPresented: $showServerPicker) {
            ServerPickerWithOidc(
                instances: meetInstances,
                customServer: $customServer,
                lang: lang,
                onComplete: { cookie, instance in
                    showServerPicker = false
                    if let cookie {
                        manager.onAuthCookieReceived(cookie, meetInstance: instance)
                    }
                },
                onDismiss: { showServerPicker = false }
            )
        }
        .sheet(isPresented: Binding(
            get: { pendingOidcInstance != nil },
            set: { if !$0 { pendingOidcInstance = nil } }
        )) {
            if let instance = pendingOidcInstance {
                OidcLoginSheet(meetInstance: instance) { cookie in
                    let inst = instance
                    pendingOidcInstance = nil
                    if let cookie {
                        manager.onAuthCookieReceived(cookie, meetInstance: inst)
                    }
                }
            }
        }
    }

    private func launchOidc(meetInstance: String) {
        pendingOidcInstance = meetInstance
    }
}

// MARK: - Server Picker

/// Server picker that navigates to the OIDC web view within the same sheet.
private struct ServerPickerWithOidc: View {
    let instances: [String]
    @Binding var customServer: String
    let lang: String
    let onComplete: (String?, String) -> Void  // (cookie?, meetInstance)
    let onDismiss: () -> Void

    @State private var selectedInstance: String? = nil

    var body: some View {
        NavigationStack {
            if let instance = selectedInstance {
                // OIDC web view — pushed in same sheet
                OidcWebView(meetInstance: instance) { cookie in
                    onComplete(cookie, instance)
                }
                .navigationTitle(instance)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button(Strings.t("home.serverPicker.cancel", lang: lang)) {
                            onDismiss()
                        }
                    }
                }
            } else {
                // Server picker list
                List {
                    Section {
                        ForEach(instances, id: \.self) { instance in
                            Button(instance) {
                                selectedInstance = instance
                            }
                        }
                    }
                    Section {
                        TextField(Strings.t("home.serverPicker.custom", lang: lang), text: $customServer)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .keyboardType(.URL)
                        Button(Strings.t("home.connect", lang: lang)) {
                            let trimmed = customServer.trimmingCharacters(in: .whitespacesAndNewlines)
                            if !trimmed.isEmpty {
                                selectedInstance = trimmed
                            }
                        }
                        .disabled(customServer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
                .navigationTitle(Strings.t("home.serverPicker.title", lang: lang))
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button(Strings.t("home.serverPicker.cancel", lang: lang)) {
                            onDismiss()
                        }
                    }
                }
            }
        }
    }
}

// MARK: - Authenticated Card

private struct AuthenticatedCard: View {
    let displayName: String
    let email: String
    let isDark: Bool
    let lang: String
    let onLogout: () -> Void

    private var initials: String {
        let parts = displayName.split(separator: " ").prefix(2)
        let result = parts.compactMap { $0.first?.uppercased() }.joined()
        if !result.isEmpty { return result }
        return email.first?.uppercased() ?? "?"
    }

    var body: some View {
        HStack(spacing: 12) {
            // Avatar circle
            ZStack {
                Circle()
                    .fill(VisioColors.primary500)
                    .frame(width: 44, height: 44)
                Text(initials)
                    .font(.system(size: 16, weight: .bold))
                    .foregroundStyle(.white)
            }

            // Name and email
            VStack(alignment: .leading, spacing: 2) {
                Text(displayName.isEmpty ? email : displayName)
                    .font(.body)
                    .fontWeight(.semibold)
                    .foregroundStyle(VisioColors.onBackground(dark: isDark))
                    .lineLimit(1)
                if !email.isEmpty && !displayName.isEmpty {
                    Text(email)
                        .font(.caption)
                        .foregroundStyle(VisioColors.secondaryText(dark: isDark))
                        .lineLimit(1)
                }
            }

            Spacer()

            // Logout button
            Button(action: onLogout) {
                Image(systemName: "rectangle.portrait.and.arrow.right")
                    .foregroundStyle(VisioColors.secondaryText(dark: isDark))
            }
        }
        .padding(16)
        .background(
            RoundedRectangle(cornerRadius: 16)
                .fill(isDark
                    ? Color(red: 0.12, green: 0.12, blue: 0.18)
                    : Color(red: 0.95, green: 0.95, blue: 0.97))
        )
    }
}

// MARK: - Create Room Sheet

private struct CreateRoomSheet: View {
    @EnvironmentObject private var manager: VisioManager
    let lang: String
    let onCreated: (String) -> Void
    let onCancel: () -> Void

    @State private var accessLevel: String = "public"
    @State private var creating: Bool = false
    @State private var error: String? = nil
    @State private var createdUrl: String? = nil
    @State private var copiedHttp: Bool = false
    @State private var copiedDeep: Bool = false
    @State private var searchQuery: String = ""
    @State private var searchResults: [UserSearchResult] = []
    @State private var invitedUsers: [UserSearchResult] = []
    @State private var createdRoomId: String? = nil
    @State private var searchTask: Task<Void, Never>? = nil

    private var deepLink: String {
        guard let url = createdUrl else { return "" }
        let stripped = url.replacingOccurrences(of: "https://", with: "")
        return "visio://\(stripped)"
    }

    var body: some View {
        NavigationStack {
            Form {
                if createdUrl == nil {
                    Section {
                        Picker(Strings.t("home.createRoom.access", lang: lang), selection: $accessLevel) {
                            Text(Strings.t("home.createRoom.public", lang: lang)).tag("public")
                            Text(Strings.t("home.createRoom.trusted", lang: lang)).tag("trusted")
                            Text(Strings.t("home.createRoom.restricted", lang: lang)).tag("restricted")
                        }
                        .pickerStyle(.inline)
                        .labelsHidden()

                        if accessLevel == "public" {
                            Text(Strings.t("home.createRoom.publicDesc", lang: lang))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        } else if accessLevel == "trusted" {
                            Text(Strings.t("home.createRoom.trustedDesc", lang: lang))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        } else {
                            Text(Strings.t("home.createRoom.restrictedDesc", lang: lang))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    } header: {
                        Text(Strings.t("home.createRoom.access", lang: lang))
                    }

                    if accessLevel == "restricted" {
                        Section(header: Text(Strings.t("restricted.invite", lang: lang))) {
                            TextField(Strings.t("restricted.searchUsers", lang: lang), text: $searchQuery)
                                .onChange(of: searchQuery) { newValue in
                                    searchTask?.cancel()
                                    guard newValue.count >= 3 else {
                                        searchResults = []
                                        return
                                    }
                                    searchTask = Task {
                                        try? await Task.sleep(nanoseconds: 300_000_000)
                                        guard !Task.isCancelled else { return }
                                        let query = newValue
                                        DispatchQueue.global(qos: .userInitiated).async {
                                            do {
                                                let results = try manager.client.searchUsers(query: query)
                                                DispatchQueue.main.async {
                                                    searchResults = results.filter { user in
                                                        !invitedUsers.contains(where: { $0.id == user.id })
                                                    }
                                                }
                                            } catch {
                                                DispatchQueue.main.async { searchResults = [] }
                                            }
                                        }
                                    }
                                }

                            ForEach(searchResults, id: \.id) { user in
                                Button {
                                    invitedUsers.append(user)
                                    searchQuery = ""
                                    searchResults = []
                                } label: {
                                    VStack(alignment: .leading) {
                                        Text(user.fullName ?? user.email)
                                        Text(user.email)
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    }
                                }
                            }
                        }

                        if !invitedUsers.isEmpty {
                            Section(header: Text(Strings.t("restricted.members", lang: lang))) {
                                ForEach(invitedUsers, id: \.id) { user in
                                    HStack {
                                        Text(user.fullName ?? user.email)
                                        Spacer()
                                        Button {
                                            invitedUsers.removeAll { $0.id == user.id }
                                        } label: {
                                            Image(systemName: "xmark.circle.fill")
                                                .foregroundStyle(.secondary)
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if let error {
                        Section {
                            Text(error)
                                .foregroundStyle(.red)
                                .font(.caption)
                        }
                    }

                    Section {
                        Button {
                            let meetInstance = manager.authenticatedMeetInstance
                            guard !meetInstance.isEmpty else { return }
                            creating = true
                            error = nil
                            DispatchQueue.global(qos: .userInitiated).async {
                                do {
                                    let result = try manager.client.createRoom(
                                        meetUrl: "https://\(meetInstance)",
                                        name: "",
                                        accessLevel: accessLevel
                                    )
                                    // Add accesses for invited users
                                    if accessLevel == "restricted" {
                                        for user in invitedUsers {
                                            _ = try? manager.client.addAccess(userId: user.id, roomId: result.id)
                                        }
                                    }
                                    DispatchQueue.main.async {
                                        createdRoomId = result.id
                                        createdUrl = "https://\(meetInstance)/\(result.slug)"
                                        creating = false
                                    }
                                } catch {
                                    DispatchQueue.main.async {
                                        self.error = error.localizedDescription
                                        creating = false
                                    }
                                }
                            }
                        } label: {
                            HStack {
                                Spacer()
                                Text(creating
                                    ? Strings.t("home.createRoom.creating", lang: lang)
                                    : Strings.t("home.createRoom.create", lang: lang))
                                    .fontWeight(.semibold)
                                Spacer()
                            }
                        }
                        .disabled(creating)
                    }
                } else {
                    Section {
                        VStack(alignment: .leading, spacing: 4) {
                            HStack {
                                Image(systemName: "globe")
                                    .font(.caption)
                                Text(Strings.t("settings.incall.roomLink", lang: lang))
                                    .font(.subheadline)
                                    .fontWeight(.semibold)
                                Spacer()
                                Button {
                                    UIPasteboard.general.string = createdUrl
                                    copiedHttp = true
                                    DispatchQueue.main.asyncAfter(deadline: .now() + 2) { copiedHttp = false }
                                } label: {
                                    Image(systemName: copiedHttp ? "checkmark" : "doc.on.doc")
                                        .font(.caption)
                                }
                                ShareLink(item: createdUrl!) {
                                    Image(systemName: "square.and.arrow.up")
                                        .font(.caption)
                                }
                            }
                            TextField("", text: .constant(createdUrl!))
                                .font(.caption)
                                .textFieldStyle(.roundedBorder)
                                .disabled(true)
                        }
                        VStack(alignment: .leading, spacing: 4) {
                            HStack {
                                Image(systemName: "iphone")
                                    .font(.caption)
                                Text(Strings.t("settings.incall.deepLink", lang: lang))
                                    .font(.subheadline)
                                    .fontWeight(.semibold)
                                Spacer()
                                Button {
                                    UIPasteboard.general.string = deepLink
                                    copiedDeep = true
                                    DispatchQueue.main.asyncAfter(deadline: .now() + 2) { copiedDeep = false }
                                } label: {
                                    Image(systemName: copiedDeep ? "checkmark" : "doc.on.doc")
                                        .font(.caption)
                                }
                                ShareLink(item: deepLink) {
                                    Image(systemName: "square.and.arrow.up")
                                        .font(.caption)
                                }
                            }
                            TextField("", text: .constant(deepLink))
                                .font(.caption)
                                .textFieldStyle(.roundedBorder)
                                .disabled(true)
                        }
                    } header: {
                        Text(Strings.t("settings.incall.roomInfo", lang: lang))
                    }

                    Section {
                        Button {
                            onCreated(createdUrl!)
                        } label: {
                            HStack {
                                Spacer()
                                Label(Strings.t("home.join", lang: lang), systemImage: "phone.fill")
                                    .fontWeight(.semibold)
                                Spacer()
                            }
                        }
                    }
                }
            }
            .navigationTitle(Strings.t("home.createRoom", lang: lang))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(Strings.t("settings.cancel", lang: lang)) { onCancel() }
                }
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
