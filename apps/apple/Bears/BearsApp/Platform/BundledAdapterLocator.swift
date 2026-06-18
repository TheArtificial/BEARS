import Foundation

protocol BundledAdapterLocating {
    func bundledAdapterExecutableURL() throws -> URL
}

enum BundledAdapterLocatorError: LocalizedError {
    case missingResource(checkedPaths: [String])

    var errorDescription: String? {
        switch self {
        case .missingResource(let checkedPaths):
            let details = checkedPaths.isEmpty ? "No candidate paths were checked." : checkedPaths.joined(separator: "\n")
            return "The Bears app bundle does not contain a bundled bears-acp-adapter executable. The app can fall back to downloading one if configured. Checked:\n\(details)"
        }
    }
}

struct BundledAdapterLocator: BundledAdapterLocating {
    private let resourceBundle: Bundle

    init(resourceBundle: Bundle = .main) {
        self.resourceBundle = resourceBundle
    }

    func bundledAdapterExecutableURL() throws -> URL {
        let candidates = [
            resourceBundle.url(forResource: "bears-acp-adapter", withExtension: nil),
            resourceBundle.url(forResource: "bears-acp-adapter", withExtension: nil, subdirectory: "Adapter"),
            resourceBundle.url(forResource: "bears-acp-adapter", withExtension: nil, subdirectory: "Resources/Adapter"),
            resourceBundle.resourceURL?
                .appendingPathComponent("Adapter", isDirectory: true)
                .appendingPathComponent("bears-acp-adapter", isDirectory: false),
            resourceBundle.resourceURL?
                .appendingPathComponent("Resources", isDirectory: true)
                .appendingPathComponent("Adapter", isDirectory: true)
                .appendingPathComponent("bears-acp-adapter", isDirectory: false),
            Bundle.main.bundleURL
                .appendingPathComponent("Contents", isDirectory: true)
                .appendingPathComponent("Resources", isDirectory: true)
                .appendingPathComponent("Adapter", isDirectory: true)
                .appendingPathComponent("bears-acp-adapter", isDirectory: false)
        ]

        for case let url? in candidates where FileManager.default.fileExists(atPath: url.path) {
            return url
        }

        let checkedPaths = candidates.map { $0?.path ?? "<nil>" }
        throw BundledAdapterLocatorError.missingResource(checkedPaths: checkedPaths)
    }
}
