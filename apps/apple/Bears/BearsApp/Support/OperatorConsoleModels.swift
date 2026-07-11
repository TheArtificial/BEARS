import Foundation

enum OperatorConsoleSeverity: Int, Comparable {
    case attention = 0
    case active = 1
    case neutral = 2
    case success = 3
    case unavailable = 4

    static func < (lhs: OperatorConsoleSeverity, rhs: OperatorConsoleSeverity) -> Bool {
        lhs.rawValue < rhs.rawValue
    }

    var label: String {
        switch self {
        case .attention: "Needs attention"
        case .active: "Active"
        case .neutral: "Ready"
        case .success: "OK"
        case .unavailable: "Unavailable"
        }
    }
}

struct OperatorConsoleCard: Identifiable, Equatable {
    let id: String
    let title: String
    let status: String
    let detail: String
    let metadata: [String]
    let lastUpdated: Date?
    let severity: OperatorConsoleSeverity

    init(
        id: String,
        title: String,
        status: String,
        detail: String,
        metadata: [String] = [],
        lastUpdated: Date? = nil,
        severity: OperatorConsoleSeverity
    ) {
        self.id = id
        self.title = title
        self.status = status
        self.detail = detail
        self.metadata = metadata
        self.lastUpdated = lastUpdated
        self.severity = severity
    }
}

struct OperatorConsoleSnapshot: Equatable {
    var activeWork: [OperatorConsoleCard]
    var needsAttention: [OperatorConsoleCard]
    var recentRuns: [OperatorConsoleCard]
    var capabilities: [OperatorConsoleCard]
    var generatedAt: Date

    static let empty = OperatorConsoleSnapshot(
        activeWork: [],
        needsAttention: [],
        recentRuns: [],
        capabilities: [],
        generatedAt: Date(timeIntervalSince1970: 0)
    )

    var hasAnyContent: Bool {
        !(activeWork.isEmpty && needsAttention.isEmpty && recentRuns.isEmpty && capabilities.isEmpty)
    }
}

enum OperatorConsoleOrdering {
    static func snapshot(
        status: InstallStatus,
        statusText: String,
        latestVersion: String,
        installedVersion: String,
        isAwaitingInstallerCompletion: Bool,
        visibleError: String?,
        generatedAt: Date = Date()
    ) -> OperatorConsoleSnapshot {
        let severity: OperatorConsoleSeverity = switch status {
        case .ok: .success
        case .missing, .repairNeeded, .error: .attention
        }

        let armatureCard = OperatorConsoleCard(
            id: "armature-install",
            title: "Armature install",
            status: isAwaitingInstallerCompletion ? "Installer open" : statusText,
            detail: visibleError ?? "Managed local armature used by editor sessions.",
            metadata: ["installed: \(installedVersion)", "latest: \(latestVersion)"],
            lastUpdated: generatedAt,
            severity: isAwaitingInstallerCompletion ? .active : severity
        )

        let capabilityCard = OperatorConsoleCard(
            id: "pairing-capability",
            title: "Editor pairing",
            status: status == .ok ? "Ready" : "Blocked",
            detail: status == .ok ? "The managed armature is ready for local workspace sessions." : "Update or repair the armature before starting local pairing.",
            metadata: [],
            lastUpdated: generatedAt,
            severity: status == .ok ? .success : .attention
        )

        return OperatorConsoleSnapshot(
            activeWork: isAwaitingInstallerCompletion ? [armatureCard] : [],
            needsAttention: status == .ok || isAwaitingInstallerCompletion ? [] : [armatureCard],
            recentRuns: status == .ok ? [armatureCard] : [],
            capabilities: [capabilityCard],
            generatedAt: generatedAt
        )
    }

    static func sortedForOperator(_ cards: [OperatorConsoleCard]) -> [OperatorConsoleCard] {
        cards.sorted { lhs, rhs in
            if lhs.severity != rhs.severity {
                return lhs.severity < rhs.severity
            }

            let lhsDate = lhs.lastUpdated ?? Date.distantPast
            let rhsDate = rhs.lastUpdated ?? Date.distantPast
            if lhsDate != rhsDate {
                return lhsDate > rhsDate
            }

            return lhs.title.localizedCaseInsensitiveCompare(rhs.title) == .orderedAscending
        }
    }

    static func cardsNeedingAttention(from snapshot: OperatorConsoleSnapshot) -> [OperatorConsoleCard] {
        sortedForOperator(
            snapshot.activeWork + snapshot.needsAttention + snapshot.recentRuns + snapshot.capabilities
        ).filter { $0.severity == .attention }
    }
}
