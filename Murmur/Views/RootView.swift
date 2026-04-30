import SwiftUI
import SwiftData
import MurmurCore
import StudioAnalytics
import os.log

private let entryLog = Logger(subsystem: Bundle.main.bundleIdentifier ?? "murmur", category: "Entries")

struct RootView: View {
    @Environment(AppState.self) private var appState
    @Environment(NotificationPreferences.self) private var notifPrefs
    @Environment(\.modelContext) private var modelContext
    @Environment(\.scenePhase) private var scenePhase
    @Query(sort: \Entry.createdAt, order: .reverse) private var entries: [Entry]
    @State private var selectedEntry: Entry?
    @State private var inputText = ""
    @State private var toastConfig: ToastContainer.ToastConfig?
    @State private var showDevMode = false
    @State private var showTextInputBar = false
    @State private var showSettings = false
    @State private var showCalendar = false
    @State private var showHelp = false
    @State private var showTopUp = false
    @State private var showOutOfCredits = false
    @State private var isPurchasingTopUp = false
    @State private var isLoadingTopUpProducts = false
    @State private var topUpPacks: [CreditPack] = []
    @State private var topUpProductIDByCredits: [Int64: String] = [:]
    @AppStorage("homeVariant") private var homeVariant: String = "zones"
    @State private var hintStep: Int = -1
    @State private var hintTimerTask: Task<Void, Never>?
    @State private var pendingDeleteEntry: Entry?
    @State private var pendingDeleteTask: Task<Void, Never>?
    @State private var snoozeEntry: Entry?
    @State private var showSnoozeDialog = false
    @State private var showCustomSnoozeSheet = false
    @State private var hasAppeared = false
    private let topUpService = StoreKitTopUpService()

    var body: some View {
        ZStack {
            // Main content based on selected tab
            mainContent
                .preferredColorScheme(.dark)

            // Dev Mode floating button (always accessible once activated)
            #if DEBUG
            if appState.isDevMode {
                VStack {
                    HStack {
                        Button {
                            showDevMode = true
                        } label: {
                            Image(systemName: "hammer.fill")
                                .font(.subheadline.weight(.semibold))
                                .foregroundStyle(.white)
                                .frame(width: 36, height: 36)
                                .background(
                                    Circle()
                                        .fill(Theme.Colors.accentPurple.opacity(0.8))
                                )
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Developer tools")
                        .padding(.leading, 16)
                        .padding(.top, 54)

                        Spacer()
                    }
                    Spacer()
                }
                .zIndex(90)
            }
            #endif

            // Onboarding overlay (highest priority)
            if appState.showOnboarding {
                OnboardingFlowView(
                    onComplete: {
                        withAnimation {
                            appState.showOnboarding = false
                        }
                        Task { @MainActor in
                            try? await Task.sleep(for: .seconds(0.6))
                            withAnimation(.spring(response: 0.5, dampingFraction: 0.8)) {
                                hintStep = 0
                            }
                            scheduleHintTimer()
                        }
                    }
                )
                .transition(.opacity)
                .zIndex(100)
            }

            // Conversation overlays
            if !appState.showOnboarding {
                let conversation = appState.conversation
                // Recording state: waveform + floating transcript
                if conversation.isRecording {
                    let transcript: String = {
                        if case .recording(let t) = conversation.inputState { return t }
                        return ""
                    }()
                    RecordingStateView(
                        transcript: transcript,
                        audioLevels: conversation.audioLevels
                    )
                    .transition(.opacity.animation(.easeInOut(duration: 0.35)))
                    .zIndex(20)
                }
                // Processing edge glow (behind stream overlay)
                if conversation.isProcessing {
                    ProcessingGlowView()
                        .transition(.opacity)
                        .zIndex(15)
                }
            }

            // Post-onboarding sequential hints
            hintOverlay

            // Tap-to-dismiss overlay when text input is open
            if showTextInputBar {
                Color.clear
                    .contentShape(Rectangle())
                    .onTapGesture {
                        showTextInputBar = false
                        inputText = ""
                    }
                    .zIndex(49)
            }

            // Bottom nav bar — always above overlays
            if !appState.showOnboarding {
                VStack {
                    Spacer()
                    let conversation = appState.conversation
                    BottomNavBar(
                        isRecording: conversation.isRecording,
                        isProcessing: conversation.isProcessing,
                        showTextInput: showTextInputBar,
                        inputText: $inputText,
                        onMicTap: { toggleRecording() },
                        onKeyboardTap: { showTextInputBar = true },
                        selectedTab: appState.selectedTab,
                        onTabChange: { appState.selectedTab = $0 },
                        onTextSubmit: { submitInput() },
                        onDismissTextInput: {
                            showTextInputBar = false
                            inputText = ""
                        }
                    )
                    .padding(.bottom, 16)
                }
                .zIndex(50)
            }

        }
        .toast($toastConfig)
        #if DEBUG
        .sheet(isPresented: $showDevMode, onDismiss: {
            if appState.homeComposition == nil && !appState.isHomeCompositionLoading {
                Task { @MainActor in
                    await appState.requestHomeComposition(
                        entries: activeEntries,
                        variant: currentVariant
                    )
                }
            }
        }) {
            DevModeView()
        }
        #endif
        .sheet(item: $selectedEntry) { entry in
            EntryDetailView(
                entry: entry,
                onBack: { selectedEntry = nil },
                onAction: { action in
                    selectedEntry = nil
                    handleEntryAction(entry, action)
                }
            )
            .environment(appState)
            .environment(notifPrefs)
        }
        .sheet(isPresented: $showTopUp) {
            TopUpView(
                packs: topUpPacks,
                isLoading: isLoadingTopUpProducts,
                onBack: { showTopUp = false },
                onPurchase: { pack in
                    handleTopUpPurchase(pack)
                }
            )
        }
        .fullScreenCover(isPresented: $showOutOfCredits) {
            OutOfCreditsView(
                onTopUp: {
                    showOutOfCredits = false
                    openTopUp()
                    Task { @MainActor in
                        try? await Task.sleep(for: .seconds(0.6))
                        showTopUp = true
                    }
                }
            )
        }
        .confirmationDialog("Snooze until...", isPresented: $showSnoozeDialog) {
            Button("In 1 hour") {
                performSnooze(.hour, value: 1)
            }
            Button("Tomorrow morning") {
                let tomorrow = Calendar.current.date(byAdding: .day, value: 1, to: Date()) ?? Date()
                let date = Calendar.current.date(bySettingHour: 9, minute: 0, second: 0, of: tomorrow)
                commitSnooze(until: date)
            }
            Button("Next week") {
                performSnooze(.weekOfYear, value: 1)
            }
            Button("Custom time...") {
                showCustomSnoozeSheet = true
            }
            Button("Cancel", role: .cancel) {
                snoozeEntry = nil
            }
        }
        .sheet(isPresented: $showCustomSnoozeSheet, onDismiss: { snoozeEntry = nil }) {
            SnoozePickerSheet(
                onSave: { date in
                    commitSnooze(until: date)
                    showCustomSnoozeSheet = false
                },
                onDismiss: { showCustomSnoozeSheet = false }
            )
        }
        .onAppear {
            wakeUpSnoozedEntries()
            if !appState.hasCompletedOnboarding {
                appState.showOnboarding = true
            }
            Task { @MainActor in
                await appState.refreshCreditBalance()
            }
            Task { @MainActor in
                await appState.requestHomeComposition(
                    entries: activeEntries,
                    variant: currentVariant
                )
                hasAppeared = true
            }
        }
        .onChange(of: scenePhase) { _, phase in
            guard hasAppeared else { return }
            if phase == .active {
                wakeUpSnoozedEntries()
                appState.startNewSession()
                if appState.homeComposition != nil {
                    appState.requestLayoutRefresh(
                        entries: activeEntries,
                        variant: currentVariant
                    )
                } else {
                    Task { @MainActor in
                        await appState.requestHomeComposition(
                            entries: activeEntries,
                            variant: currentVariant
                        )
                    }
                }
            }
        }
        .onChange(of: homeVariant) { _, _ in
            appState.refreshTask?.cancel()
            appState.invalidateHomeComposition()
            appState.resetConversation()
            Task { @MainActor in
                await appState.requestHomeComposition(
                    entries: activeEntries,
                    variant: currentVariant
                )
            }
        }
        .onReceive(Timer.publish(every: 30, on: .main, in: .common).autoconnect()) { _ in
            wakeUpSnoozedEntries()
        }
        .onReceive(NotificationCenter.default.publisher(for: .murmurOpenEntry)) { notification in
            guard let uuid = notification.userInfo?["entryID"] as? UUID else { return }
            selectedEntry = entries.first { $0.id == uuid }
        }
        .onChange(of: appState.conversation.completionText) { _, text in
            guard let text, !text.isEmpty else { return }
            showToast(text, type: .info)
            appState.conversation.completionText = nil
        }
        .onChange(of: appState.conversation.outOfCreditsInfo) { _, triggered in
            guard triggered else { return }
            appState.conversation.outOfCreditsInfo = false
            showOutOfCredits = true
        }
        .onReceive(NotificationCenter.default.publisher(for: .murmurShowError)) { notification in
            guard let info = notification.userInfo,
                  let kind = info["kind"] as? String else { return }
            switch kind {
            case "micDenied":
                showToast(
                    "Microphone access needed",
                    type: .error,
                    actionLabel: "Settings"
                ) {
                    if let url = URL(string: UIApplication.openSettingsURLString) {
                        UIApplication.shared.open(url)
                    }
                }
            case "pipelineUnavailable":
                showToast("Voice processing unavailable", type: .error)
            case "processingFailed":
                let message = info["message"] as? String ?? "Couldn't process — try again."
                let isWarning = info["isWarning"] as? Bool ?? false
                showToast(message, type: isWarning ? .warning : .error)
            default:
                break
            }
        }
    }

    // MARK: - Main Content

    @ViewBuilder
    private var mainContent: some View {
        homeContent
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Theme.Colors.bgDeep)
            .safeAreaInset(edge: .bottom, spacing: 0) {
                // Spacer matching BottomNavBar height (+ 16pt lift off home indicator)
                Color.clear.frame(height: Theme.Spacing.micButtonSize + 16)
            }
            .ignoresSafeArea(.keyboard)
            .sheet(isPresented: $showSettings) {
                SettingsFullView(
                    onBack: { showSettings = false },
                    onTopUp: {
                        showSettings = false
                        openTopUp()
                        Task { @MainActor in
                            try? await Task.sleep(for: .seconds(0.45))
                            showTopUp = true
                        }
                    }
                )
            }
            .sheet(isPresented: $showCalendar) {
                CalendarView(onEntryTap: { selectedEntry = $0 })
            }
            .sheet(isPresented: $showHelp) {
                HelpView(onBack: { showHelp = false })
            }
    }

    // MARK: - Home Content

    private func toggleRecording() {
        let conversation = appState.conversation
        if conversation.isRecording {
            conversation.stopRecording(
                entries: entries,
                modelContext: modelContext,
                preferences: notifPrefs
            )
        } else {
            conversation.startRecording()
        }
    }

    private func submitInput() {
        let conversation = appState.conversation
        conversation.inputText = inputText
        inputText = ""
        showTextInputBar = false
        conversation.submitText(
            entries: entries,
            modelContext: modelContext,
            preferences: notifPrefs
        )
    }

    @ViewBuilder
    private var homeContent: some View {
        if homeVariant == "scanner" {
            DamHomeView(
                inputText: $inputText,
                entries: activeEntries,
                onMicTap: toggleRecording,
                onSubmit: submitInput,
                onEntryTap: { selectedEntry = $0 },
                onSettingsTap: { showSettings = true },
                onAction: { handleEntryAction($0, $1) }
            )
        } else {
            ZonedFocusHomeView(
                inputText: $inputText,
                entries: activeEntries,
                onMicTap: toggleRecording,
                onSubmit: submitInput,
                onEntryTap: { selectedEntry = $0 },
                onKeyboardTap: { showTextInputBar = true },
                onSettingsTap: { showSettings = true },
                onHelpTap: { showHelp = true },
                onCalendarTap: { showCalendar = true },
                onAction: { handleEntryAction($0, $1) }
            )
        }
    }

}

// MARK: - Actions & Helpers

private extension RootView {

    struct HintItem {
        let icon: String
        let text: String
    }

    var onboardingHints: [HintItem] {
        [
            HintItem(icon: "sparkles", text: "Focus = AI's top picks · All = everything"),
            HintItem(icon: "arrow.left", text: "Swipe left to snooze or complete"),
            HintItem(icon: "circle", text: "Tap the circle on habits to check them off"),
            HintItem(icon: "calendar", text: "Tap calendar to see your schedule"),
            HintItem(icon: "bell", text: "Customize notification preferences in Settings"),
        ]
    }

    func advanceHint() {
        hintTimerTask?.cancel()
        withAnimation(.spring(response: 0.4, dampingFraction: 0.82)) {
            hintStep += 1
        }
        scheduleHintTimer()
    }

    func scheduleHintTimer() {
        hintTimerTask?.cancel()
        guard hintStep < onboardingHints.count else { return }
        hintTimerTask = Task { @MainActor in
            try? await Task.sleep(for: .seconds(4.5))
            guard !Task.isCancelled else { return }
            advanceHint()
        }
    }

    @ViewBuilder var hintOverlay: some View {
        if hintStep >= 0 && hintStep < onboardingHints.count {
            let hint = onboardingHints[hintStep]
            VStack {
                Spacer()
                VStack(spacing: 12) {
                    HStack(spacing: 12) {
                        Image(systemName: hint.icon)
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(Theme.Colors.accentPurple)
                            .frame(width: 32, height: 32)
                            .background(Theme.Colors.accentPurple.opacity(0.12), in: Circle())
                            .id("icon_\(hintStep)")
                            .transition(.asymmetric(
                                insertion: .opacity.combined(with: .offset(y: 6)),
                                removal: .opacity.combined(with: .offset(y: -6))
                            ))
                        Text(hint.text)
                            .font(Theme.Typography.body)
                            .foregroundStyle(Theme.Colors.textPrimary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .id("text_\(hintStep)")
                            .transition(.asymmetric(
                                insertion: .opacity.combined(with: .offset(y: 6)),
                                removal: .opacity.combined(with: .offset(y: -6))
                            ))
                    }
                    HStack(spacing: 6) {
                        ForEach(0..<onboardingHints.count, id: \.self) { i in
                            Capsule()
                                .fill(i == hintStep ? Theme.Colors.accentPurple : Theme.Colors.borderSubtle)
                                .frame(width: i == hintStep ? 16 : 6, height: 4)
                                .animation(.spring(response: 0.3, dampingFraction: 0.8), value: hintStep)
                        }
                        Spacer()
                        let isLast = hintStep == onboardingHints.count - 1
                        Text(isLast ? "Got it" : "Tap to continue")
                            .font(Theme.Typography.caption)
                            .foregroundStyle(isLast ? Theme.Colors.accentPurple : Theme.Colors.textTertiary)
                    }
                }
                .padding(.horizontal, 18)
                .padding(.vertical, 16)
                .background(
                    RoundedRectangle(cornerRadius: Theme.Spacing.cardRadius)
                        .fill(Theme.Colors.bgCard)
                        .overlay(
                            RoundedRectangle(cornerRadius: Theme.Spacing.cardRadius)
                                .stroke(Theme.Colors.accentPurple.opacity(0.2), lineWidth: 1)
                        )
                )
                .shadow(color: .black.opacity(0.3), radius: 20, y: 8)
                .padding(.horizontal, Theme.Spacing.screenPadding)
                .padding(.bottom, Theme.Spacing.micButtonSize + 28)
                .onTapGesture { advanceHint() }
            }
            .transition(.move(edge: .bottom).combined(with: .opacity))
            .zIndex(55)
        }
    }

    var currentVariant: CompositionVariant {
        homeVariant == "scanner" ? .scanner : .navigator
    }

    func handleTopUpPurchase(_ pack: CreditPack) {
        guard !isPurchasingTopUp else { return }

        isPurchasingTopUp = true
        Task { @MainActor in
            defer { isPurchasingTopUp = false }
            do {
                let credits = Int64(pack.credits)
                guard let productID = topUpProductIDByCredits[credits] else {
                    showToast("Top-up product unavailable in StoreKit. Check IAP IDs/config.", type: .error)
                    return
                }

                let receipt = try await topUpService.purchase(productID: productID)
                try await appState.applyTopUp(credits: receipt.creditsGranted)
                showTopUp = false
                showToast("Top-up successful: +\(receipt.creditsGranted.formatted()) credits")
            } catch let error as StoreKitTopUpError {
                switch error {
                case .userCancelled:
                    break
                case .pending:
                    showToast("Purchase pending approval.", type: .warning)
                default:
                    showToast("Top-up failed: \(error.localizedDescription)", type: .error)
                }
            } catch {
                showToast("Top-up failed: \(error.localizedDescription)", type: .error)
            }
        }
    }

    func handleEntryAction(_ entry: Entry, _ action: EntryAction) {
        switch action {
        case .complete:
            UINotificationFeedbackGenerator().notificationOccurred(.success)
            entry.perform(.complete, in: modelContext, preferences: notifPrefs)
            StudioAnalytics.track(EntryCompleted(
                entryId: entry.id.uuidString,
                category: entry.category.rawValue,
                timeSinceCreationMs: Int(Date().timeIntervalSince(entry.createdAt) * 1000),
                source: "user"
            ))
            showToast("Completed")

        case .archive:
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
            entry.perform(.archive, in: modelContext, preferences: notifPrefs)
            StudioAnalytics.track(EntryArchived(
                entryId: entry.id.uuidString,
                category: entry.category.rawValue,
                timeSinceCreationMs: Int(Date().timeIntervalSince(entry.createdAt) * 1000),
                source: "user"
            ))
            showToast("Archived", type: .info)

        case .unarchive:
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
            entry.perform(.unarchive, in: modelContext, preferences: notifPrefs)
            showToast("Restored")

        case .delete:
            UIImpactFeedbackGenerator(style: .rigid).impactOccurred()
            pendingDeleteTask?.cancel()
            pendingDeleteEntry = entry
            let deleteEntryId = entry.id.uuidString
            let deleteCategory = entry.category.rawValue
            let deleteAgeMs = Int(Date().timeIntervalSince(entry.createdAt) * 1000)
            showToast("Deleted", type: .warning, actionLabel: "Undo") {
                pendingDeleteTask?.cancel()
                pendingDeleteEntry = nil
            }
            pendingDeleteTask = Task { @MainActor in
                try? await Task.sleep(for: .seconds(4))
                guard let pending = pendingDeleteEntry else { return }
                pending.perform(.delete, in: modelContext, preferences: notifPrefs)
                StudioAnalytics.track(EntryDeleted(
                    entryId: deleteEntryId,
                    category: deleteCategory,
                    timeSinceCreationMs: deleteAgeMs,
                    source: "user"
                ))
                pendingDeleteEntry = nil
            }

        case .checkOffHabit:
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
            entry.perform(.checkOffHabit, in: modelContext, preferences: notifPrefs)

        case .toggleListItem(let index):
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
            entry.perform(.toggleListItem(index: index), in: modelContext, preferences: notifPrefs)

        case .snooze(nil):
            snoozeEntry = entry
            showSnoozeDialog = true

        case .snooze(let until):
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
            entry.perform(.snooze(until: until), in: modelContext, preferences: notifPrefs)
            showToast("Snoozed", type: .info)

        case .wake:
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
            entry.perform(.wake, in: modelContext, preferences: notifPrefs)
            showToast("Awake", type: .info)
        }
    }

    func showToast(
        _ message: String,
        type: ToastView.ToastType = .success,
        actionLabel: String? = nil,
        action: (() -> Void)? = nil
    ) {
        toastConfig = ToastContainer.ToastConfig(
            message: message,
            type: type,
            duration: action != nil ? 4.5 : max(2.5, min(6.0, Double(message.count) / 12.0)),
            actionLabel: actionLabel,
            action: action
        )
    }

    var activeEntries: [Entry] {
        let pendingReveal = appState.conversation.pendingRevealEntryIDs
        return entries.filter {
            $0.status == .active
                && $0.persistentModelID != pendingDeleteEntry?.persistentModelID
                && !pendingReveal.contains($0.id)
        }
    }

    func performSnooze(_ component: Calendar.Component, value: Int) {
        let date = Calendar.current.date(byAdding: component, value: value, to: Date())
        commitSnooze(until: date)
    }

    func commitSnooze(until date: Date?) {
        guard let entry = snoozeEntry else { return }
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        entry.perform(.snooze(until: date), in: modelContext, preferences: notifPrefs)
        showToast("Snoozed", type: .info)
        snoozeEntry = nil
    }

    func wakeUpSnoozedEntries() {
        let now = Date()
        var woken: [Entry] = []
        for entry in entries where entry.status == .snoozed {
            if let snoozeUntil = entry.snoozeUntil, snoozeUntil <= now {
                entry.status = .active
                entry.snoozeUntil = nil
                entry.updatedAt = now
                woken.append(entry)
            }
        }
        if !woken.isEmpty {
            do {
                try modelContext.save()
            } catch {
                entryLog.error("Failed to save woken entries: \(error.localizedDescription)")
            }
            for entry in woken {
                NotificationService.shared.sync(entry, preferences: notifPrefs)
            }
        }
    }

    private func openTopUp() {
        Task { @MainActor in
            await loadTopUpProducts()
        }
    }

    private func loadTopUpProducts() async {
        if isLoadingTopUpProducts { return }
        isLoadingTopUpProducts = true
        defer { isLoadingTopUpProducts = false }

        do {
            let products = try await topUpService.loadProducts()
            topUpPacks = products.enumerated().map { index, product in
                CreditPack(
                    credits: Int(product.credits),
                    price: product.priceText,
                    isPopular: index == 1,
                    isBestValue: index == (products.count - 1)
                )
            }
            topUpProductIDByCredits = Dictionary(
                uniqueKeysWithValues: products.map { ($0.credits, $0.id) }
            )
        } catch {
            topUpPacks = []
            topUpProductIDByCredits = [:]
            showToast("Failed to load purchases: \(error.localizedDescription)", type: .error)
        }
    }

}

#Preview("Empty") {
    @Previewable @State var appState = AppState()

    return RootView()
        .environment(appState)
}

#Preview("With Onboarding") {
    @Previewable @State var appState = AppState()

    appState.showOnboarding = true

    return RootView()
        .environment(appState)
}

#Preview("Recording State") {
    @Previewable @State var appState = AppState()

    return RootView()
        .environment(appState)
}
