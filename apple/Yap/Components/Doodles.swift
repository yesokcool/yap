import SwiftUI
import YapCore

// Doodle reactions (PROTOCOL.md › Doodle reactions): the drawing surface and
// the image, as helpers (like `FlowLayout` or the GIF players) that the
// doodle window and the reaction tiles wrap (`Extras98.swift`).

/// The doodle being drawn, its tools and undo.
@MainActor
@Observable
final class DoodleEditor {
    var drawing = DoodleDrawing()
    /// Palette index; 0 = the eraser (paper).
    var color = 1
    var size = DoodleDrawing.sizes[1]
    var sending = false
    var error: String?
    private var live: DoodleDrawing.Stroke?

    var canUndo: Bool { !drawing.strokes.isEmpty }
    var canSend: Bool { !drawing.isEmpty && !sending }

    func begin(at point: DoodleDrawing.Point) {
        live = DoodleDrawing.Stroke(color: color, size: size, points: [point])
        drawing.strokes.append(live!)
    }

    func extend(to point: DoodleDrawing.Point) {
        guard var stroke = live else { return begin(at: point) }
        if let last = stroke.points.last, abs(last.x - point.x) < 0.25, abs(last.y - point.y) < 0.25 { return }
        stroke.points.append(point)
        live = stroke
        drawing.strokes[drawing.strokes.count - 1] = stroke
    }

    func end() { live = nil }

    func undo() {
        guard !drawing.strokes.isEmpty else { return }
        drawing.strokes.removeLast()
    }

    func clear() { drawing.strokes = [] }

    /// Upload it and react; true when done (the sheet closes).
    func send(on message: Message, client: YapClient) async -> Bool {
        let gif = drawing.gif()
        guard gif.count <= Doodle.maxBytes else {
            error = "That doodle is too detailed to send (64 KB at most)."
            return false
        }
        sending = true
        defer { sending = false }
        do {
            try await client.react(to: message, doodle: gif, mime: "image/gif")
            return true
        } catch {
            self.error = error.localizedDescription
            return false
        }
    }

    static func color(_ index: Int) -> Color {
        let c = DoodleDrawing.palette[max(0, min(DoodleDrawing.palette.count - 1, index))]
        return Color(red: Double(c.r) / 255, green: Double(c.g) / 255, blue: Double(c.b) / 255)
    }
}

/// The 48 × 48 box, drawn with hard pixel edges; drag to draw. When the
/// doodle wiggles, the preview cycles its three frames like the GIF will.
struct DoodleCanvas: View {
    @Bindable var editor: DoodleEditor
    var side: CGFloat = 240

    var body: some View {
        TimelineView(.periodic(from: .now, by: 0.14)) { context in
            let frame = editor.drawing.wiggle ? Int(context.date.timeIntervalSinceReferenceDate / 0.14) % Doodle.maxFrames : 0
            let pixels = editor.drawing.pixels(frame: frame)
            Canvas { gc, size in
                let n = DoodleDrawing.side
                let cell = size.width / CGFloat(n)
                gc.fill(Path(CGRect(origin: .zero, size: size)), with: .color(DoodleEditor.color(0)))
                for y in 0..<n {
                    for x in 0..<n {
                        let index = Int(pixels[y * n + x])
                        guard index != 0 else { continue }
                        gc.fill(Path(CGRect(x: CGFloat(x) * cell, y: CGFloat(y) * cell, width: cell + 0.5, height: cell + 0.5)),
                                with: .color(DoodleEditor.color(index)))
                    }
                }
            }
        }
        .frame(width: side, height: side)
        .contentShape(Rectangle())
        .gesture(
            DragGesture(minimumDistance: 0)
                .onChanged { value in
                    let scale = Float(CGFloat(DoodleDrawing.side) / side)
                    editor.extend(to: DoodleDrawing.Point(x: Float(value.location.x) * scale, y: Float(value.location.y) * scale))
                }
                .onEnded { _ in editor.end() }
        )
        .accessibilityLabel("Doodle canvas")
        .accessibilityHint("Drag to draw")
    }
}

/// A doodle reaction's picture (`/media/<id>`), pixel-crisp; GIFs play.
struct DoodleImage: View {
    @Environment(AppModel.self) private var model
    var id: String

    var body: some View {
        if let endpoint = model.client.endpoint {
            DoodleFrames(url: endpoint.mediaURL(id))
        } else {
            Color.white
        }
    }
}

private struct DoodleFrames: View {
    var url: URL
    @State private var animation = AnimatedImageModel()

    var body: some View {
        Group {
            if let frame = animation.frame {
                Image(decorative: frame, scale: 1).resizable().interpolation(.none).scaledToFit()
            } else {
                Color.white.opacity(0.6)
            }
        }
        .task(id: url) { await animation.load(url) }
        .onDisappear { animation.stop() }
    }
}
