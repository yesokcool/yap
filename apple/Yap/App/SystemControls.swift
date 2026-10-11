import SwiftUI
#if os(iOS)
import UIKit
#endif

/// Whether the clipboard holds something a Paste button could use. Asking
/// what *kind* of thing is there doesn't read it (no "Allow Paste?"), so the
/// system's Paste button — which can't look like Windows 98 — only shows when it's
/// useful. Always true on the Mac.
struct ClipboardHas: ViewModifier {
    var pictures = false
    @Binding var available: Bool

    func body(content: Content) -> some View {
        #if os(iOS)
        content
            .onAppear(perform: refresh)
            .onReceive(NotificationCenter.default.publisher(for: UIPasteboard.changedNotification)) { _ in refresh() }
            .onReceive(NotificationCenter.default.publisher(for: UIApplication.didBecomeActiveNotification)) { _ in refresh() }
        #else
        content.onAppear { available = true }
        #endif
    }

    #if os(iOS)
    private func refresh() {
        let board = UIPasteboard.general
        available = board.hasStrings || board.hasURLs || (pictures && board.hasImages)
    }
    #endif
}

extension View {
    func clipboardHas(pictures: Bool = false, _ available: Binding<Bool>) -> some View {
        modifier(ClipboardHas(pictures: pictures, available: available))
    }
}

/// A date or time picker drawn in Win98 style. On iPhone the system's compact
/// picker draws rounded pills; here they're made invisible (still the thing
/// you tap, so the system's own picker opens) with a Win98 field drawn in
/// their place. The Mac keeps the system's field-and-stepper.
struct SkinnedDatePicker<Face: View>: View {
    var selection: Binding<Date>
    var range: ClosedRange<Date>?
    var components: DatePickerComponents
    @ViewBuilder var face: (String) -> Face

    var body: some View {
        #if os(iOS)
        HStack(spacing: 6) {
            if components.contains(.date) {
                piece(.date, text: selection.wrappedValue.formatted(date: .abbreviated, time: .omitted))
            }
            if components.contains(.hourAndMinute) {
                piece(.hourAndMinute, text: selection.wrappedValue.formatted(date: .omitted, time: .shortened))
            }
        }
        #else
        picker(components).labelsHidden()
        #endif
    }

    @ViewBuilder
    private func picker(_ shown: DatePickerComponents) -> some View {
        if let range {
            DatePicker("", selection: selection, in: range, displayedComponents: shown)
        } else {
            DatePicker("", selection: selection, displayedComponents: shown)
        }
    }

    private func piece(_ shown: DatePickerComponents, text: String) -> some View {
        picker(shown)
            .labelsHidden()
            .colorMultiply(.clear)
            .background { face(text) }
            .accessibilityElement(children: .contain)
    }
}

/// Wraps buttons onto new lines.
struct FlowLayout: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? .infinity
        var x: CGFloat = 0, y: CGFloat = 0, rowHeight: CGFloat = 0, maxX: CGFloat = 0
        for view in subviews {
            let size = view.sizeThatFits(.unspecified)
            if x > 0, x + size.width > width {
                x = 0
                y += rowHeight + spacing
                rowHeight = 0
            }
            x += size.width + spacing
            maxX = max(maxX, x - spacing)
            rowHeight = max(rowHeight, size.height)
        }
        return CGSize(width: maxX, height: y + rowHeight)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x = bounds.minX, y = bounds.minY, rowHeight: CGFloat = 0
        for view in subviews {
            let size = view.sizeThatFits(.unspecified)
            if x > bounds.minX, x + size.width > bounds.maxX {
                x = bounds.minX
                y += rowHeight + spacing
                rowHeight = 0
            }
            view.place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
        }
    }
}
