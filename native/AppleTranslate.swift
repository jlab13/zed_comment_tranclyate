import Foundation
import Translation

struct AppleTranslate {
    /// Мови збігаються без урахування регіону: uk-UA і uk. Китайська розрізняє письмо.
    static func sameLanguage(_ source: String, _ target: String) -> Bool {
        let source = Locale.Language(identifier: source)
        let target = Locale.Language(identifier: target)
        guard let code = source.languageCode, code == target.languageCode else { return false }
        guard code.identifier == "zh" else { return true }
        // Повний ідентифікатор додає типове письмо: zh → zh-Hans-CN, zh-TW → zh-Hant-TW.
        let script = { (language: Locale.Language) in Locale.Language(identifier: language.maximalIdentifier).script }
        return script(source) == script(target)
    }

    @available(macOS 26.0, *)
    static func check(source: String, target: String) async -> TranslationResponse {
        if sameLanguage(source, target) {
            return TranslationResponse(status: "same_language", source_language: source)
        }
        let selection = await selectStrategy(source: Locale.Language(identifier: source), target: Locale.Language(identifier: target))
        return TranslationResponse(
            error: availabilityError(selection.status, source: source, target: target),
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
    static func selectStrategy(source: Locale.Language, target: Locale.Language) async -> StrategySelection {
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

    static func availabilityError(_ status: TranslationAvailability, source: String, target: String) -> String? {
        switch status {
        case .installed: return nil
        case .supported: return missingLanguages(source, target: target)
        case .unsupported: return "Apple Translation не підтримує переклад \(source) → \(target)."
        case .unknown: return "Невідомий стан моделей Apple Translation."
        }
    }

    static func missingLanguages(_ source: String, target: String) -> String {
        "Моделі перекладу \(source) → \(target) не готові. На macOS 26.4+ перевірте, чи ввімкнено Apple Intelligence та завантажено його моделі. Або завантажте вихідну й цільову мови: Системні параметри → Загальні → Мова й регіон → Мови перекладу."
    }

    @available(macOS 26.0, *)
    static func translate(_ text: String, source: String, target: String) async -> TranslationResponse {
        let sourceLanguage = Locale.Language(identifier: source)
        let targetLanguage = Locale.Language(identifier: target)
        let selection = await selectStrategy(source: sourceLanguage, target: targetLanguage)
        guard !Task.isCancelled else { return TranslationResponse(error: "Переклад скасовано.") }
        guard let strategy = selection.strategy else {
            return TranslationResponse(
                error: availabilityError(selection.status, source: source, target: target),
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
            return TranslationResponse(error: missingLanguages(source, target: target))
        } catch {
            return TranslationResponse(error: "Не вдалося виконати переклад Apple Translation: \(error.localizedDescription)")
        }
    }
}
