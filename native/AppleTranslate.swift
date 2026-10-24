import Foundation
import Translation

struct AppleTranslate {
    @available(macOS 26.0, *)
    static func check(source: String) async -> TranslationResponse {
        if Locale.Language(identifier: source).languageCode?.identifier == "uk" {
            return TranslationResponse(status: "same_language", source_language: source)
        }
        let selection = await selectStrategy(source: Locale.Language(identifier: source))
        return TranslationResponse(
            error: availabilityError(selection.status, source: source),
            status: selection.status.rawValue,
            source_language: source,
            strategy: selection.strategy?.rawValue,
            checked_strategies: selection.checkedStrategies
        )
    }

    @available(macOS 26.0, *)
    static func availability(_ status: LanguageAvailability.Status) -> TranslationAvailability {
        switch status {
        case .installed: return .installed
        case .supported: return .supported
        case .unsupported: return .unsupported
        @unknown default: return .unknown
        }
    }

    @available(macOS 26.0, *)
    static func selectStrategy(source: Locale.Language) async -> StrategySelection {
        let target = Locale.Language(identifier: "uk")
        if #available(macOS 26.4, *) {
            let high = availability(await LanguageAvailability(preferredStrategy: .highFidelity)
                .status(from: source, to: target))
            if high == .installed {
                return StrategySelection.choose(highFidelity: high, lowLatency: nil)
            }
            guard !Task.isCancelled else {
                return StrategySelection.choose(highFidelity: .unknown, lowLatency: nil)
            }
            // Перевіряємо традиційні моделі, якщо highFidelity ще не готова.
            let low = availability(await LanguageAvailability(preferredStrategy: .lowLatency)
                .status(from: source, to: target))
            return StrategySelection.choose(highFidelity: high, lowLatency: low)
        }
        let low = availability(await LanguageAvailability().status(from: source, to: target))
        return StrategySelection.choose(highFidelity: nil, lowLatency: low)
    }

    static func availabilityError(_ status: TranslationAvailability, source: String) -> String? {
        switch status {
        case .installed: return nil
        case .supported: return missingLanguages(source)
        case .unsupported: return "Apple Translation не підтримує переклад \(source) → uk."
        case .unknown: return "Невідомий стан моделей Apple Translation."
        }
    }

    static func missingLanguages(_ source: String) -> String {
        "Моделі перекладу \(source) → uk не готові. На macOS 26.4+ перевірте, чи ввімкнено Apple Intelligence та завантажено його моделі. Або завантажте вихідну мову й українську: Системні параметри → Загальні → Мова й регіон → Мови перекладу."
    }

    @available(macOS 26.0, *)
    static func translate(_ text: String, source: String) async -> TranslationResponse {
        let sourceLanguage = Locale.Language(identifier: source)
        let targetLanguage = Locale.Language(identifier: "uk")
        let selection = await selectStrategy(source: sourceLanguage)
        guard !Task.isCancelled else { return TranslationResponse(error: "Переклад скасовано.") }
        guard let strategy = selection.strategy else {
            return TranslationResponse(
                error: availabilityError(selection.status, source: source),
                status: selection.status.rawValue,
                source_language: source,
                checked_strategies: selection.checkedStrategies
            )
        }
        do {
            let session: TranslationSession
            if #available(macOS 26.4, *) {
                let preferred: TranslationSession.Strategy = strategy == .highFidelity ? .highFidelity : .lowLatency
                session = TranslationSession(installedSource: sourceLanguage, target: targetLanguage, preferredStrategy: preferred)
            } else {
                session = TranslationSession(installedSource: sourceLanguage, target: targetLanguage)
            }
            let response = try await withTaskCancellationHandler {
                try await session.translate(text)
            } onCancel: {
                session.cancel()
            }
            return TranslationResponse(translation: response.targetText, source_language: source, strategy: strategy.rawValue)
        } catch TranslationError.notInstalled {
            return TranslationResponse(error: missingLanguages(source))
        } catch {
            return TranslationResponse(error: "Не вдалося виконати переклад Apple Translation: \(error.localizedDescription)")
        }
    }
}
