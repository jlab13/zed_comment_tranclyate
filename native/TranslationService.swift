import Foundation
import NaturalLanguage

struct TranslationRequest: Decodable {
    let provider: String?
    let operation: String?
    let text: String?
    let source_language: String?
    let timeout_seconds: Int?
}

struct TranslationResponse: Encodable {
    var provider: String? = nil
    var translation: String? = nil
    var error: String? = nil
    var status: String? = nil
    var source_language: String? = nil
    var strategy: String? = nil
    var checked_strategies: [String: String]? = nil

    func json() -> String {
        guard let data = try? JSONEncoder().encode(self), let json = String(data: data, encoding: .utf8) else {
            return #"{"error":"Не вдалося закодувати відповідь перекладача."}"#
        }
        return json
    }
}

struct TranslationService {
    static func handle(_ json: String) async -> TranslationResponse {
        guard !Task.isCancelled else { return TranslationResponse(error: "Переклад скасовано.") }
        guard #available(macOS 26.0, *) else {
            return TranslationResponse(error: "Потрібна macOS 26 або новіша.")
        }
        let request: TranslationRequest
        do {
            request = try JSONDecoder().decode(TranslationRequest.self, from: Data(json.utf8))
        } catch {
            return TranslationResponse(error: "Некоректний запит JSON. Очікується текстове поле text.")
        }
        let provider = request.provider ?? "apple"
        guard provider == "apple" else {
            return TranslationResponse(error: "Нативна бібліотека обслуговує лише Apple; Google викликається з Rust.")
        }
        guard (1...300).contains(request.timeout_seconds ?? 60) else {
            return TranslationResponse(provider: provider, error: "timeout_seconds має бути в межах 1–300.")
        }
        if request.operation == "check" {
            var response = await AppleTranslate.check(source: request.source_language ?? "en")
            response.provider = provider
            return response
        }
        guard request.operation == nil || request.operation == "translate" else {
            return TranslationResponse(provider: provider, error: "Невідома операція перекладу.")
        }
        guard let text = request.text else {
            return TranslationResponse(provider: provider, error: "Некоректний запит JSON. Очікується текстове поле text.")
        }
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return TranslationResponse(provider: provider, error: "Немає тексту для перекладу.")
        }
        let requestedSource = request.source_language ?? "auto"
        guard !requestedSource.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return TranslationResponse(provider: provider, error: "source_language не може бути порожнім.")
        }
        if Locale.Language(identifier: requestedSource).languageCode?.identifier == "uk" {
            return TranslationResponse(provider: provider, translation: text, source_language: requestedSource)
        }
        let source: String
        if requestedSource == "auto" {
            guard let detected = NLLanguageRecognizer.dominantLanguage(for: text) else {
                return TranslationResponse(provider: provider, error: "Не вдалося визначити мову. Вкажіть source_language, наприклад en.")
            }
            source = detected.rawValue
        } else {
            source = requestedSource
        }
        if Locale.Language(identifier: source).languageCode?.identifier == "uk" {
            return TranslationResponse(provider: provider, translation: text, source_language: source)
        }
        var response = await AppleTranslate.translate(text, source: source)
        response.provider = provider
        return response
    }
}
