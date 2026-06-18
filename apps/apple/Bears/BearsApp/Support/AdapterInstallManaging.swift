import Foundation

enum AdapterInstallUpdateResult {
    case installerOpened(message: String)
    case installed(InstallState)
}

struct InstalledAdapterSnapshot {
    let version: String?
    let exists: Bool
    let fileSize: UInt64?
    let modificationDate: Date?
}

protocol AdapterInstallManaging {
    func loadInstallState() throws -> InstallState?
    func inspectInstallState() throws -> InstallState
    func updateInstall() throws -> AdapterInstallUpdateResult
    func installedAdapterSnapshot() -> InstalledAdapterSnapshot
}

protocol AdapterVersionProviding {
    func bundledAdapterVersion() throws -> AdapterVersionInfo
    func referenceAdapterVersion() throws -> AdapterVersionInfo
    func installedAdapterVersion() throws -> AdapterVersionInfo
}

protocol AdapterPathProviding {
    var applicationSupportRoot: URL { get }
    var managedAdapterPath: URL { get }
    var installStatePath: URL { get }
    var acpLogsDirectory: URL { get }
}
