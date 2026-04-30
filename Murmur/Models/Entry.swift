import Foundation
import SwiftData
import MurmurCore
import os.log

private let entryLog = Logger(subsystem: Bundle.main.bundleIdentifier ?? "murmur", category: "Entries")

/// The atomic unit — every voice input is interpreted, categorized, and stored as an Entry.
@Model
public final class Entry {
    @Attribute(.unique) public var id: UUID

    /// Original voice-to-text transcription (full recording)
    public var transcript: String

    /// AI-structured version of the transcript (cleaned/formatted)
    public var content: String

    /// Stored as raw string for SwiftData predicate support
    public var categoryRawValue: String

    /// AI-assigned category
    public var category: EntryCategory {
        get { EntryCategory(from: categoryRawValue) }
        set { categoryRawValue = newValue.rawValue }
    }

    /// The specific part of the transcript this entry was extracted from
    public var sourceText: String

    /// When the entry was captured
    public var createdAt: Date

    /// When the entry was last modified
    public var updatedAt: Date

    // MARK: - LLM-populated fields

    /// One-liner summary for cards/lists
    public var summary: String

    /// User-added supplementary notes
    public var notes: String = ""

    /// Priority 1-5 scale (1 = highest)
    public var priority: Int?

    /// Raw time phrase extracted by LLM (e.g. "next Thursday", "in 2 hours")
    public var dueDateDescription: String?

    /// Resolved date from dueDateDescription (resolved on-device)
    public var dueDate: Date?

    /// Raw storage for HabitCadence (SwiftData column — lightweight migration)
    public var cadenceRawValue: String?

    /// How often this habit repeats
    public var cadence: HabitCadence? {
        get { cadenceRawValue.flatMap { HabitCadence(rawValue: $0) } }
        set { cadenceRawValue = newValue?.rawValue }
    }

    // MARK: - Status (app-managed, not LLM)

    /// Stored as raw string for SwiftData predicate support
    public var statusRawValue: String

    /// Entry lifecycle status
    public var status: EntryStatus {
        get { EntryStatus(from: statusRawValue) }
        set { statusRawValue = newValue.rawValue }
    }

    /// When the entry was marked completed
    public var completedAt: Date?

    /// When a snoozed entry should resurface
    public var snoozeUntil: Date?

    /// When this habit was last checked off (used to determine isDoneForPeriod)
    public var lastHabitCompletionDate: Date?

    /// Full history of habit check-off dates (used to compute streaks)
    public var habitCompletionDates: [Date] = []

    // MARK: - Source metadata

    /// Recording length in seconds
    public var audioDuration: TimeInterval?

    /// Stored as raw string for SwiftData predicate support
    public var sourceRawValue: String

    /// How the entry was captured
    public var source: EntrySource {
        get { EntrySource(from: sourceRawValue) }
        set { sourceRawValue = newValue.rawValue }
    }

    /// Resolve a date string to a Date. Tries ISO 8601 first (LLM output), then NSDataDetector fallback.
    public static func resolveDate(from phrase: String?) -> Date? {
        guard let phrase, !phrase.isEmpty else { return nil }

        // Primary: ISO 8601 (what the LLM now outputs for relative + absolute times)
        let iso = ISO8601DateFormatter()
        iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = iso.date(from: phrase) { return date }
        // Also try without fractional seconds
        iso.formatOptions = [.withInternetDateTime]
        if let date = iso.date(from: phrase) { return date }
        // Try without timezone (bare datetime like "2025-03-17T15:30:00")
        let bare = DateFormatter()
        bare.dateFormat = "yyyy-MM-dd'T'HH:mm:ss"
        bare.locale = Locale(identifier: "en_US_POSIX")
        if let date = bare.date(from: phrase) { return date }

        // Fallback: NSDataDetector for legacy verbatim phrases ("next Thursday", "tomorrow at 3pm")
        guard let detector = try? NSDataDetector(types: NSTextCheckingResult.CheckingType.date.rawValue) else {
            return nil
        }
        let range = NSRange(phrase.startIndex..., in: phrase)
        return detector.matches(in: phrase, options: [], range: range).first?.date
    }

    /// Bridge initializer: convert an ExtractedEntry (from MurmurCore) into a persistable Entry.
    public convenience init(
        from extracted: ExtractedEntry,
        transcript: String,
        source: EntrySource,
        audioDuration: TimeInterval?
    ) {
        self.init(
            transcript: transcript,
            content: extracted.content,
            category: extracted.category,
            sourceText: extracted.sourceText,
            summary: extracted.summary,
            priority: extracted.priority,
            dueDateDescription: extracted.dueDateDescription,
            dueDate: Entry.resolveDate(from: extracted.dueDateDescription),
            cadenceRawValue: extracted.cadence?.rawValue,
            audioDuration: audioDuration,
            source: source
        )
    }

    public init(
        id: UUID = UUID(),
        transcript: String,
        content: String,
        category: EntryCategory,
        sourceText: String,
        createdAt: Date = Date(),
        updatedAt: Date = Date(),
        summary: String = "",
        priority: Int? = nil,
        dueDateDescription: String? = nil,
        dueDate: Date? = nil,
        cadenceRawValue: String? = nil,
        status: EntryStatus = .active,
        completedAt: Date? = nil,
        snoozeUntil: Date? = nil,
        lastHabitCompletionDate: Date? = nil,
        audioDuration: TimeInterval? = nil,
        source: EntrySource = .voice
    ) {
        self.id = id
        self.transcript = transcript
        self.content = content
        self.categoryRawValue = category.rawValue
        self.sourceText = sourceText
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.summary = summary
        self.priority = priority
        self.dueDateDescription = dueDateDescription
        self.dueDate = dueDate
        self.cadenceRawValue = cadenceRawValue
        self.statusRawValue = status.rawValue
        self.completedAt = completedAt
        self.snoozeUntil = snoozeUntil
        self.lastHabitCompletionDate = lastHabitCompletionDate
        self.audioDuration = audioDuration
        self.sourceRawValue = source.rawValue
    }
}

/// Entry lifecycle status
public enum EntryStatus: String, Codable, Sendable, CaseIterable {
    case active
    case completed
    case archived
    case snoozed

    public var displayName: String {
        switch self {
        case .active: return "Active"
        case .completed: return "Completed"
        case .archived: return "Archived"
        case .snoozed: return "Snoozed"
        }
    }

    /// Defensive initializer — falls back to .active for unknown raw values
    public init(from rawValue: String) {
        self = EntryStatus(rawValue: rawValue) ?? .active
    }
}

// MARK: - Agent Context Bridge

extension Entry {
    /// Short ID for LLM context — first 6 chars of UUID, lowercased.
    var shortID: String {
        String(id.uuidString.lowercased().prefix(6))
    }

    /// Convert to the compact snapshot format used in LLM context.
    func toAgentContext() -> AgentContextEntry {
        let agentStatus: AgentEntryStatus = switch status {
        case .active: .active
        case .completed: .completed
        case .archived: .archived
        case .snoozed: .snoozed
        }

        let streak = category == .habit && currentStreak > 0 ? currentStreak : nil

        // Format resolved dueDate as absolute string so the LLM sees "Mar 8" not "tomorrow".
        // Falls back to raw dueDateDescription if dueDate was never resolved.
        let formattedDue: String? = if let dueDate {
            Self.formatDueDateForContext(dueDate)
        } else {
            dueDateDescription
        }

        return AgentContextEntry(
            id: shortID,
            summary: summary,
            category: category,
            priority: priority,
            dueDateDescription: formattedDue,
            cadence: cadence,
            status: agentStatus,
            createdAt: createdAt,
            currentStreak: streak,
            notes: notes.isEmpty ? nil : notes
        )
    }

    /// Format a due date as a compact absolute string for LLM context.
    private static let shortDateFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "MMM d"
        return f
    }()

    private static func formatDueDateForContext(_ date: Date) -> String {
        let calendar = Calendar.current
        if calendar.isDateInYesterday(date) { return "yesterday" }
        if calendar.isDateInToday(date) { return "today" }
        if calendar.isDateInTomorrow(date) { return "tomorrow" }
        return shortDateFormatter.string(from: date)
    }

    /// Resolve a short ID prefix back to an Entry from a list.
    /// Returns nil if zero or 2+ entries match (ambiguous).
    static func resolve(shortID: String, in entries: [Entry]) -> Entry? {
        let matches = entries.filter {
            $0.id.uuidString.lowercased().hasPrefix(shortID.lowercased())
        }
        return matches.count == 1 ? matches[0] : nil
    }
}

// MARK: - Habit Period Tracking

extension Entry {
    /// True if this entry is past due and still active.
    public var isOverdue: Bool {
        guard let dueDate else { return false }
        return dueDate < Date() && status == .active
    }

    /// True if this habit is done for its current period, or was completed today.
    public var isDone: Bool {
        isDoneForPeriod || isCompletedToday
    }

    /// True if this habit was checked off today (regardless of cadence).
    public var isCompletedToday: Bool {
        guard let lastCompleted = lastHabitCompletionDate else { return false }
        return Calendar.current.isDateInToday(lastCompleted)
    }

    /// True if this habit's cadence applies on today's day of week.
    /// e.g. weekday-only habits return false on Saturday/Sunday.
    public var appliesToday: Bool {
        guard category == .habit else { return true }
        switch cadence ?? .daily {
        case .daily, .weekly, .monthly:
            return true
        case .weekdays:
            let weekday = Calendar.current.component(.weekday, from: Date())
            return weekday != 1 && weekday != 7
        }
    }

    /// True if this habit has been checked off for the current cadence period.
    public var isDoneForPeriod: Bool {
        guard category == .habit, let lastCompleted = lastHabitCompletionDate else { return false }
        let calendar = Calendar.current
        let now = Date()
        switch cadence ?? .daily {
        case .daily:
            return calendar.isDateInToday(lastCompleted)
        case .weekdays:
            let weekday = calendar.component(.weekday, from: now)
            guard weekday != 1 && weekday != 7 else { return false }
            return calendar.isDateInToday(lastCompleted)
        case .weekly:
            return calendar.isDate(lastCompleted, equalTo: now, toGranularity: .weekOfYear)
        case .monthly:
            return calendar.isDate(lastCompleted, equalTo: now, toGranularity: .month)
        }
    }
}

// MARK: - Habit Streaks

extension Entry {
    /// Number of consecutive periods this habit has been completed up to and including the current period.
    /// Returns 0 if the streak is broken (a period was skipped).
    public var currentStreak: Int {
        guard category == .habit else { return 0 }
        return computeStreaks().current
    }

    /// Longest consecutive run ever recorded.
    public var longestStreak: Int {
        guard category == .habit else { return 0 }
        return computeStreaks().longest
    }

    private func computeStreaks() -> (current: Int, longest: Int) {
        guard !habitCompletionDates.isEmpty else { return (0, 0) }
        let calendar = Calendar.current
        let c = cadence ?? .daily

        // Normalize each completion to its period boundary, deduplicate, sort descending
        let periods = Array(
            Set(habitCompletionDates.map { periodStart(for: $0, cadence: c, calendar: calendar) })
        ).sorted(by: >)

        guard !periods.isEmpty else { return (0, 0) }

        // Group into consecutive runs
        var runs: [[Date]] = []
        var run = [periods[0]]
        for i in 1..<periods.count {
            let expected = prevPeriodStart(before: periods[i - 1], cadence: c, calendar: calendar)
            if periods[i] == expected {
                run.append(periods[i])
            } else {
                runs.append(run)
                run = [periods[i]]
            }
        }
        runs.append(run)

        let longest = runs.map(\.count).max() ?? 0

        // Current streak is alive if the latest period is today or the previous applicable period
        let today = periodStart(for: Date(), cadence: c, calendar: calendar)
        let prev = prevPeriodStart(before: today, cadence: c, calendar: calendar)
        let latestPeriod = runs[0][0]
        let current = (latestPeriod == today || latestPeriod == prev) ? runs[0].count : 0

        return (current, longest)
    }

    private func periodStart(for date: Date, cadence: HabitCadence, calendar: Calendar) -> Date {
        switch cadence {
        case .daily, .weekdays:
            return calendar.startOfDay(for: date)
        case .weekly:
            return calendar.dateInterval(of: .weekOfYear, for: date)?.start ?? calendar.startOfDay(for: date)
        case .monthly:
            return calendar.dateInterval(of: .month, for: date)?.start ?? calendar.startOfDay(for: date)
        }
    }

    private func prevPeriodStart(before date: Date, cadence: HabitCadence, calendar: Calendar) -> Date {
        let period = periodStart(for: date, cadence: cadence, calendar: calendar)
        switch cadence {
        case .daily:
            return calendar.date(byAdding: .day, value: -1, to: period) ?? period
        case .weekdays:
            // Monday → Friday (skip weekend)
            let weekday = calendar.component(.weekday, from: period)
            let daysBack = weekday == 2 ? 3 : 1
            return calendar.date(byAdding: .day, value: -daysBack, to: period) ?? period
        case .weekly:
            return calendar.date(byAdding: .weekOfYear, value: -1, to: period) ?? period
        case .monthly:
            return calendar.date(byAdding: .month, value: -1, to: period) ?? period
        }
    }
}

// MARK: - Entry Actions

enum EntryAction {
    case snooze(until: Date?)  // nil = default 1 hour
    case wake                  // wake a snoozed entry: set status = .active, clear snoozeUntil
    case complete
    case archive
    case unarchive
    case delete
    case checkOffHabit         // toggle done-for-period on habit entries
    case toggleListItem(index: Int) // toggle checkbox on a list item
}

extension Entry {
    /// Single source of truth for all entry status mutations.
    func perform(_ action: EntryAction, in context: ModelContext, preferences: NotificationPreferences) {
        switch action {
        case .snooze(let until):
            let target = until ?? Calendar.current.date(byAdding: .hour, value: 1, to: Date())
            snoozeUntil = target
            status = .snoozed
            updatedAt = Date()
            save(in: context)
            NotificationService.shared.sync(self, preferences: preferences)

        case .wake:
            status = .active
            snoozeUntil = nil
            updatedAt = Date()
            save(in: context)
            NotificationService.shared.cancel(self)

        case .complete:
            status = .completed
            completedAt = Date()
            updatedAt = Date()
            save(in: context)
            NotificationService.shared.cancel(self)

        case .archive:
            status = .archived
            updatedAt = Date()
            save(in: context)
            NotificationService.shared.cancel(self)

        case .unarchive:
            status = .active
            updatedAt = Date()
            save(in: context)
            NotificationService.shared.sync(self, preferences: preferences)

        case .delete:
            NotificationService.shared.cancel(self)
            context.delete(self)
            save(in: context)

        case .checkOffHabit:
            if isCompletedToday {
                habitCompletionDates.removeAll { Calendar.current.isDateInToday($0) }
                lastHabitCompletionDate = nil
            } else {
                let now = Date()
                habitCompletionDates.append(now)
                lastHabitCompletionDate = now
            }
            updatedAt = Date()
            save(in: context)

        case .toggleListItem(let index):
            toggleListItem(at: index)
            save(in: context)
        }
    }

    /// Toggle a checkbox item in list content by its index among list items.
    private func toggleListItem(at index: Int) {
        var lines = content.components(separatedBy: "\n")
        var itemIndex = 0
        for i in lines.indices {
            let trimmed = lines[i].trimmingCharacters(in: .whitespaces)
            let isListItem = trimmed.hasPrefix("- [x] ")
                || trimmed.hasPrefix("- [ ] ")
                || trimmed.hasPrefix("- ")
            guard isListItem else { continue }
            if itemIndex == index {
                let leading = String(lines[i].prefix(while: { $0 == " " || $0 == "\t" }))
                if trimmed.hasPrefix("- [x] ") {
                    lines[i] = leading + "- [ ] " + String(trimmed.dropFirst(6))
                } else if trimmed.hasPrefix("- [ ] ") {
                    lines[i] = leading + "- [x] " + String(trimmed.dropFirst(6))
                } else if trimmed.hasPrefix("- ") {
                    lines[i] = leading + "- [x] " + String(trimmed.dropFirst(2))
                }
                break
            }
            itemIndex += 1
        }
        content = lines.joined(separator: "\n")
        updatedAt = Date()
    }

    private func save(in context: ModelContext) {
        do {
            try context.save()
        } catch {
            entryLog.error("Failed to save entry: \(error.localizedDescription)")
        }
    }
}
