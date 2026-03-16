import SwiftUI
import visioFFI

struct SettingsView: View {
    @EnvironmentObject private var manager: VisioManager
    @Environment(\.dismiss) private var dismiss

    @State private var displayName: String = ""
    @State private var micOnJoin: Bool = true
    @State private var cameraOnJoin: Bool = false
    @State private var adaptiveModeEnabled: Bool = true
    @State private var language: String = Strings.detectSystemLang()
    @State private var theme: String = "light"
    @State private var meetInstances: [String] = ["meet.numerique.gouv.fr"]
    @State private var newInstance: String = ""

    private var lang: String { manager.currentLang }
    private var isDark: Bool { theme == "dark" }

    /// Normalizes a meet instance by stripping protocol prefixes and trailing slashes.
    /// Converts "https://meet.example.com/" to "meet.example.com".
    private func normalizeInstance(_ input: String) -> String {
        var result = input
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        if result.hasPrefix("https://") {
            result = String(result.dropFirst(8))
        } else if result.hasPrefix("http://") {
            result = String(result.dropFirst(7))
        }
        // Remove trailing slashes and any path
        if let slashIndex = result.firstIndex(of: "/") {
            result = String(result[..<slashIndex])
        }
        return result
    }

    var body: some View {
        NavigationStack {
            Form {
                Section(Strings.t("settings.profile", lang: lang)) {
                    TextField(Strings.t("settings.displayName", lang: lang), text: $displayName)
                        .autocorrectionDisabled()
                }

                Section(Strings.t("settings.joinMeeting", lang: lang)) {
                    Toggle(Strings.t("settings.micOnJoin", lang: lang), isOn: $micOnJoin)
                    Toggle(Strings.t("settings.camOnJoin", lang: lang), isOn: $cameraOnJoin)
                    Toggle(Strings.t("settings.adaptiveMode", lang: lang), isOn: $adaptiveModeEnabled)
                }

                Section(Strings.t("settings.theme", lang: lang)) {
                    ForEach(["light", "dark"], id: \.self) { option in
                        ThemeOptionRow(
                            label: Strings.t("settings.theme.\(option)", lang: lang),
                            isSelected: theme == option,
                            isDark: isDark,
                            onTap: {
                                theme = option
                                manager.setTheme(option)
                            }
                        )
                    }
                }

                Section(Strings.t("settings.language", lang: lang)) {
                    Picker(Strings.t("settings.language", lang: lang), selection: $language) {
                        ForEach(Strings.supportedLangs, id: \.self) { code in
                            Text(Strings.t("lang.\(code)", lang: code)).tag(code)
                        }
                    }
                    .pickerStyle(.menu)
                    .onChange(of: language) { newLang in
                        manager.setLanguage(newLang)
                    }
                }

                Section(Strings.t("settings.meetInstances", lang: lang)) {
                    ForEach(meetInstances, id: \.self) { instance in
                        HStack {
                            Text(instance)
                            Spacer()
                            Button {
                                meetInstances.removeAll { $0 == instance }
                            } label: {
                                Image(systemName: "minus.circle.fill")
                                    .foregroundStyle(.red)
                            }
                        }
                    }
                    HStack {
                        TextField(Strings.t("settings.instancePlaceholder", lang: lang), text: $newInstance)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .keyboardType(.URL)
                        Button {
                            let normalized = normalizeInstance(newInstance)
                            if !normalized.isEmpty && !meetInstances.contains(normalized) {
                                meetInstances.append(normalized)
                                newInstance = ""
                            }
                        } label: {
                            Image(systemName: "plus.circle.fill")
                                .foregroundStyle(VisioColors.primary500)
                        }
                        .disabled(newInstance.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
            .scrollContentBackground(.hidden)
            .background(VisioColors.background(dark: isDark))
            .navigationTitle(Strings.t("settings", lang: lang))
            .navigationBarTitleDisplayMode(.inline)
            .toolbarColorScheme(isDark ? .dark : .light, for: .navigationBar)
            .toolbarBackground(VisioColors.surface(dark: isDark), for: .navigationBar)
            .toolbarBackground(.visible, for: .navigationBar)
            .appToolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(Strings.t("settings.save", lang: lang)) {
                        save()
                        dismiss()
                    }
                    .foregroundStyle(VisioColors.primary500)
                }
                ToolbarItem(placement: .cancellationAction) {
                    Button(Strings.t("settings.cancel", lang: lang)) {
                        dismiss()
                    }
                    .foregroundStyle(VisioColors.secondaryText(dark: isDark))
                }
            }
        }
        .preferredColorScheme(isDark ? .dark : .light)
        .onAppear { load() }
    }

    private func load() {
        let settings = manager.getSettings()
        displayName = settings.displayName ?? ""
        micOnJoin = settings.micEnabledOnJoin
        cameraOnJoin = settings.cameraEnabledOnJoin
        language = settings.language ?? Strings.detectSystemLang()
        theme = settings.theme ?? "light"
        adaptiveModeEnabled = manager.client.isAdaptiveModeEnabled()
        meetInstances = manager.client.getMeetInstances()
    }

    private func save() {
        let name = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        manager.setDisplayName(name.isEmpty ? nil : name)
        manager.updateDisplayName(name)
        manager.setMicEnabledOnJoin(micOnJoin)
        manager.setCameraEnabledOnJoin(cameraOnJoin)
        manager.setLanguage(language)
        let wasEnabled = manager.client.isAdaptiveModeEnabled()
        manager.client.setAdaptiveModeEnabled(enabled: adaptiveModeEnabled)
        if wasEnabled && !adaptiveModeEnabled {
            manager.stopContextDetection()
        } else if !wasEnabled && adaptiveModeEnabled {
            manager.startContextDetection()
        }
        manager.client.setMeetInstances(instances: meetInstances)
    }
}

private struct PressedKey: PreferenceKey {
    static var defaultValue = false
    static func reduce(value: inout Bool, nextValue: () -> Bool) {
        value = value || nextValue()
    }
}

private struct ThemeRowStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .preference(key: PressedKey.self, value: configuration.isPressed)
    }
}

private struct ThemeOptionRow: View {
    let label: String
    let isSelected: Bool
    let isDark: Bool
    let onTap: () -> Void

    @State private var pressed = false

    var body: some View {
        Button(action: onTap) {
            HStack {
                Text(label)
                    .foregroundStyle(VisioColors.onSurface(dark: isDark))
                Spacer()
                if isSelected {
                    Image(systemName: "checkmark")
                        .foregroundStyle(VisioColors.primary500)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(ThemeRowStyle())
        .onPreferenceChange(PressedKey.self) { pressed = $0 }
        .listRowBackground(
            pressed
                ? VisioColors.surfaceVariant(dark: isDark)
                : VisioColors.surface(dark: isDark)
        )
    }
}

#Preview {
    SettingsView()
        .environmentObject(VisioManager())
}
