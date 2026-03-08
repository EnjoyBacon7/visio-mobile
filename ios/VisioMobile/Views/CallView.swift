import SwiftUI
import AVFoundation
import visioFFI

struct CallView: View {
    @EnvironmentObject private var manager: VisioManager
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase

    let roomURL: String
    let displayName: String

    @State private var showChat: Bool = false
    @State private var showAudioDevices: Bool = false
    @State private var showInCallSettings: Bool = false
    @State private var inCallSettingsTab: Int = 0
    @State private var showParticipantList: Bool = false
    @State private var focusedParticipant: String? = nil
    // lobbyNotificationDismissTask removed — banner is now persistent while participants wait
    @State private var showOverflow: Bool = false
    @State private var showReactionPicker: Bool = false
    @State private var adaptiveModeOverride: AdaptiveMode? = nil

    private var lang: String { manager.currentLang }
    private var isDark: Bool { manager.currentTheme == "dark" }

    var body: some View {
        ZStack {
            (manager.adaptiveMode == .office
                ? VisioColors.background(dark: isDark)
                : Color.black
            ).ignoresSafeArea()

            VStack(spacing: 0) {
                // Lobby join notification banner
                lobbyNotificationBanner

                // Connection state banner
                connectionBanner

                // Error banner
                if let error = manager.errorMessage {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.white)
                        .padding(8)
                        .frame(maxWidth: .infinity)
                        .background(VisioColors.error500)
                }

                // Main content area: video grid, waiting for host, or waiting for participants
                ZStack {
                    if case .waitingForHost = manager.connectionState {
                        VStack(spacing: 24) {
                            Spacer()
                            ProgressView()
                                .scaleEffect(1.5)
                                .tint(isDark ? .white : VisioColors.primary500)
                            Text(Strings.t("lobby.waiting", lang: lang))
                                .font(.title2)
                                .foregroundStyle(VisioColors.onBackground(dark: isDark))
                            Text(Strings.t("lobby.waitingDesc", lang: lang))
                                .foregroundStyle(VisioColors.secondaryText(dark: isDark))
                            Button(action: {
                                manager.cancelLobby()
                                manager.disconnect()
                                CallKitManager.shared.reportCallEnded()
                                dismiss()
                            }) {
                                Text(Strings.t("lobby.cancel", lang: lang))
                            }
                            .buttonStyle(.bordered)
                            Spacer()
                        }
                        .padding()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                    } else if manager.participants.isEmpty {
                        VStack(spacing: 12) {
                            Spacer()
                            ProgressView()
                                .tint(isDark ? .white : VisioColors.primary500)
                            Text(Strings.t("call.waiting", lang: lang))
                                .foregroundStyle(VisioColors.secondaryText(dark: isDark))
                            Spacer()
                        }
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                    } else if manager.adaptiveMode == .car {
                        // Car mode: audio-only screen with active speaker name
                        carAudioOnlyView
                    } else if manager.adaptiveMode == .pedestrian {
                        // Pedestrian mode: single active speaker tile
                        pedestrianSingleTile
                    } else if let focused = focusedParticipant,
                              let focusedP = manager.participants.first(where: { $0.sid == focused }) {
                        // Focus layout
                        focusLayout(focused: focusedP)
                    } else {
                        // Grid layout
                        gridLayout
                    }

                    // Reaction overlay
                    ReactionOverlay(reactions: manager.reactions)

                    // Persistent adaptive mode indicator
                    VStack {
                        HStack {
                            Spacer()
                            HStack(spacing: 4) {
                                Image(systemName: modeIcon(manager.adaptiveMode))
                                Text(modeLabel(manager.adaptiveMode))
                            }
                            .font(.caption2)
                            .foregroundColor(.white)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 4)
                            .background(Color.black.opacity(0.5))
                            .cornerRadius(12)
                            .padding(8)
                        }
                        Spacer()
                    }
                }

                // Control bar (hidden when waiting for host)
                if case .waitingForHost = manager.connectionState {
                    EmptyView()
                } else {
                    controlBar
                }
            }
        }
        .navigationTitle(Strings.t("call.title", lang: lang))
        .navigationBarTitleDisplayMode(.inline)
        .navigationBarBackButtonHidden(true)
        .toolbarColorScheme(isDark ? .dark : .light, for: .navigationBar)
        .toolbarBackground(VisioColors.surface(dark: isDark), for: .navigationBar)
        .toolbarBackground(.visible, for: .navigationBar)
        .fullScreenCover(isPresented: $showChat) {
            NavigationStack {
                ChatView()
                    .environmentObject(manager)
            }
            .onAppear { manager.setChatOpen(true) }
            .onDisappear { manager.setChatOpen(false) }
        }
        .sheet(isPresented: $showParticipantList) {
            ParticipantListSheet()
                .environmentObject(manager)
                .presentationDetents([.medium, .large])
        }
        .sheet(isPresented: $showAudioDevices) {
            AudioDeviceSheet()
                .environmentObject(manager)
                .presentationDetents([.medium])
        }
        .sheet(isPresented: $showInCallSettings) {
            InCallSettingsSheet(roomURL: roomURL, selectedTab: inCallSettingsTab)
                .environmentObject(manager)
                .presentationDetents([.medium, .large])
        }
        .onAppear {
            let name = displayName.isEmpty ? nil : displayName
            manager.connect(url: roomURL, username: name)
            manager.startAudioPlayout()
            CallKitManager.shared.reportCallStarted(roomName: roomURL)
            UIApplication.shared.isIdleTimerDisabled = true
            PiPManager.shared.setup()
        }
        .onDisappear {
            manager.stopAudioPlayout()
            UIApplication.shared.isIdleTimerDisabled = false
            PiPManager.shared.tearDown()
        }
        .onChange(of: scenePhase) { phase in
            if phase == .background {
                PiPManager.shared.startIfNeeded()
            } else if phase == .active {
                PiPManager.shared.stop()
            }
        }
        .onChange(of: manager.lobbyDenied) { denied in
            if denied {
                manager.lobbyDenied = false
                manager.disconnect()
                CallKitManager.shared.reportCallEnded()
                dismiss()
            }
        }
        // Lobby banner is now persistent — driven by waitingParticipants list
    }

    // MARK: - Adaptive Mode Helpers

    private func modeIcon(_ mode: AdaptiveMode) -> String {
        switch mode {
        case .office: return "wifi"
        case .pedestrian: return "figure.walk"
        case .car: return "car.fill"
        }
    }

    private func modeLabel(_ mode: AdaptiveMode) -> String {
        switch mode {
        case .office: return Strings.t("adaptive.office", lang: lang)
        case .pedestrian: return Strings.t("adaptive.pedestrian", lang: lang)
        case .car: return Strings.t("adaptive.car", lang: lang)
        }
    }

    // MARK: - Grid Layout

    private var gridLayout: some View {
        let count = manager.participants.count

        return GeometryReader { geo in
            let isLandscape = geo.size.width > geo.size.height
            let columnCount: Int = {
                if count == 1 { return 1 }
                if isLandscape { return min(count, 3) }
                return count <= 2 ? 1 : 2
            }()
            let rowCount = (count + columnCount - 1) / columnCount
            let tileHeight = (geo.size.height - 16 - CGFloat(rowCount - 1) * 8) / CGFloat(rowCount)

            VStack(spacing: 8) {
                ForEach(Array(stride(from: 0, to: count, by: columnCount)), id: \.self) { rowStart in
                    HStack(spacing: 8) {
                        ForEach(rowStart..<min(rowStart + columnCount, count), id: \.self) { idx in
                            let participant = manager.participants[idx]
                            ParticipantTile(
                                participant: participant,
                                isActiveSpeaker: manager.activeSpeakers.contains(participant.sid),
                                handRaisePosition: manager.handRaisedMap[participant.sid] ?? 0,
                                isDark: isDark
                            )
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                            .clipShape(RoundedRectangle(cornerRadius: 8))
                            .onTapGesture {
                                withAnimation(.easeInOut(duration: 0.2)) {
                                    focusedParticipant = participant.sid
                                }
                            }
                        }
                    }
                    .frame(height: tileHeight)
                }
            }
            .padding(8)
        }
    }

    // MARK: - Focus Layout

    private func focusLayout(focused: ParticipantInfo) -> some View {
        ParticipantTile(
            participant: focused,
            large: true,
            isActiveSpeaker: manager.activeSpeakers.contains(focused.sid),
            handRaisePosition: manager.handRaisedMap[focused.sid] ?? 0,
            isDark: isDark
        )
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .padding(8)
        .onTapGesture {
            withAnimation(.easeInOut(duration: 0.2)) {
                focusedParticipant = nil
            }
        }
    }

    // MARK: - Lobby Waiting Banner

    @ViewBuilder
    private var lobbyNotificationBanner: some View {
        if !manager.waitingParticipants.isEmpty {
            let first = manager.waitingParticipants[0]
            let message: String = {
                let base = Strings.t("lobby.joinRequest", lang: lang)
                    .replacingOccurrences(of: "{{name}}", with: first.username)
                if manager.waitingParticipants.count > 1 {
                    return base + " (+\(manager.waitingParticipants.count - 1))"
                }
                return base
            }()
            HStack(spacing: 12) {
                Text(message)
                    .font(.subheadline)
                    .fontWeight(.medium)
                    .foregroundStyle(.white)
                    .lineLimit(2)

                Spacer()

                Button {
                    manager.admitParticipant(first.id)
                } label: {
                    Text(Strings.t("lobby.admit", lang: lang))
                        .font(.caption)
                        .fontWeight(.semibold)
                        .foregroundStyle(.white)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(VisioColors.primary500)
                        .clipShape(Capsule())
                }

                Button {
                    showParticipantList = true
                } label: {
                    Text(Strings.t("lobby.view", lang: lang))
                        .font(.caption)
                        .fontWeight(.semibold)
                        .foregroundStyle(.white)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(VisioColors.primaryDark300)
                        .clipShape(Capsule())
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .background(VisioColors.primaryDark100)
            .clipShape(RoundedRectangle(cornerRadius: 12))
            .padding(.horizontal, 12)
            .padding(.top, 8)
            .transition(.move(edge: .top).combined(with: .opacity))
        }
    }

    // MARK: - Pedestrian Single Tile

    private var pedestrianSingleTile: some View {
        // Find if active speaker is a remote participant
        // (participants[0] is local, so skip it when looking for remote speaker)
        let remoteParticipants = Array(manager.participants.dropFirst())
        let remoteSpeaker = remoteParticipants.first(where: {
            manager.activeSpeakers.contains($0.sid)
        })
        let displayParticipant = remoteSpeaker ?? manager.participants.first
        return Group {
            if let speaker = displayParticipant {
                ParticipantTile(
                    participant: speaker,
                    large: true,
                    isActiveSpeaker: manager.activeSpeakers.contains(speaker.sid),
                    handRaisePosition: manager.handRaisedMap[speaker.sid] ?? 0,
                    isDark: true
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .clipShape(RoundedRectangle(cornerRadius: 8))
                .padding(8)
            }
        }
    }

    // MARK: - Car Audio-Only View

    private var carAudioOnlyView: some View {
        let activeSpeaker = manager.participants.first(where: {
            manager.activeSpeakers.contains($0.sid)
        }) ?? manager.participants.first
        let speakerName = activeSpeaker?.name ?? activeSpeaker?.identity ?? ""
        return VStack(spacing: 24) {
            Spacer()
            Image(systemName: "speaker.wave.3.fill")
                .font(.system(size: 64))
                .foregroundStyle(.white.opacity(0.6))
            Text(speakerName)
                .font(.system(size: 36, weight: .bold))
                .foregroundStyle(.white)
                .lineLimit(2)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 32)
            Text(Strings.t("adaptive.audioOnly", lang: lang))
                .font(.subheadline)
                .foregroundStyle(.white.opacity(0.5))
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - Connection Banner

    @ViewBuilder
    private var connectionBanner: some View {
        switch manager.connectionState {
        case .connecting:
            bannerView(text: "\(Strings.t("status.connecting", lang: lang))...", color: .orange)
        case .reconnecting(let attempt):
            bannerView(text: "\(Strings.t("status.reconnecting", lang: lang)) (\(attempt))...", color: .orange)
        case .disconnected:
            bannerView(text: Strings.t("status.disconnected", lang: lang), color: VisioColors.greyscale400)
        case .waitingForHost:
            EmptyView()
        case .connected:
            EmptyView()
        }
    }

    private func bannerView(text: String, color: Color) -> some View {
        Text(text)
            .font(.caption)
            .fontWeight(.medium)
            .foregroundStyle(.white)
            .padding(6)
            .frame(maxWidth: .infinity)
            .background(color)
    }

    // MARK: - Reaction Emojis

    private static let reactionEmojis: [(id: String, emoji: String)] = [
        ("thumbs-up", "\u{1F44D}"),
        ("thumbs-down", "\u{1F44E}"),
        ("clapping-hands", "\u{1F44F}"),
        ("red-heart", "\u{2764}\u{FE0F}"),
        ("face-with-tears-of-joy", "\u{1F602}"),
        ("face-with-open-mouth", "\u{1F62E}"),
        ("party-popper", "\u{1F389}"),
        ("folded-hands", "\u{1F64F}"),
    ]

    // MARK: - Control Bar

    private var isLargeButtons: Bool {
        manager.adaptiveMode == .pedestrian || manager.adaptiveMode == .car
    }

    private var buttonSize: CGFloat { isLargeButtons ? 96 : 38 }
    private var buttonIconSize: CGFloat { isLargeButtons ? 36 : 18 }
    private var buttonCornerRadius: CGFloat { isLargeButtons ? 16 : 8 }

    private var controlBar: some View {
        VStack(spacing: 4) {
            // Reaction picker row (above control bar) — office only
            if showReactionPicker && manager.adaptiveMode == .office {
                HStack(spacing: 0) {
                    ForEach(Self.reactionEmojis, id: \.id) { item in
                        Button {
                            manager.sendReaction(item.id)
                            showReactionPicker = false
                        } label: {
                            Text(item.emoji)
                                .font(.system(size: 28))
                                .padding(4)
                        }
                    }
                }
                .frame(maxWidth: .infinity)
                .padding(.horizontal, 8)
                .padding(.vertical, 8)
                .background(Color.black.opacity(0.8))
                .clipShape(RoundedRectangle(cornerRadius: 12))
                .padding(.horizontal, 16)
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }

            // Overflow menu row (above control bar) — office only
            if showOverflow && manager.adaptiveMode == .office {
                HStack(spacing: 0) {
                    Spacer()

                    // Hand raise
                    overflowItem(
                        icon: "hand.raised.fill",
                        label: Strings.t(manager.isHandRaised ? "control.lowerHand" : "control.raiseHand", lang: lang),
                        isActive: manager.isHandRaised,
                        activeColor: VisioColors.handRaise
                    ) {
                        manager.toggleHandRaise()
                        showOverflow = false
                    }

                    Spacer()

                    // Reaction
                    overflowItem(
                        icon: "face.smiling",
                        label: "Reaction",
                        isActive: false,
                        activeColor: .clear
                    ) {
                        showOverflow = false
                        withAnimation(.easeInOut(duration: 0.2)) {
                            showReactionPicker.toggle()
                        }
                    }

                    Spacer()

                    // Settings
                    overflowItem(
                        icon: "gearshape.fill",
                        label: Strings.t("settings.incall", lang: lang),
                        isActive: false,
                        activeColor: .clear
                    ) {
                        showOverflow = false
                        inCallSettingsTab = 0
                        showInCallSettings = true
                    }

                    Spacer()
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(Color.black.opacity(0.8))
                .clipShape(RoundedRectangle(cornerRadius: 12))
                .padding(.horizontal, 16)
                .transition(.move(edge: .bottom).combined(with: .opacity))

                // Adaptive mode override
                VStack(alignment: .leading, spacing: 6) {
                    Text(Strings.t("adaptive.override", lang: lang))
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(.white)
                    HStack(spacing: 8) {
                        let modeOptions: [(AdaptiveMode?, String)] = [
                            (nil, Strings.t("adaptive.auto", lang: lang)),
                            (.office, Strings.t("adaptive.office", lang: lang)),
                            (.pedestrian, Strings.t("adaptive.pedestrian", lang: lang)),
                            (.car, Strings.t("adaptive.car", lang: lang)),
                        ]
                        ForEach(Array(modeOptions.enumerated()), id: \.offset) { _, option in
                            let (mode, label) = option
                            let isSelected = mode == adaptiveModeOverride
                            Button {
                                adaptiveModeOverride = mode
                                manager.client.setAdaptiveModeOverride(mode: mode)
                            } label: {
                                Text(label)
                                    .font(.system(size: 11, weight: isSelected ? .bold : .regular))
                                    .foregroundStyle(isSelected ? .black : .white)
                                    .padding(.horizontal, 10)
                                    .padding(.vertical, 6)
                                    .background(isSelected ? VisioColors.primary500 : VisioColors.primaryDark100)
                                    .clipShape(Capsule())
                            }
                        }
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.black.opacity(0.8))
                .clipShape(RoundedRectangle(cornerRadius: 12))
                .padding(.horizontal, 16)
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }

            // Main control bar
            HStack(spacing: isLargeButtons ? 16 : 8) {
                // Mic toggle + audio route chevron (grouped) — all modes
                HStack(spacing: 1) {
                    Button {
                        manager.toggleMic()
                    } label: {
                        Image(systemName: manager.isMicEnabled ? "mic.fill" : "mic.slash.fill")
                            .font(.system(size: buttonIconSize, weight: .medium))
                            .foregroundStyle(.white)
                            .frame(width: buttonSize, height: buttonSize)
                            .background(manager.isMicEnabled ? VisioColors.primaryDark100 : VisioColors.error200)
                            .clipShape(UnevenRoundedRectangle(
                                topLeadingRadius: buttonCornerRadius,
                                bottomLeadingRadius: buttonCornerRadius,
                                bottomTrailingRadius: 2,
                                topTrailingRadius: 2
                            ))
                    }
                    .accessibilityLabel(Strings.t(manager.isMicEnabled ? "control.mute" : "control.unmute", lang: lang))

                    Button {
                        showAudioDevices = true
                    } label: {
                        Image(systemName: "chevron.up")
                            .font(.system(size: isLargeButtons ? 16 : 10, weight: .bold))
                            .foregroundStyle(.white)
                            .frame(width: isLargeButtons ? 40 : 22, height: buttonSize)
                            .background(VisioColors.primaryDark100)
                            .clipShape(UnevenRoundedRectangle(
                                topLeadingRadius: 2,
                                bottomLeadingRadius: 2,
                                bottomTrailingRadius: buttonCornerRadius,
                                topTrailingRadius: buttonCornerRadius
                            ))
                    }
                    .accessibilityLabel(Strings.t("control.audioDevices", lang: lang))
                }

                // Camera toggle — office and pedestrian only
                if manager.adaptiveMode != .car {
                    Button {
                        manager.toggleCamera()
                    } label: {
                        Image(systemName: manager.isCameraEnabled ? "video.fill" : "video.slash.fill")
                            .font(.system(size: buttonIconSize, weight: .medium))
                            .foregroundStyle(.white)
                            .frame(width: buttonSize, height: buttonSize)
                            .background(manager.isCameraEnabled ? VisioColors.primaryDark100 : VisioColors.error200)
                            .clipShape(RoundedRectangle(cornerRadius: buttonCornerRadius))
                    }
                    .accessibilityLabel(Strings.t(manager.isCameraEnabled ? "control.camOff" : "control.camOn", lang: lang))
                }

                // Participants with count badge — office only
                if manager.adaptiveMode == .office {
                    Button {
                        showParticipantList = true
                    } label: {
                        ZStack(alignment: .topTrailing) {
                            Image(systemName: "person.2.fill")
                                .font(.system(size: 16, weight: .medium))
                                .foregroundStyle(.white)
                                .frame(width: 38, height: 38)
                                .background(VisioColors.primaryDark100)
                                .clipShape(RoundedRectangle(cornerRadius: 8))

                            Text("\(manager.participants.count)")
                                .font(.system(size: 10, weight: .bold))
                                .foregroundStyle(.white)
                                .padding(.horizontal, 4)
                                .padding(.vertical, 1)
                                .background(VisioColors.primary500)
                                .clipShape(Capsule())
                                .offset(x: 4, y: -4)
                        }
                    }
                    .accessibilityLabel(Strings.t("participants.title", lang: lang))
                }

                // Chat with unread badge — office only
                if manager.adaptiveMode == .office {
                    Button {
                        showChat = true
                    } label: {
                        ZStack(alignment: .topTrailing) {
                            Image(systemName: "message.fill")
                                .font(.system(size: 18, weight: .medium))
                                .foregroundStyle(.white)
                                .frame(width: 38, height: 38)
                                .background(VisioColors.primaryDark100)
                                .clipShape(RoundedRectangle(cornerRadius: 8))

                            if manager.unreadCount > 0 {
                                Text(manager.unreadCount <= 9 ? "\(manager.unreadCount)" : "9+")
                                    .font(.system(size: 10, weight: .bold))
                                    .foregroundStyle(.white)
                                    .padding(.horizontal, 4)
                                    .padding(.vertical, 1)
                                    .background(VisioColors.error500)
                                    .clipShape(Capsule())
                                    .offset(x: 4, y: -4)
                            }
                        }
                    }
                    .accessibilityLabel(Strings.t("chat", lang: lang))
                }

                // More (overflow) button — office only
                if manager.adaptiveMode == .office {
                    Button {
                        withAnimation(.easeInOut(duration: 0.2)) {
                            showOverflow.toggle()
                            if showOverflow {
                                showReactionPicker = false
                            }
                        }
                    } label: {
                        Image(systemName: "ellipsis")
                            .font(.system(size: 18, weight: .medium))
                            .foregroundStyle(.white)
                            .frame(width: 38, height: 38)
                            .background(showOverflow ? VisioColors.primary500 : VisioColors.primaryDark100)
                            .clipShape(RoundedRectangle(cornerRadius: 8))
                    }
                    .accessibilityLabel("More")
                }

                // Hangup
                Button {
                    manager.disconnect()
                    CallKitManager.shared.reportCallEnded()
                    dismiss()
                } label: {
                    Image(systemName: "phone.down.fill")
                        .font(.system(size: buttonIconSize, weight: .medium))
                        .foregroundStyle(.white)
                        .frame(width: buttonSize, height: buttonSize)
                        .background(VisioColors.error500)
                        .clipShape(RoundedRectangle(cornerRadius: buttonCornerRadius))
                }
                .accessibilityLabel(Strings.t("control.leave", lang: lang))
            }
            .padding(isLargeButtons ? 12 : 8)
            .background(manager.adaptiveMode == .office
                ? VisioColors.surface(dark: isDark)
                : Color.black.opacity(0.6)
            )
            .clipShape(RoundedRectangle(cornerRadius: isLargeButtons ? 24 : 16))
            .padding(.horizontal, 8)
        }
        .padding(.bottom, 8)
    }

    // MARK: - Overflow Menu Item

    private func overflowItem(
        icon: String,
        label: String,
        isActive: Bool,
        activeColor: Color,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            VStack(spacing: 4) {
                Image(systemName: icon)
                    .font(.system(size: 18, weight: .medium))
                    .foregroundStyle(isActive ? .black : .white)
                    .frame(width: 38, height: 38)
                    .background(isActive ? activeColor : VisioColors.primaryDark100)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                Text(label)
                    .font(.system(size: 10))
                    .foregroundStyle(.white)
                    .lineLimit(1)
            }
            .accessibilityLabel(Strings.t("control.leave", lang: manager.currentLang))
        }
    }
}

// MARK: - Participant Tile

struct ParticipantTile: View {
    let participant: ParticipantInfo
    var large: Bool = false
    var isActiveSpeaker: Bool = false
    var handRaisePosition: Int = 0
    var isDark: Bool = true

    var body: some View {
        ZStack(alignment: .bottom) {
            // Video or avatar fallback
            if let trackSid = participant.videoTrackSid {
                VideoLayerView(trackSid: trackSid)
            } else {
                avatarView
            }

            // Metadata bar at bottom
            metadataBar
        }
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .stroke(isActiveSpeaker ? VisioColors.primary500 : .clear, lineWidth: 2)
        )
        .shadow(color: isActiveSpeaker ? VisioColors.primary500.opacity(0.5) : .clear, radius: 6)
    }

    private var avatarView: some View {
        ZStack {
            VisioColors.background(dark: isDark)

            Circle()
                .fill(Color(hue: nameHue, saturation: 0.5, brightness: 0.35))
                .frame(width: large ? 80 : 64, height: large ? 80 : 64)
                .overlay(
                    Text(initials)
                        .font(large ? .title : .title2)
                        .bold()
                        .foregroundStyle(.white)
                )
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var metadataBar: some View {
        HStack(spacing: 6) {
            // Mic muted indicator
            if participant.isMuted {
                Image(systemName: "mic.slash.fill")
                    .font(.system(size: 12))
                    .foregroundStyle(VisioColors.error500)
            }

            // Hand raise pill
            if handRaisePosition > 0 {
                HStack(spacing: 2) {
                    Image(systemName: "hand.raised.fill")
                        .font(.system(size: 11))
                    Text("\(handRaisePosition)")
                        .font(.caption2)
                        .bold()
                }
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(VisioColors.handRaise)
                .clipShape(Capsule())
                .foregroundStyle(.black)
            }

            Spacer()

            // Participant name
            Text(participant.name ?? participant.identity)
                .font(.caption)
                .lineLimit(1)
                .foregroundStyle(.white)

            // Connection quality
            connectionQualityIndicator
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(Color.black.opacity(0.6))
    }

    @ViewBuilder
    private var connectionQualityIndicator: some View {
        switch participant.connectionQuality {
        case .excellent:
            Image(systemName: "wifi")
                .font(.system(size: 10))
                .foregroundStyle(.green)
        case .good:
            Image(systemName: "wifi")
                .font(.system(size: 10))
                .foregroundStyle(.yellow)
        case .poor:
            Image(systemName: "wifi.exclamationmark")
                .font(.system(size: 10))
                .foregroundStyle(.orange)
        case .lost:
            Image(systemName: "wifi.slash")
                .font(.system(size: 10))
                .foregroundStyle(VisioColors.error500)
        }
    }

    // MARK: - Helpers

    private var initials: String {
        let name = participant.name ?? participant.identity
        let parts = name.split(separator: " ")
        if parts.count >= 2 {
            return String(parts[0].prefix(1) + parts[1].prefix(1)).uppercased()
        }
        return String(name.prefix(2)).uppercased()
    }

    private var nameHue: Double {
        let name = participant.name ?? participant.identity
        let hash = name.unicodeScalars.reduce(0) { $0 + Int($1.value) }
        return Double(hash % 360) / 360.0
    }
}

// MARK: - Audio Device Sheet

struct AudioDeviceSheet: View {
    @EnvironmentObject private var manager: VisioManager
    @State private var availableInputs: [AVAudioSessionPortDescription] = []
    @State private var currentOutputs: [AVAudioSessionPortDescription] = []
    @State private var currentInput: AVAudioSessionPortDescription?
    @State private var isSpeakerOverride: Bool = false
    @Environment(\.dismiss) private var dismiss

    private var lang: String { manager.currentLang }
    private var isDark: Bool { manager.currentTheme == "dark" }

    var body: some View {
        NavigationStack {
            List {
                Section(Strings.t("audio.output", lang: lang)) {
                    Button {
                        do { try AVAudioSession.sharedInstance().overrideOutputAudioPort(.speaker) }
                        catch { NSLog("Failed to override audio to speaker: %@", error.localizedDescription) }
                        loadDevices()
                    } label: {
                        HStack {
                            Image(systemName: "speaker.wave.2.fill")
                                .foregroundStyle(VisioColors.primary500)
                            Text(Strings.t("audio.speaker", lang: lang))
                                .foregroundStyle(VisioColors.onSurface(dark: isDark))
                            Spacer()
                            if isSpeakerOverride {
                                Image(systemName: "checkmark")
                                    .foregroundStyle(VisioColors.primary500)
                            }
                        }
                    }

                    Button {
                        do { try AVAudioSession.sharedInstance().overrideOutputAudioPort(.none) }
                        catch { NSLog("Failed to override audio to earpiece: %@", error.localizedDescription) }
                        loadDevices()
                    } label: {
                        HStack {
                            Image(systemName: "iphone")
                                .foregroundStyle(VisioColors.primary500)
                            Text(Strings.t("audio.earpiece", lang: lang))
                                .foregroundStyle(VisioColors.onSurface(dark: isDark))
                            Spacer()
                            if !isSpeakerOverride && currentOutputs.contains(where: { $0.portType == .builtInReceiver || $0.portType == .builtInSpeaker }) && !currentOutputs.contains(where: { isExternalOutput($0) }) {
                                Image(systemName: "checkmark")
                                    .foregroundStyle(VisioColors.primary500)
                            }
                        }
                    }

                    ForEach(currentOutputs.filter { isExternalOutput($0) }, id: \.uid) { port in
                        HStack {
                            Image(systemName: iconForOutputPort(port))
                                .foregroundStyle(VisioColors.primary500)
                            Text(port.portName)
                                .foregroundStyle(VisioColors.onSurface(dark: isDark))
                            Spacer()
                            Image(systemName: "checkmark")
                                .foregroundStyle(VisioColors.primary500)
                        }
                    }
                }

                Section(Strings.t("audio.input", lang: lang)) {
                    ForEach(availableInputs, id: \.uid) { port in
                        Button {
                            selectInput(port)
                        } label: {
                            HStack {
                                Image(systemName: iconForInputPort(port))
                                    .foregroundStyle(VisioColors.primary500)
                                Text(port.portName)
                                    .foregroundStyle(VisioColors.onSurface(dark: isDark))
                                Spacer()
                                if port.uid == currentInput?.uid {
                                    Image(systemName: "checkmark")
                                        .foregroundStyle(VisioColors.primary500)
                                }
                            }
                        }
                    }
                }
            }
            .scrollContentBackground(.hidden)
            .background(VisioColors.background(dark: isDark))
            .navigationTitle(Strings.t("audio.source", lang: lang))
            .navigationBarTitleDisplayMode(.inline)
            .toolbarColorScheme(isDark ? .dark : .light, for: .navigationBar)
            .toolbarBackground(VisioColors.surface(dark: isDark), for: .navigationBar)
            .toolbarBackground(.visible, for: .navigationBar)
            .appToolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(Strings.t("audio.done", lang: lang)) { dismiss() }
                        .foregroundStyle(VisioColors.primary500)
                }
            }
        }
        .onAppear { loadDevices() }
    }

    private func loadDevices() {
        let session = AVAudioSession.sharedInstance()
        availableInputs = session.availableInputs ?? []
        currentInput = session.currentRoute.inputs.first
        currentOutputs = session.currentRoute.outputs
        isSpeakerOverride = currentOutputs.contains { $0.portType == .builtInSpeaker }
    }

    private func selectInput(_ port: AVAudioSessionPortDescription) {
        do { try AVAudioSession.sharedInstance().setPreferredInput(port) }
        catch { NSLog("Failed to set preferred input: %@", error.localizedDescription) }
        currentInput = port
    }

    private func isExternalOutput(_ port: AVAudioSessionPortDescription) -> Bool {
        switch port.portType {
        case .bluetoothA2DP, .bluetoothLE, .bluetoothHFP, .headphones, .airPlay, .carAudio, .usbAudio:
            return true
        default:
            return false
        }
    }

    private func iconForOutputPort(_ port: AVAudioSessionPortDescription) -> String {
        switch port.portType {
        case .bluetoothA2DP, .bluetoothLE, .bluetoothHFP:
            return "wave.3.right"
        case .headphones:
            return "headphones"
        case .airPlay:
            return "airplayaudio"
        case .carAudio:
            return "car"
        default:
            return "speaker.wave.2"
        }
    }

    private func iconForInputPort(_ port: AVAudioSessionPortDescription) -> String {
        switch port.portType {
        case .bluetoothHFP:
            return "wave.3.right"
        case .headsetMic:
            return "headphones"
        case .builtInMic:
            return "iphone"
        default:
            return "mic"
        }
    }
}

// MARK: - Reaction Overlay

struct ReactionOverlay: View {
    let reactions: [ReactionData]

    private static let reactionEmojis: [(id: String, emoji: String)] = [
        ("thumbs-up", "\u{1F44D}"),
        ("thumbs-down", "\u{1F44E}"),
        ("clapping-hands", "\u{1F44F}"),
        ("red-heart", "\u{2764}\u{FE0F}"),
        ("face-with-tears-of-joy", "\u{1F602}"),
        ("face-with-open-mouth", "\u{1F62E}"),
        ("party-popper", "\u{1F389}"),
        ("folded-hands", "\u{1F64F}"),
    ]

    var body: some View {
        let now = Date()
        let active = reactions.filter { now.timeIntervalSince($0.timestamp) < 3.0 }

        GeometryReader { geo in
            ZStack(alignment: .bottomLeading) {
                Color.clear

                ForEach(active) { reaction in
                    FloatingReaction(
                        reaction: reaction,
                        emojiDisplay: Self.reactionEmojis.first(where: { $0.id == reaction.emoji })?.emoji ?? reaction.emoji,
                        screenWidth: geo.size.width
                    )
                }
            }
        }
        .allowsHitTesting(false)
    }
}

struct FloatingReaction: View {
    let reaction: ReactionData
    let emojiDisplay: String
    let screenWidth: CGFloat

    @State private var progress: CGFloat = 0

    var body: some View {
        let xOffset = CGFloat(abs((reaction.id * 37 + 13) % Int64(max(screenWidth * 0.2, 1))))
        let yOffset = -300.0 * progress
        let alpha = progress > 0.7 ? 1.0 - ((progress - 0.7) / 0.3) : 1.0

        VStack(spacing: 2) {
            Text(emojiDisplay)
                .font(.system(size: 32))
            Text(reaction.participantName)
                .font(.system(size: 10))
                .foregroundStyle(.white)
                .lineLimit(1)
                .padding(.horizontal, 4)
                .padding(.vertical, 1)
                .background(Color.black.opacity(0.6))
                .clipShape(RoundedRectangle(cornerRadius: 4))
        }
        .offset(x: xOffset, y: yOffset)
        .opacity(alpha)
        .onAppear {
            withAnimation(.linear(duration: 3.0)) {
                progress = 1.0
            }
        }
    }
}

#Preview {
    NavigationStack {
        CallView(roomURL: "meet.example.com/test", displayName: "Alice")
            .environmentObject(VisioManager())
    }
}
