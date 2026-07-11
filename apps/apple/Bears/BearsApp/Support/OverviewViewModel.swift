import Foundation
#if os(macOS)
import AppKit
#endif

@MainActor
final class OverviewViewModel: ObservableObject {
    private var hasRefreshedOnce = false
    private var installMonitorTask: Task<Void, Never>?
    private var directoryObserver: ManagedAdapterDirectoryObserver?
    private let installMonitorFastPollSeconds: UInt64 = 1
    private let installMonitorSlowPollSeconds: UInt64 = 3
    private let installMonitorFastPhaseDuration: TimeInterval = 60
    private let installMonitorTimeout: TimeInterval = 600

    @Published private(set) var installState: InstallState?
    @Published private(set) var managedAdapterPath: String
    @Published private(set) var latestVersion: String = "Unavailable"
    @Published private(set) var installedVersion: String = "Unavailable"
    @Published private(set) var latestVersionDetails: String = "Unavailable"
    @Published private(set) var installedVersionDetails: String = "Unavailable"
    @Published private(set) var statusText: String = "Not checked"
    @Published private(set) var canUpdate = false
    @Published private(set) var lastError: String?
    @Published private(set) var actionMessage: String?
    @Published private(set) var isAwaitingInstallerCompletion = false
    @Published private(set) var statusCopied = false
    @Published private(set) var latestVersionCopied = false
    @Published private(set) var installedVersionCopied = false
    @Published private(set) var operatorConsoleSnapshot = OperatorConsoleSnapshot.empty

    private let installManager: AdapterInstallManager
    private let pathProvider: BearsPathResolver

    init(
        installManager: AdapterInstallManager = AdapterInstallManager(),
        pathProvider: BearsPathResolver = BearsPathResolver()
    ) {
        self.installManager = installManager
        self.pathProvider = pathProvider
        self.managedAdapterPath = pathProvider.managedAdapterPath.path
    }

    func refresh() {
        guard !hasRefreshedOnce else {
            refreshManifestAndState()
            return
        }

        hasRefreshedOnce = true
        refreshManifestAndState()
    }

    func refreshManifestAndState() {
        do {
            let manifestVersionResult = installManager.currentManifestVersion()
            let installedInfoResult = Result { try installManager.installedAdapterVersion() }
            let installedInfo = try? installedInfoResult.get()
            let state = try installManager.inspectInstallState()
            installState = state
            latestVersion = Self.manifestVersionDisplay(from: manifestVersionResult)
            installedVersion = state.installedVersion ?? installedInfo?.version ?? "Unavailable"
            latestVersionDetails = Self.manifestVersionDetails(from: manifestVersionResult)
            installedVersionDetails = Self.versionDetails(from: installedInfo)
            statusText = Self.statusText(for: state.lastInstallStatus)
            canUpdate = state.lastInstallStatus == .repairNeeded
            let combinedError = Self.combinedError(
                primary: state.lastError,
                referenceVersionError: Self.errorDescription(from: manifestVersionResult, prefix: "Latest version read failed"),
                installedVersionError: Self.errorDescription(from: installedInfoResult, prefix: "Installed version read failed")
            )
            let visibleError = installedInfo != nil ? nil : Self.shortVisibleError(from: combinedError)
            if visibleError != nil && !isAwaitingInstallerCompletion {
                actionMessage = nil
            }
            lastError = visibleError
            if let combinedError, lastError != nil {
                fputs("[Bears][OverviewViewModel][refresh][visibleError] \(combinedError)\n", stderr)
            }
            operatorConsoleSnapshot = OperatorConsoleOrdering.snapshot(
                status: state.lastInstallStatus,
                statusText: statusText,
                latestVersion: latestVersion,
                installedVersion: installedVersion,
                isAwaitingInstallerCompletion: isAwaitingInstallerCompletion,
                visibleError: visibleError
            )
        } catch {
            statusText = "Error"
            canUpdate = false
            actionMessage = nil
            lastError = error.localizedDescription
            latestVersion = "Unavailable"
            latestVersionDetails = "Unavailable"
            installedVersionDetails = "Unavailable"
            operatorConsoleSnapshot = OperatorConsoleOrdering.snapshot(
                status: .error,
                statusText: statusText,
                latestVersion: latestVersion,
                installedVersion: installedVersion,
                isAwaitingInstallerCompletion: false,
                visibleError: lastError
            )
            fputs("[Bears][refresh] \(error.localizedDescription)\n", stderr)
        }
    }

    func updateInstall() {
        let baselineSnapshot = installManager.installedAdapterSnapshot()

        do {
            let result = try installManager.updateInstall()
            switch result {
            case .installed:
                installMonitorTask?.cancel()
                stopDirectoryObservation()
                isAwaitingInstallerCompletion = false
                actionMessage = "Installed version updated."
                lastError = nil
                refreshManifestAndState()
            case .installerOpened(let message):
                actionMessage = "\(message) Complete installation there. Bears will refresh the installed version automatically."
                lastError = nil
                isAwaitingInstallerCompletion = true
                beginInstallMonitoring(baselineSnapshot: baselineSnapshot)
            }
        } catch {
            installMonitorTask?.cancel()
            stopDirectoryObservation()
            isAwaitingInstallerCompletion = false
            statusText = "Error"
            canUpdate = false
            actionMessage = nil
            lastError = error.localizedDescription
            latestVersion = "Unavailable"
            latestVersionDetails = "Unavailable"
            installedVersionDetails = "Unavailable"
            operatorConsoleSnapshot = OperatorConsoleOrdering.snapshot(
                status: .error,
                statusText: statusText,
                latestVersion: latestVersion,
                installedVersion: installedVersion,
                isAwaitingInstallerCompletion: false,
                visibleError: lastError
            )
            fputs("[Bears][updateInstall] \(error.localizedDescription)\n", stderr)
        }
    }

    deinit {
        installMonitorTask?.cancel()
    }

    private func beginInstallMonitoring(baselineSnapshot: InstalledAdapterSnapshot) {
        installMonitorTask?.cancel()
        stopDirectoryObservation()
        startDirectoryObservation(baselineSnapshot: baselineSnapshot)
        installMonitorTask = Task { @MainActor [weak self] in
            await self?.monitorInstalledVersionChange(baselineSnapshot: baselineSnapshot)
        }
    }

    private func monitorInstalledVersionChange(baselineSnapshot: InstalledAdapterSnapshot) async {
        let start = Date()

        while !Task.isCancelled, Date().timeIntervalSince(start) < installMonitorTimeout {
            refreshManifestAndState()

            let currentSnapshot = installManager.installedAdapterSnapshot()
            if finishInstallMonitoringIfCompleted(previous: baselineSnapshot, current: currentSnapshot) {
                return
            }

            actionMessage = "Waiting for installation to complete…"

            let elapsed = Date().timeIntervalSince(start)
            let sleepSeconds = elapsed < installMonitorFastPhaseDuration ? installMonitorFastPollSeconds : installMonitorSlowPollSeconds
            do {
                try await Task.sleep(nanoseconds: sleepSeconds * 1_000_000_000)
            } catch {
                break
            }
        }

        refreshManifestAndState()
        actionMessage = "Still waiting for installation to complete. You can click Refresh if needed."
        isAwaitingInstallerCompletion = false
        stopDirectoryObservation()
        installMonitorTask = nil
    }

    private func startDirectoryObservation(baselineSnapshot: InstalledAdapterSnapshot) {
        let observer = ManagedAdapterDirectoryObserver(directoryURL: pathProvider.adapterDirectory)
        observer.start { [weak self] in
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.refreshManifestAndState()
                let currentSnapshot = self.installManager.installedAdapterSnapshot()
                _ = self.finishInstallMonitoringIfCompleted(previous: baselineSnapshot, current: currentSnapshot)
            }
        }
        directoryObserver = observer
    }

    private func stopDirectoryObservation() {
        directoryObserver?.stop()
        directoryObserver = nil
    }

    private func finishInstallMonitoringIfCompleted(previous: InstalledAdapterSnapshot, current: InstalledAdapterSnapshot) -> Bool {
        guard Self.hasObservedInstallCompletion(previous: previous, current: current) else {
            return false
        }

        let message: String
        if previous.version != current.version, let version = current.version {
            message = "Installed version updated to \(version)."
        } else if let version = current.version {
            message = "Armature reinstalled. Installed version remains \(version)."
        } else {
            message = "Installation detected."
        }

        actionMessage = message
        isAwaitingInstallerCompletion = false
        stopDirectoryObservation()
        installMonitorTask = nil
        return true
    }

    private static func hasObservedInstallCompletion(previous: InstalledAdapterSnapshot, current: InstalledAdapterSnapshot) -> Bool {
        if !previous.exists && current.exists {
            return true
        }
        if previous.version != current.version, current.version != nil {
            return true
        }
        if previous.fileSize != current.fileSize, current.fileSize != nil {
            return true
        }
        if previous.modificationDate != current.modificationDate, current.modificationDate != nil {
            return true
        }
        return false
    }

    private static func versionDetails(from info: AdapterVersionInfo?) -> String {
        guard let info else {
            return "Unavailable"
        }

        return [
            "version=\(info.version)",
            "buildGitSha=\(info.buildGitSha)",
            "localHeadSha=\(info.localHeadSha)",
            "builtAtUTC=\(info.builtAtUtc)",
            "chromeTools=\(info.chromeTools)",
            "directTools=\(info.directTools?.count ?? 0) entries"
        ].joined(separator: "\n")
    }

    private static func manifestVersionDisplay(from result: Result<String?, Error>) -> String {
        switch result {
        case .success(let version):
            return version ?? "Unavailable"
        case .failure(let error as GitHubReleaseAdapterSourceError):
            switch error {
            case .manifestNotFound:
                return "Not Found"
            case .manifestUnavailable, .invalidManifestJSON:
                return "Error"
            default:
                return "Unavailable"
            }
        case .failure:
            return "Error"
        }
    }

    private static func manifestVersionDetails(from result: Result<String?, Error>) -> String {
        switch result {
        case .success(let version):
            return version.map { "version=\($0)" } ?? "Latest version unavailable from manifest"
        case .failure(let error):
            return "Latest version unavailable: \(error.localizedDescription)"
        }
    }

    private static func errorDescription<T>(from result: Result<T, Error>, prefix: String) -> String? {
        switch result {
        case .success:
            return nil
        case .failure(let error):
            return "\(prefix): \(error.localizedDescription)"
        }
    }

    func versionDetails(forInstalledVersion: Bool) -> String {
        forInstalledVersion ? installedVersionDetails : latestVersionDetails
    }

    func copyManagedAdapterPath() {
        #if os(macOS)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(managedAdapterPath, forType: .string)
        #endif

        statusCopied = true
        Task {
            try? await Task.sleep(nanoseconds: 1_000_000_000)
            statusCopied = false
        }
    }

    func copyVersionDetails(forInstalledVersion: Bool) {
        #if os(macOS)
        let details = versionDetails(forInstalledVersion: forInstalledVersion)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(details, forType: .string)
        #endif

        if forInstalledVersion {
            installedVersionCopied = true
        } else {
            latestVersionCopied = true
        }

        Task {
            try? await Task.sleep(nanoseconds: 1_000_000_000)
            if forInstalledVersion {
                installedVersionCopied = false
            } else {
                latestVersionCopied = false
            }
        }
    }

    private static func combinedError(primary: String?, referenceVersionError: String?, installedVersionError: String?) -> String? {
        let parts = [primary, referenceVersionError, installedVersionError].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: "\n")
    }

    private static func shortVisibleError(from error: String?) -> String? {
        guard let error, !error.isEmpty else {
            return nil
        }

        let firstLine = error
            .split(separator: "\n", omittingEmptySubsequences: false)
            .first
            .map(String.init)?
            .trimmingCharacters(in: .whitespacesAndNewlines)

        guard let firstLine, !firstLine.isEmpty else {
            return "Error details available"
        }

        return firstLine.count > 160 ? String(firstLine.prefix(157)) + "..." : firstLine
    }

    private static func statusText(for status: InstallStatus) -> String {
        switch status {
        case .ok:
            return "Up to Date"
        case .missing:
            return "Not Installed"
        case .repairNeeded:
            return "Needs Update"
        case .error:
            return "Needs Update"
        }
    }
}
