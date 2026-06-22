import Foundation

struct AdapterVersionInfo: Codable, Equatable {
    let name: String
    let version: String
    let buildGitSha: String
    let builtAtUtc: String
    let localHeadSha: String
    let supportsSessionList: Bool
    let supportsSessionResume: Bool
    let supportsSessionLoad: Bool
    let directTools: [String: JSONValue]?
    let chromeTools: String

    private enum CodingKeys: String, CodingKey {
        case name
        case version
        case buildGitSha
        case builtAtUtc
        case localHeadSha
        case supportsSessionList
        case supportsSessionResume
        case supportsSessionLoad
        case directTools
        case chromeTools
    }

    init(
        name: String,
        version: String,
        buildGitSha: String,
        builtAtUtc: String,
        localHeadSha: String,
        supportsSessionList: Bool,
        supportsSessionResume: Bool,
        supportsSessionLoad: Bool,
        directTools: [String: JSONValue]?,
        chromeTools: String
    ) {
        self.name = name
        self.version = version
        self.buildGitSha = buildGitSha
        self.builtAtUtc = builtAtUtc
        self.localHeadSha = localHeadSha
        self.supportsSessionList = supportsSessionList
        self.supportsSessionResume = supportsSessionResume
        self.supportsSessionLoad = supportsSessionLoad
        self.directTools = directTools
        self.chromeTools = chromeTools
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        name = try container.decodeIfPresent(String.self, forKey: .name) ?? "bear-armature"
        version = try container.decode(String.self, forKey: .version)
        buildGitSha = try container.decodeIfPresent(String.self, forKey: .buildGitSha) ?? "unknown"
        builtAtUtc = try container.decodeIfPresent(String.self, forKey: .builtAtUtc) ?? "unknown"
        localHeadSha = try container.decodeIfPresent(String.self, forKey: .localHeadSha) ?? "unknown"
        supportsSessionList = try container.decodeIfPresent(Bool.self, forKey: .supportsSessionList) ?? false
        supportsSessionResume = try container.decodeIfPresent(Bool.self, forKey: .supportsSessionResume) ?? false
        supportsSessionLoad = try container.decodeIfPresent(Bool.self, forKey: .supportsSessionLoad) ?? false
        directTools = try container.decodeIfPresent([String: JSONValue].self, forKey: .directTools)
        chromeTools = try container.decodeIfPresent(String.self, forKey: .chromeTools) ?? "unknown"
    }
}

enum JSONValue: Codable, Equatable {
    case string(String)
    case bool(Bool)
    case number(Double)
    case object([String: JSONValue])
    case array([JSONValue])
    case null

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([String: JSONValue].self) {
            self = .object(value)
        } else if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
        } else {
            throw DecodingError.dataCorruptedError(in: container, debugDescription: "Unsupported JSON value")
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .string(let value):
            try container.encode(value)
        case .bool(let value):
            try container.encode(value)
        case .number(let value):
            try container.encode(value)
        case .object(let value):
            try container.encode(value)
        case .array(let value):
            try container.encode(value)
        case .null:
            try container.encodeNil()
        }
    }
}
