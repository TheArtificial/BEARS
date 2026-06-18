import Foundation
import Dispatch

final class ManagedAdapterDirectoryObserver {
    private let directoryURL: URL
    private let queue = DispatchQueue(label: "ai.bears.managed-adapter-directory-observer")
    private var fileDescriptor: Int32 = -1
    private var source: DispatchSourceFileSystemObject?

    init(directoryURL: URL) {
        self.directoryURL = directoryURL
    }

    func start(onChange: @escaping @Sendable () -> Void) {
        stop()

        fileDescriptor = open(directoryURL.path, O_EVTONLY)
        guard fileDescriptor >= 0 else {
            return
        }

        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: fileDescriptor,
            eventMask: [.write, .delete, .rename, .attrib, .extend, .link, .revoke],
            queue: queue
        )

        source.setEventHandler(handler: onChange)
        source.setCancelHandler { [fileDescriptor] in
            if fileDescriptor >= 0 {
                close(fileDescriptor)
            }
        }
        self.source = source
        source.resume()
    }

    func stop() {
        source?.cancel()
        source = nil
        fileDescriptor = -1
    }

    deinit {
        stop()
    }
}
