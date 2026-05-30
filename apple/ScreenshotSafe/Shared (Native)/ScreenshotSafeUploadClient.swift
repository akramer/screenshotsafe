import Foundation

struct ScreenshotSafeUploadResult: Decodable {
    let id: String
    let shareId: String
    let shareURL: URL
    let rawURL: URL

    enum CodingKeys: String, CodingKey {
        case id
        case shareId = "share_id"
        case shareURL = "share_url"
        case rawURL = "raw_url"
    }
}

enum ScreenshotSafeUploadError: LocalizedError {
    case notConfigured
    case invalidServerURL
    case invalidResponse
    case server(String)

    var errorDescription: String? {
        switch self {
        case .notConfigured:
            return "Add your ScreenshotSafe server URL and API token first."
        case .invalidServerURL:
            return "The ScreenshotSafe server URL is invalid."
        case .invalidResponse:
            return "ScreenshotSafe returned an unexpected response."
        case .server(let message):
            return message
        }
    }
}

final class ScreenshotSafeUploadClient {
    private let session: URLSession

    init(session: URLSession = .shared) {
        self.session = session
    }

    func verify(settings: ScreenshotSafeSettings, completion: @escaping (Result<Void, Error>) -> Void) {
        guard settings.isConfigured else {
            completion(.failure(ScreenshotSafeUploadError.notConfigured))
            return
        }

        guard let url = endpoint(path: "/api/ping", serverURL: settings.serverURL) else {
            completion(.failure(ScreenshotSafeUploadError.invalidServerURL))
            return
        }

        var request = URLRequest(url: url)
        request.setValue("Bearer \(settings.apiToken)", forHTTPHeaderField: "Authorization")

        session.dataTask(with: request) { _, response, error in
            if let error = error {
                completion(.failure(error))
                return
            }

            guard let http = response as? HTTPURLResponse else {
                completion(.failure(ScreenshotSafeUploadError.invalidResponse))
                return
            }

            if http.statusCode == 200 {
                completion(.success(()))
            } else if http.statusCode == 401 {
                completion(.failure(ScreenshotSafeUploadError.server("The API token was rejected.")))
            } else {
                completion(.failure(ScreenshotSafeUploadError.server("Server returned \(http.statusCode).")))
            }
        }.resume()
    }

    func upload(
        imageData: Data,
        filename: String,
        title: String,
        sourceURL: String?,
        imageDPI: Double? = nil,
        settings: ScreenshotSafeSettings,
        completion: @escaping (Result<ScreenshotSafeUploadResult, Error>) -> Void
    ) {
        guard settings.isConfigured else {
            completion(.failure(ScreenshotSafeUploadError.notConfigured))
            return
        }

        guard let url = endpoint(path: "/api/screenshots", serverURL: settings.serverURL) else {
            completion(.failure(ScreenshotSafeUploadError.invalidServerURL))
            return
        }

        let boundary = "ScreenshotSafe-\(UUID().uuidString)"
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("Bearer \(settings.apiToken)", forHTTPHeaderField: "Authorization")
        request.setValue("multipart/form-data; boundary=\(boundary)", forHTTPHeaderField: "Content-Type")

        let body = multipartBody(
            boundary: boundary,
            imageData: imageData,
            filename: filename,
            title: title,
            sourceURL: sourceURL,
            imageDPI: imageDPI,
            expiresIn: settings.defaultExpiry
        )
        request.setValue(String(body.count), forHTTPHeaderField: "Content-Length")
        request.httpBody = body

        session.dataTask(with: request) { data, response, error in
            if let error = error {
                completion(.failure(error))
                return
            }

            guard let http = response as? HTTPURLResponse, let data = data else {
                completion(.failure(ScreenshotSafeUploadError.invalidResponse))
                return
            }

            guard (200..<300).contains(http.statusCode) else {
                let message = Self.serverErrorMessage(from: data) ?? "Upload failed (\(http.statusCode))."
                completion(.failure(ScreenshotSafeUploadError.server(message)))
                return
            }

            do {
                completion(.success(try JSONDecoder().decode(ScreenshotSafeUploadResult.self, from: data)))
            } catch {
                completion(.failure(error))
            }
        }.resume()
    }

    private func endpoint(path: String, serverURL: String) -> URL? {
        let trimmed = serverURL.trimmingCharacters(in: .whitespacesAndNewlines).trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        guard !trimmed.isEmpty, let base = URL(string: trimmed) else {
            return nil
        }
        return base.appendingPathComponent(path.trimmingCharacters(in: CharacterSet(charactersIn: "/")))
    }

    private func multipartBody(
        boundary: String,
        imageData: Data,
        filename: String,
        title: String,
        sourceURL: String?,
        imageDPI: Double?,
        expiresIn: String
    ) -> Data {
        var body = Data()
        body.appendField("title", value: title.isEmpty ? "Screenshot" : title, boundary: boundary)
        if let sourceURL = sourceURL, !sourceURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            body.appendField("source_url", value: sourceURL, boundary: boundary)
        }
        if let imageDPI = imageDPI, imageDPI.isFinite, imageDPI > 0 {
            body.appendField("image_dpi", value: Self.formattedDPI(imageDPI), boundary: boundary)
        }
        if !expiresIn.isEmpty {
            body.appendField("expires_in", value: expiresIn, boundary: boundary)
        }
        body.appendFile("image", filename: Self.pngFilename(from: filename), mimeType: "image/png", data: imageData, boundary: boundary)
        body.appendString("--\(boundary)--\r\n")
        return body
    }

    private static func pngFilename(from filename: String) -> String {
        let trimmed = filename.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return "screenshot.png"
        }

        let base = (trimmed as NSString).lastPathComponent
        let withoutExtension = (base as NSString).deletingPathExtension
        let safeBase = withoutExtension.isEmpty ? "screenshot" : withoutExtension
        return "\(safeBase).png"
    }

    private static func formattedDPI(_ dpi: Double) -> String {
        let clamped = min(max(dpi, 1), 2400)
        if clamped.rounded() == clamped {
            return String(Int(clamped))
        }
        return String(format: "%.2f", clamped)
    }

    private static func serverErrorMessage(from data: Data) -> String? {
        guard
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let error = object["error"] as? String
        else {
            return nil
        }
        return error
    }
}

private extension Data {
    mutating func appendField(_ name: String, value: String, boundary: String) {
        appendString("--\(boundary)\r\n")
        appendString("Content-Disposition: form-data; name=\"\(Self.multipartEscaped(name))\"\r\n\r\n")
        appendString("\(value)\r\n")
    }

    mutating func appendFile(_ name: String, filename: String, mimeType: String, data: Data, boundary: String) {
        appendString("--\(boundary)\r\n")
        appendString("Content-Disposition: form-data; name=\"\(Self.multipartEscaped(name))\"; filename=\"\(Self.multipartEscaped(filename))\"\r\n")
        appendString("Content-Type: \(mimeType)\r\n\r\n")
        append(data)
        appendString("\r\n")
    }

    mutating func appendString(_ string: String) {
        append(Data(string.utf8))
    }

    static func multipartEscaped(_ value: String) -> String {
        value
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
            .replacingOccurrences(of: "\r", with: "")
            .replacingOccurrences(of: "\n", with: "")
    }
}
