import Foundation
#if os(macOS)
import AppKit
#endif

private final class OpenResultBox: @unchecked Sendable {
    var error: Error?
}

protocol AdapterPackageInstalling {
    func installPackage(at packageURL: URL) throws -> String
}

enum AdapterPackageInstallerError: LocalizedError {
    case installerFailed(String)

    var errorDescription: String? {
        switch self {
        case .installerFailed(let message):
            return message
        }
    }
}

struct InstallerAppAdapterPackageInstaller: AdapterPackageInstalling {
    func installPackage(at packageURL: URL) throws -> String {
        #if os(macOS)
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = true

        let resultBox = OpenResultBox()
        let semaphore = DispatchSemaphore(value: 0)

        NSWorkspace.shared.open([packageURL], withApplicationAt: URL(fileURLWithPath: "/System/Library/CoreServices/Installer.app"), configuration: configuration) { _, error in
            resultBox.error = error
            semaphore.signal()
        }

        semaphore.wait()

        if let openError = resultBox.error {
            throw AdapterPackageInstallerError.installerFailed("Failed to open the armature package in Installer.app: \(openError.localizedDescription)")
        }

        return "Opened armature package in Installer.app."
        #else
        throw AdapterPackageInstallerError.installerFailed("Opening Installer.app is only supported on macOS.")
        #endif
    }
}
