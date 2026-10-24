import Foundation

// Rust передає ідентифікатор, а не вказівник на свій стан: пізня відповідь безпечна.
public typealias TranslationCallback = @convention(c) (UInt64, UnsafePointer<CChar>?) -> Void

private final class TranslationTasks: @unchecked Sendable {
    static let shared = TranslationTasks()
    private let lock = NSLock()
    private var tasks: [UInt64: Task<Void, Never>] = [:]

    func start(id: UInt64, json: String, callback: @escaping TranslationCallback) {
        lock.lock()
        // Обмежуємо також задачі, чий системний виклик не реагує на скасування.
        guard tasks.count < 8 else {
            lock.unlock()
            TranslationResponse(error: "Перекладач зайнятий: попередні запити ще не завершено.")
                .json().withCString { callback(id, $0) }
            return
        }
        tasks[id] = Task.detached {
            let response = await TranslationService.handle(json)
            if !Task.isCancelled {
                // Буфер дійсний лише протягом callback; Rust одразу копіює його.
                response.json().withCString { callback(id, $0) }
            }
            self.remove(id: id)
        }
        lock.unlock()
    }

    private func remove(id: UInt64) {
        lock.lock()
        tasks.removeValue(forKey: id)
        lock.unlock()
    }

    func cancel(id: UInt64) {
        lock.lock()
        let task = tasks[id]
        lock.unlock()
        task?.cancel()
    }
}

@_cdecl("comment_translate_start")
public func commentTranslateStart(_ id: UInt64, _ json: UnsafePointer<CChar>?, _ callback: @escaping TranslationCallback) {
    // Копіюємо вхідний рядок до повернення в Rust.
    let request = json.map { String(cString: $0) } ?? ""
    TranslationTasks.shared.start(id: id, json: request, callback: callback)
}

@_cdecl("comment_translate_cancel")
public func commentTranslateCancel(_ id: UInt64) {
    TranslationTasks.shared.cancel(id: id)
}

@_cdecl("comment_translate_run_main_loop")
public func commentTranslateRunMainLoop() {
    // Обслуговуємо головну чергу macOS для Swift concurrency та системних callback.
    dispatchMain()
}
