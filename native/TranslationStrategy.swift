enum TranslationStrategy: String {
    case highFidelity
    case lowLatency
}

enum TranslationAvailability: String {
    case installed
    case supported
    case unsupported
    case unknown
}

struct StrategySelection {
    let status: TranslationAvailability
    let strategy: TranslationStrategy?
    let checkedStrategies: [String: String]

    static func choose(highFidelity: TranslationAvailability?, lowLatency: TranslationAvailability?) -> StrategySelection {
        var checked: [String: String] = [:]
        if let highFidelity { checked[TranslationStrategy.highFidelity.rawValue] = highFidelity.rawValue }
        if let lowLatency { checked[TranslationStrategy.lowLatency.rawValue] = lowLatency.rawValue }
        if highFidelity == .installed {
            return StrategySelection(status: .installed, strategy: .highFidelity, checkedStrategies: checked)
        }
        if lowLatency == .installed {
            return StrategySelection(status: .installed, strategy: .lowLatency, checkedStrategies: checked)
        }
        if highFidelity == .supported || lowLatency == .supported {
            return StrategySelection(status: .supported, strategy: nil, checkedStrategies: checked)
        }
        let status: TranslationAvailability = lowLatency == .unsupported && (highFidelity == nil || highFidelity == .unsupported)
            ? .unsupported : .unknown
        return StrategySelection(status: status, strategy: nil, checkedStrategies: checked)
    }
}
