import SwiftUI
import MurmurCore
import StudioAnalytics

// MARK: - All Entries View (category sections browser, shared by both home variants)

struct AllEntriesView: View {
    let entries: [Entry]
    let snoozedEntries: [Entry]
    let isProcessing: Bool
    let arrivedEntryIDs: Set<UUID>
    @Binding var activeSwipeEntryID: UUID?
    let onEntryTap: (Entry) -> Void
    let swipeActionsProvider: (Entry) -> [CardSwipeAction]
    let onAction: (Entry, EntryAction) -> Void
    let onGlowComplete: (UUID) -> Void

    @State private var searchText = ""
    @State private var expandedListIDs: Set<UUID> = []
    @State private var showSnoozed = false

    private static let categoryDisplayOrder: [EntryCategory] = [
        .todo, .reminder, .habit, .idea, .list, .note, .question
    ]

    private var combinedEntries: [Entry] {
        showSnoozed ? entries + snoozedEntries : entries
    }

    private var filteredEntries: [Entry] {
        let source = combinedEntries
        guard !searchText.isEmpty else { return source }
        let q = searchText.lowercased()
        return source.filter { $0.summary.lowercased().contains(q) }
    }

    private var entriesByCategory: [(category: EntryCategory, entries: [Entry])] {
        let grouped = Dictionary(grouping: filteredEntries) { $0.category }
        return Self.categoryDisplayOrder.compactMap { category in
            guard let items = grouped[category], !items.isEmpty else { return nil }
            let sorted = items.sorted { lhs, rhs in
                let pa = lhs.priority ?? Int.max
                let pb = rhs.priority ?? Int.max
                if pa != pb { return pa < pb }
                let da = lhs.dueDate ?? Date.distantFuture
                let db = rhs.dueDate ?? Date.distantFuture
                if da != db { return da < db }
                return lhs.createdAt > rhs.createdAt
            }
            return (category: category, entries: sorted)
        }
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 0) {
                // Search bar + snooze filter toggle
                HStack(spacing: 8) {
                    searchBar
                    snoozeToggleButton
                }
                .padding(.horizontal, Theme.Spacing.screenPadding)
                .padding(.top, 12)
                .padding(.bottom, 4)

                if isProcessing && searchText.isEmpty {
                    SharedProcessingDotsView()
                        .transition(.opacity)
                }

                if !searchText.isEmpty {
                    if filteredEntries.isEmpty {
                        VStack(spacing: 12) {
                            Image(systemName: "magnifyingglass")
                                .font(.system(size: 32))
                                .foregroundStyle(Theme.Colors.textTertiary)
                            Text("No results for \"\(searchText)\"")
                                .font(Theme.Typography.body)
                                .foregroundStyle(Theme.Colors.textSecondary)
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.top, 60)
                        .transition(.opacity)
                    } else {
                        LazyVStack(spacing: 12) {
                            ForEach(filteredEntries) { entry in
                                if entry.category == .list {
                                    GlowingEntryRow(
                                        entry: entry,
                                        isArrived: arrivedEntryIDs.contains(entry.id),
                                        category: entry.category,
                                        onAction: onAction,
                                        onTap: { onEntryTap(entry) },
                                        listExpanded: Binding(
                                            get: { expandedListIDs.contains(entry.id) },
                                            set: { if $0 { expandedListIDs.insert(entry.id) } else { expandedListIDs.remove(entry.id) } }
                                        ),
                                        onGlowComplete: { onGlowComplete(entry.id) }
                                    )
                                } else {
                                    SwipeableCard(
                                        actions: swipeActionsProvider(entry),
                                        activeSwipeID: $activeSwipeEntryID,
                                        entryID: entry.id,
                                        onTap: searchTapAction(entry)
                                    ) {
                                        GlowingEntryRow(
                                            entry: entry,
                                            isArrived: arrivedEntryIDs.contains(entry.id),
                                            category: entry.category,
                                            onAction: onAction,
                                            onGlowComplete: { onGlowComplete(entry.id) }
                                        )
                                    }
                                }
                            }
                        }
                        .padding(.horizontal, 12)
                        .padding(.top, 8)
                        .transition(.opacity)
                    }
                } else {
                    VStack(spacing: 0) {
                        ForEach(entriesByCategory, id: \.category) { group in
                            CategorySectionView(
                                category: group.category,
                                entries: group.entries,
                                arrivedEntryIDs: arrivedEntryIDs,
                                activeSwipeEntryID: $activeSwipeEntryID,
                                onEntryTap: onEntryTap,
                                swipeActionsProvider: swipeActionsProvider,
                                onAction: onAction,
                                onGlowComplete: onGlowComplete
                            )
                        }
                    }
                    .animation(Animations.smoothSlide, value: combinedEntries.map(\.id))
                }

                Color.clear.frame(height: 160)
            }
        }
        .scrollIndicators(.hidden)
    }

    private func searchTapAction(_ entry: Entry) -> () -> Void {
        if entry.category == .habit {
            return { if entry.appliesToday { onAction(entry, .checkOffHabit) } }
        } else if entry.category == .list {
            return {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.8)) {
                    if expandedListIDs.contains(entry.id) { expandedListIDs.remove(entry.id) } else { expandedListIDs.insert(entry.id) }
                }
            }
        }
        return { onEntryTap(entry) }
    }

    @ViewBuilder
    private var searchBar: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 14, weight: .medium))
                .foregroundStyle(Theme.Colors.textTertiary)

            TextField("Search entries", text: $searchText)
                .font(Theme.Typography.body)
                .foregroundStyle(Theme.Colors.textPrimary)
                .autocorrectionDisabled()

            if !searchText.isEmpty {
                Button { searchText = "" } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 15))
                        .foregroundStyle(Theme.Colors.textTertiary)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(
            RoundedRectangle(cornerRadius: 10)
                .fill(Theme.Colors.bgCard)
                .overlay(
                    RoundedRectangle(cornerRadius: 10)
                        .stroke(Theme.Colors.borderSubtle, lineWidth: 1)
                )
        )
    }

    @ViewBuilder
    private var snoozeToggleButton: some View {
        Button {
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
            let newValue = !showSnoozed
            StudioAnalytics.track(AllViewSnoozeFilterToggled(
                enabled: newValue,
                snoozedCount: snoozedEntries.count
            ))
            withAnimation(.easeInOut(duration: 0.2)) {
                showSnoozed = newValue
            }
        } label: {
            Image(systemName: showSnoozed ? "moon.zzz.fill" : "moon.zzz")
                .font(.system(size: 16, weight: .medium))
                .foregroundStyle(showSnoozed ? Theme.Colors.accentYellow : Theme.Colors.textTertiary)
                .frame(width: 40, height: 40)
                .background(
                    RoundedRectangle(cornerRadius: 10)
                        .fill(Theme.Colors.bgCard)
                        .overlay(
                            RoundedRectangle(cornerRadius: 10)
                                .stroke(Theme.Colors.borderSubtle, lineWidth: 1)
                        )
                )
                .shadow(
                    color: showSnoozed ? Theme.Colors.accentYellow.opacity(0.4) : .clear,
                    radius: showSnoozed ? 6 : 0
                )
        }
        .buttonStyle(.plain)
        .accessibilityLabel(showSnoozed ? "Hide snoozed entries" : "Show snoozed entries")
    }
}

// MARK: - Category Section

struct CategorySectionView: View {
    let category: EntryCategory
    let entries: [Entry]
    let arrivedEntryIDs: Set<UUID>
    @Binding var activeSwipeEntryID: UUID?
    let onEntryTap: (Entry) -> Void
    let swipeActionsProvider: (Entry) -> [CardSwipeAction]
    let onAction: (Entry, EntryAction) -> Void
    let onGlowComplete: (UUID) -> Void

    @AppStorage private var isCollapsed: Bool
    @State private var expandedListIDs: Set<UUID> = []

    // MARK: - Peek State (collapsed section arrival preview)
    @State private var peekEntry: Entry?
    @State private var peekCount: Int = 0
    @State private var peekVisible: Bool = false
    @State private var peekTask: Task<Void, Never>?
    @State private var headerGlowIntensity: Double = 0

    init(
        category: EntryCategory,
        entries: [Entry],
        arrivedEntryIDs: Set<UUID>,
        activeSwipeEntryID: Binding<UUID?>,
        onEntryTap: @escaping (Entry) -> Void,
        swipeActionsProvider: @escaping (Entry) -> [CardSwipeAction],
        onAction: @escaping (Entry, EntryAction) -> Void,
        onGlowComplete: @escaping (UUID) -> Void
    ) {
        self.category = category
        self.entries = entries
        self.arrivedEntryIDs = arrivedEntryIDs
        self._activeSwipeEntryID = activeSwipeEntryID
        self.onEntryTap = onEntryTap
        self.swipeActionsProvider = swipeActionsProvider
        self.onAction = onAction
        self.onGlowComplete = onGlowComplete
        self._isCollapsed = AppStorage(wrappedValue: true, "section_\(category.rawValue)_collapsed")
    }

    private var color: Color { Theme.categoryColor(category) }

    var body: some View {
        VStack(spacing: 0) {
            // Section header
            Button {
                withAnimation(Animations.smoothSlide) {
                    isCollapsed.toggle()
                    // Clear peek when expanding
                    if !isCollapsed {
                        peekTask?.cancel()
                        peekVisible = false
                        peekEntry = nil
                        peekCount = 0
                    }
                }
            } label: {
                HStack(spacing: 0) {
                    HStack(spacing: 8) {
                        Circle()
                            .fill(color)
                            .frame(width: 6, height: 6)
                            .shadow(color: color.opacity(0.6), radius: 3)

                        Text(category.displayName.uppercased())
                            .font(Theme.Typography.badge)
                            .foregroundStyle(color)
                            .tracking(0.8)

                        Rectangle()
                            .fill(color.opacity(0.2))
                            .frame(height: 1)
                    }

                    Spacer()

                    HStack(spacing: 8) {
                        // Arrival count badge
                        if peekCount > 0 {
                            Text("+\(peekCount)")
                                .font(Theme.Typography.badge)
                                .foregroundStyle(color)
                                .transition(.scale.combined(with: .opacity))
                        }

                        Text("\(entries.count)")
                            .font(Theme.Typography.badge)
                            .foregroundStyle(Theme.Colors.textTertiary)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 3)
                            .background(
                                Capsule()
                                    .fill(Theme.Colors.bgCard)
                                    .overlay(Capsule().stroke(Theme.Colors.borderSubtle, lineWidth: 1))
                            )

                        Image(systemName: "chevron.down")
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundStyle(Theme.Colors.textTertiary)
                            .rotationEffect(.degrees(isCollapsed ? -90 : 0))
                            .animation(Animations.smoothSlide, value: isCollapsed)
                    }
                }
            }
            .buttonStyle(.plain)
            .padding(.horizontal, Theme.Spacing.screenPadding)
            .padding(.top, 20)
            .padding(.bottom, isCollapsed && !peekVisible ? 8 : 12)
            .shadow(color: color.opacity(0.3 * headerGlowIntensity), radius: 8)

            // Peek slot (collapsed section arrival preview)
            if isCollapsed && peekVisible, let peekEntry {
                Group {
                    if peekEntry.category == .list {
                        ListCardView(
                            entry: peekEntry,
                            onAction: onAction,
                            onTap: { onEntryTap(peekEntry) },
                            glowAccent: color,
                            glowIntensity: 1.0
                        )
                    } else {
                        SmartListRow(
                            entry: peekEntry,
                            onAction: onAction,
                            glowAccent: color,
                            glowIntensity: 1.0
                        )
                    }
                }
                .padding(.horizontal, 12)
                .transition(.opacity.combined(with: .move(edge: .top)))
                .onTapGesture {
                    // Expand section, cancel retract
                    peekTask?.cancel()
                    withAnimation(Animations.smoothSlide) {
                        isCollapsed = false
                        peekVisible = false
                        self.peekEntry = nil
                        peekCount = 0
                    }
                }
            }

            // Section body (expanded)
            if !isCollapsed {
                LazyVStack(spacing: 12) {
                    ForEach(entries) { entry in
                        if entry.category == .habit {
                            // Habits skip SwipeableCard — its UIKit overlay steals taps
                            // before the circle Button can receive them. Match Focus tab:
                            // circle Button = check-off, row tap = navigate to detail.
                            GlowingEntryRow(
                                entry: entry,
                                isArrived: arrivedEntryIDs.contains(entry.id),
                                category: category,
                                onAction: onAction,
                                onTap: nil,
                                listExpanded: nil,
                                onGlowComplete: { onGlowComplete(entry.id) }
                            )
                            .contentShape(Rectangle())
                            .onTapGesture { onEntryTap(entry) }
                            .transition(.asymmetric(
                                insertion: .opacity.combined(with: .scale(scale: 0.97)).combined(with: .offset(y: 8)),
                                removal: .opacity.combined(with: .scale(scale: 0.95))
                            ))
                        } else if entry.category == .list {
                            // Lists skip SwipeableCard — UIKit overlay blocks item check-off Buttons.
                            // ListCardView handles header nav (onTap), expand/collapse (chevron),
                            // and item check-off (row Buttons) internally — no outer tap needed.
                            GlowingEntryRow(
                                entry: entry,
                                isArrived: arrivedEntryIDs.contains(entry.id),
                                category: category,
                                onAction: onAction,
                                onTap: { onEntryTap(entry) },
                                listExpanded: Binding(
                                    get: { expandedListIDs.contains(entry.id) },
                                    set: { if $0 { expandedListIDs.insert(entry.id) } else { expandedListIDs.remove(entry.id) } }
                                ),
                                onGlowComplete: { onGlowComplete(entry.id) }
                            )
                            .transition(.asymmetric(
                                insertion: .opacity.combined(with: .scale(scale: 0.97)).combined(with: .offset(y: 8)),
                                removal: .opacity.combined(with: .scale(scale: 0.95))
                            ))
                        } else {
                            SwipeableCard(
                                actions: swipeActionsProvider(entry),
                                activeSwipeID: $activeSwipeEntryID,
                                entryID: entry.id,
                                onTap: sectionTapAction(entry)
                            ) {
                                GlowingEntryRow(
                                    entry: entry,
                                    isArrived: arrivedEntryIDs.contains(entry.id),
                                    category: category,
                                    onAction: onAction,
                                    onTap: { onEntryTap(entry) },
                                    listExpanded: nil,
                                    onGlowComplete: { onGlowComplete(entry.id) }
                                )
                            }
                            .transition(.asymmetric(
                                insertion: .opacity.combined(with: .scale(scale: 0.97)).combined(with: .offset(y: 8)),
                                removal: .opacity.combined(with: .scale(scale: 0.95))
                            ))
                        }
                    }
                }
                .padding(.horizontal, 12)
                .transition(.opacity.combined(with: .move(edge: .top)))
                .animation(Animations.cardAppear, value: entries.map(\.id))
            }
        }
        .onAppear {
            guard isCollapsed, !arrivedEntryIDs.isEmpty else { return }
            let newInSection = entries.filter { arrivedEntryIDs.contains($0.id) }
            guard let latest = newInSection.first else { return }

            peekEntry = latest
            peekCount += newInSection.count
            showPeek()
        }
        .onChange(of: arrivedEntryIDs) { oldIDs, newIDs in
            guard isCollapsed else { return }
            let added = newIDs.subtracting(oldIDs)
            let newInSection = entries.filter { added.contains($0.id) }
            guard let latest = newInSection.first else { return }

            peekEntry = latest
            peekCount += newInSection.count
            showPeek()
        }
    }

    // MARK: - Tap Routing

    private func sectionTapAction(_ entry: Entry) -> () -> Void {
        if entry.category == .habit {
            return { if entry.appliesToday { onAction(entry, .checkOffHabit) } }
        } else if entry.category == .list {
            return {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.8)) {
                    if expandedListIDs.contains(entry.id) { expandedListIDs.remove(entry.id) } else { expandedListIDs.insert(entry.id) }
                }
            }
        }
        return { onEntryTap(entry) }
    }

    // MARK: - Peek Helpers

    private func showPeek() {
        headerGlowIntensity = 1.0
        withAnimation(.easeOut(duration: 1.0)) {
            headerGlowIntensity = 0
        }

        withAnimation(Animations.cardAppear) {
            peekVisible = true
        }

        peekTask?.cancel()
        peekTask = Task {
            try? await Task.sleep(for: .seconds(3))
            guard !Task.isCancelled else { return }
            await MainActor.run {
                withAnimation(Animations.smoothSlide) {
                    peekVisible = false
                }
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                    peekEntry = nil
                    peekCount = 0
                }
            }
        }
    }
}

// MARK: - Glowing Entry Row

private struct GlowingEntryRow: View {
    let entry: Entry
    let isArrived: Bool
    let category: EntryCategory
    let onAction: (Entry, EntryAction) -> Void
    var onTap: (() -> Void)?
    var listExpanded: Binding<Bool>?
    let onGlowComplete: () -> Void

    @State private var glowIntensity: Double = 0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Group {
            if entry.category == .list {
                ListCardView(
                    entry: entry,
                    onAction: onAction,
                    onTap: onTap,
                    glowAccent: glowIntensity > 0 ? Theme.categoryColor(category) : nil,
                    glowIntensity: glowIntensity,
                    externalExpanded: listExpanded
                )
            } else {
                SmartListRow(
                    entry: entry,
                    onAction: onAction,
                    glowAccent: glowIntensity > 0 ? Theme.categoryColor(category) : nil,
                    glowIntensity: glowIntensity
                )
            }
        }
        .onChange(of: isArrived) { _, newValue in
            if newValue { triggerGlow() }
        }
        .onAppear {
            if isArrived && glowIntensity == 0 { triggerGlow() }
        }
    }

    private func triggerGlow() {
        if reduceMotion {
            onGlowComplete()
            return
        }
        glowIntensity = 1.0
        withAnimation(.easeOut(duration: 3.5)) {
            glowIntensity = 0
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 3.5) {
            onGlowComplete()
        }
    }
}

// MARK: - Smart List Row

struct SmartListRow: View {
    let entry: Entry
    let onAction: (Entry, EntryAction) -> Void
    var glowAccent: Color?
    var glowIntensity: Double = 0

    private var isOverdue: Bool { entry.isOverdue }

    private var isSnoozed: Bool { entry.status == .snoozed }

    private var snoozeText: String? {
        guard isSnoozed, let snoozeUntil = entry.snoozeUntil else { return nil }
        let calendar = Calendar.current
        let formatter = DateFormatter()
        if calendar.isDateInToday(snoozeUntil) {
            formatter.dateFormat = "h:mm a"
            return "Until \(formatter.string(from: snoozeUntil))"
        }
        if calendar.isDateInTomorrow(snoozeUntil) { return "Until tomorrow" }
        formatter.dateFormat = "MMM d"
        return "Until \(formatter.string(from: snoozeUntil))"
    }

    private var listItems: [String] {
        guard entry.category == .list else { return [] }
        return entry.content
            .components(separatedBy: "\n")
            .map { line -> String in
                var s = line.trimmingCharacters(in: .whitespacesAndNewlines)
                if s.hasPrefix("- ") { s = String(s.dropFirst(2)) } else if s.hasPrefix("• ") { s = String(s.dropFirst(2)) } else if s.hasPrefix("* ") { s = String(s.dropFirst(2)) }
                return s
            }
            .filter { !$0.isEmpty }
    }

    private var dueText: String? {
        guard entry.category == .todo || entry.category == .reminder else { return nil }
        guard let dueDate = entry.dueDate else { return nil }
        let calendar = Calendar.current
        if isOverdue { return "Overdue" }
        if calendar.isDateInToday(dueDate) { return "Due today" }
        if calendar.isDateInTomorrow(dueDate) { return "Due tomorrow" }
        let days = calendar.dateComponents([.day], from: Date(), to: dueDate).day ?? 0
        return "Due in \(days)d"
    }

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            if entry.category == .habit {
                Button {
                    guard entry.appliesToday else { return }
                    onAction(entry, .checkOffHabit)
                } label: {
                    Image(systemName: entry.isCompletedToday ? "checkmark.circle.fill" : "circle")
                        .font(.system(size: 24))
                        .foregroundStyle(Theme.categoryColor(entry.category))
                        .animation(.spring(response: 0.3, dampingFraction: 0.65), value: entry.isCompletedToday)
                        .frame(width: 36)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            } else if isSnoozed {
                let dotColor = Theme.categoryColor(entry.category)
                Image(systemName: "moon.zzz.fill")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(dotColor)
                    .frame(width: 14, height: 14)
                    .shadow(color: dotColor.opacity(0.6), radius: 4)
            } else {
                let dotColor = Theme.categoryColor(entry.category)
                Circle()
                    .fill(dotColor)
                    .frame(width: 8, height: 8)
                    .shadow(color: dotColor.opacity(0.6), radius: 4)
                    .padding(.leading, 2)
            }

            VStack(alignment: .leading, spacing: 3) {
                Text(entry.summary)
                    .font(.subheadline)
                    .foregroundStyle(entry.isDone ? Theme.Colors.textTertiary : Theme.Colors.textPrimary)
                    .lineLimit(2)

                if let snoozeText {
                    Text(snoozeText)
                        .font(.caption)
                        .foregroundStyle(Theme.Colors.textTertiary)
                } else if entry.category == .habit, let cadence = entry.cadence {
                    Text(cadence.displayName)
                        .font(.caption)
                        .foregroundStyle(Theme.Colors.textTertiary)
                } else if let dueText {
                    Text(dueText)
                        .font(.caption)
                        .foregroundStyle(isOverdue ? Theme.Colors.accentRed : Theme.Colors.textTertiary)
                }

                if !listItems.isEmpty {
                    VStack(alignment: .leading, spacing: 2) {
                        let displayItems = Array(listItems.prefix(3))
                        let remaining = listItems.count - displayItems.count
                        ForEach(displayItems, id: \.self) { item in
                            HStack(alignment: .center, spacing: 5) {
                                Circle()
                                    .fill(Theme.Colors.textMuted)
                                    .frame(width: 3, height: 3)
                                Text(item)
                                    .font(.caption2)
                                    .foregroundStyle(Theme.Colors.textSecondary)
                                    .lineLimit(1)
                            }
                        }
                        if remaining > 0 {
                            Text("+\(remaining) more")
                                .font(.caption2)
                                .foregroundStyle(Theme.Colors.textTertiary)
                                .padding(.leading, 8)
                        }
                    }
                    .padding(.top, 4)
                }
            }
            .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
        }
        .cardStyle(accent: glowAccent, intensity: glowIntensity)
        .opacity(entry.isDone ? 0.5 : (isSnoozed ? 0.7 : 1.0))
        .animation(.easeInOut(duration: 0.2), value: entry.isCompletedToday)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(entry.category.displayName): \(entry.summary)")
    }
}

// MARK: - Focus Loading (shared by home variants)

struct FocusLoadingView: View {
    @State private var isPulsing = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(Greeting.current + ".")
                .font(.title3.weight(.semibold))
                .foregroundStyle(Theme.Colors.textPrimary.opacity(0.35))
            Text("Murmur is selecting your focus…")
                .font(Theme.Typography.caption)
                .foregroundStyle(Theme.Colors.textTertiary)
                .opacity(isPulsing ? 0.45 : 0.85)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.vertical, 8)
        .onAppear {
            withAnimation(.easeInOut(duration: 1.1).repeatForever(autoreverses: true)) {
                isPulsing = true
            }
        }
    }
}

// MARK: - Processing Dots (inline streaming indicator)

struct SharedProcessingDotsView: View {
    @State private var phase: Int = 0

    var body: some View {
        HStack(spacing: 7) {
            ForEach(0..<3, id: \.self) { i in
                Circle()
                    .fill(Theme.Colors.accentPurple)
                    .frame(width: 7, height: 7)
                    .scaleEffect(phase == i ? 1.5 : 0.7)
                    .opacity(phase == i ? 1.0 : 0.25)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 16)
        .accessibilityLabel("Processing")
        .task {
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(350))
                withAnimation(.easeInOut(duration: 0.25)) {
                    phase = (phase + 1) % 3
                }
            }
        }
    }
}
