@main
struct StrategyTests {
    static func main() {
        let ready = StrategySelection.choose(highFidelity: .installed, lowLatency: nil)
        assert(ready.strategy == .highFidelity)
        assert(ready.status == .installed)
        assert(ready.checkedStrategies == ["highFidelity": "installed"])

        let preferred = StrategySelection.choose(highFidelity: .installed, lowLatency: .installed)
        assert(preferred.strategy == .highFidelity)

        for high in [TranslationAvailability.supported, .unsupported, .unknown] {
            let fallback = StrategySelection.choose(highFidelity: high, lowLatency: .installed)
            assert(fallback.strategy == .lowLatency)
            assert(fallback.checkedStrategies["highFidelity"] == high.rawValue)
        }

        let legacy = StrategySelection.choose(highFidelity: nil, lowLatency: .installed)
        assert(legacy.strategy == .lowLatency)
        assert(legacy.checkedStrategies["highFidelity"] == nil)

        let download = StrategySelection.choose(highFidelity: .supported, lowLatency: .unsupported)
        assert(download.strategy == nil && download.status == .supported)
        let traditionalDownload = StrategySelection.choose(highFidelity: .unsupported, lowLatency: .supported)
        assert(traditionalDownload.strategy == nil && traditionalDownload.status == .supported)

        let unsupported = StrategySelection.choose(highFidelity: .unsupported, lowLatency: .unsupported)
        assert(unsupported.status == .unsupported)
        let legacyUnsupported = StrategySelection.choose(highFidelity: nil, lowLatency: .unsupported)
        assert(legacyUnsupported.status == .unsupported)
        let unknown = StrategySelection.choose(highFidelity: .unknown, lowLatency: .unsupported)
        assert(unknown.status == .unknown)
        print("Вибір стратегії: 11 сценаріїв пройшли")
    }
}
