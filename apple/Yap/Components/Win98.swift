import SwiftUI
#if os(iOS)
import UIKit
#else
import AppKit
#endif

// The Windows 98 building blocks: two-ring bevels, beveled buttons, navy
// title bars, sunken wells, status-bar panels, etched group boxes and
// property-sheet tabs. All square, no soft shadows; colors from `Theme`.

/// Which way a bevel faces.
enum BevelStyle {
    /// Buttons, windows: hilight/light top-left, shadow/dkShadow bottom-right.
    case raised
    /// A pressed button: the rings invert.
    case pressed
    /// Text fields, lists, the chat log: shadow/dkShadow top-left, hilight/light bottom-right.
    case sunken
    /// A status-bar panel: one ring, shadow top-left / hilight bottom-right.
    case status
    /// A thin raised ring (title-bar buttons, small pills).
    case thin
}

/// Draws the 1pt bevel rings inside its frame.
struct Bevel: View {
    var style: BevelStyle

    var body: some View {
        Canvas { context, size in
            func ring(_ inset: CGFloat, topLeft: Color, bottomRight: Color) {
                let w = size.width - inset * 2, h = size.height - inset * 2
                guard w > 2, h > 2 else { return }
                let x = inset, y = inset
                context.fill(Path(CGRect(x: x, y: y, width: w - 1, height: 1)), with: .color(topLeft))
                context.fill(Path(CGRect(x: x, y: y, width: 1, height: h - 1)), with: .color(topLeft))
                context.fill(Path(CGRect(x: x, y: y + h - 1, width: w, height: 1)), with: .color(bottomRight))
                context.fill(Path(CGRect(x: x + w - 1, y: y, width: 1, height: h)), with: .color(bottomRight))
            }
            switch style {
            case .raised:
                ring(0, topLeft: Theme.hilight, bottomRight: Theme.dkShadow)
                ring(1, topLeft: Theme.light, bottomRight: Theme.shadow)
            case .pressed:
                ring(0, topLeft: Theme.dkShadow, bottomRight: Theme.hilight)
                ring(1, topLeft: Theme.shadow, bottomRight: Theme.light)
            case .sunken:
                ring(0, topLeft: Theme.shadow, bottomRight: Theme.hilight)
                ring(1, topLeft: Theme.dkShadow, bottomRight: Theme.light)
            case .status:
                ring(0, topLeft: Theme.shadow, bottomRight: Theme.hilight)
            case .thin:
                ring(0, topLeft: Theme.hilight, bottomRight: Theme.shadow)
            }
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

extension View {
    /// A fill plus a bevel, square.
    func bevel(_ style: BevelStyle, fill: Color = Theme.face) -> some View {
        background(fill).overlay(Bevel(style: style))
    }

    /// A white (window-color) sunken well: text fields, lists, the chat log.
    func well() -> some View {
        padding(2).background(Theme.window).overlay(Bevel(style: .sunken))
    }
}

// MARK: - Buttons

/// A Windows 98 push button. Pressed = inverted rings, label nudged 1pt
/// right and down. `isDefault` adds the black outline of the default button.
/// On iOS the drawn button stays Win98-sized but the hit target is 44 pt.
struct Win98ButtonStyle: ButtonStyle {
    var isDefault = false
    var compact = false
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        let pressed = configuration.isPressed
        #if os(iOS)
        let minimum = compact ? CGSize(width: 32, height: 30) : Theme.minButtonSize
        #else
        // Icon buttons can be narrow, but nothing is shorter than 23.
        let minimum = compact ? CGSize(width: 23, height: 23) : Theme.minButtonSize
        #endif
        configuration.label
            .font(Theme.chrome)
            .foregroundStyle(isEnabled ? Theme.text : Theme.grayText)
            .shadow(color: isEnabled ? .clear : Theme.hilight, radius: 0, x: 1, y: 1)
            .lineLimit(1)
            .padding(.horizontal, compact ? 6 : 10)
            .padding(.vertical, compact ? 3 : 2)
            .frame(minWidth: minimum.width, minHeight: minimum.height)
            .offset(x: pressed ? 1 : 0, y: pressed ? 1 : 0)
            .background(Theme.face)
            .overlay(Bevel(style: pressed ? .pressed : .raised))
            .padding(isDefault ? 1 : 0)
            .overlay(isDefault ? Rectangle().stroke(Theme.dkShadow, lineWidth: 1) : nil)
            #if os(iOS)
            .padding(.vertical, compact ? 0 : max(0, (44 - minimum.height) / 2))
            // Compact buttons keep their drawn size; the touch area grows to ~44 pt.
            .contentShape(Rectangle().inset(by: compact ? -7 : 0))
            #else
            .contentShape(.rect)
            #endif
    }
}

extension ButtonStyle where Self == Win98ButtonStyle {
    static var win98: Win98ButtonStyle { Win98ButtonStyle() }
    static var win98Default: Win98ButtonStyle { Win98ButtonStyle(isDefault: true) }
    static var win98Compact: Win98ButtonStyle { Win98ButtonStyle(compact: true) }
}

/// A toggle drawn as a button that stays pushed in while on (mute, deafen, reactions).
struct Win98ToggleButtonStyle: ButtonStyle {
    var isOn: Bool
    var compact = false

    func makeBody(configuration: Configuration) -> some View {
        let down = isOn || configuration.isPressed
        configuration.label
            .font(Theme.chrome)
            .foregroundStyle(Theme.text)
            .padding(.horizontal, compact ? 6 : 10)
            .padding(.vertical, compact ? 3 : 2)
            .frame(minHeight: compact ? 0 : Theme.minButtonSize.height)
            .offset(x: down ? 1 : 0, y: down ? 1 : 0)
            .background(isOn ? Theme.light : Theme.face)
            .overlay(Bevel(style: down ? .pressed : .raised))
            #if os(iOS)
            // Small (reactions, tones): drawn small, touched ~44 pt like win98Compact.
            .contentShape(Rectangle().inset(by: compact ? -7 : 0))
            #else
            .contentShape(.rect)
            #endif
    }
}

// MARK: - Title bars, windows, status bars

/// A navy gradient title bar with bold white text and optional buttons.
struct TitleBar<Trailing: View>: View {
    var title: String
    var systemImage: String? = nil
    var active = true
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 5) {
            if let systemImage {
                Image(systemName: systemImage)
                    .font(.system(size: 10, weight: .bold))
            }
            Text(title)
                .font(Theme.title)
                .lineLimit(1)
            Spacer(minLength: 4)
            trailing
        }
        .foregroundStyle(active ? Theme.titleText : Theme.inactiveTitleText)
        .padding(.leading, 5)
        .padding(.trailing, 2)
        .frame(minHeight: Theme.titleBarHeight)
        .background(
            LinearGradient(colors: active ? [Theme.titleStart, Theme.titleEnd] : [Theme.inactiveTitleStart, Theme.inactiveTitleEnd],
                           startPoint: .leading, endPoint: .trailing)
        )
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(.isHeader)
    }
}

extension TitleBar where Trailing == EmptyView {
    init(title: String, systemImage: String? = nil, active: Bool = true) {
        self.init(title: title, systemImage: systemImage, active: active) { EmptyView() }
    }
}

/// The little square buttons on a title bar (×, menu, members…).
struct TitleBarButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 9, weight: .black))
            .foregroundStyle(Theme.text)
            .frame(minWidth: 16, minHeight: 14)
            .padding(.horizontal, 2)
            .offset(x: configuration.isPressed ? 1 : 0, y: configuration.isPressed ? 1 : 0)
            .background(Theme.face)
            .overlay(Bevel(style: configuration.isPressed ? .pressed : .raised))
            #if os(iOS)
            .padding(.vertical, 8)
            .padding(.horizontal, 4)
            #endif
            .contentShape(.rect)
    }
}

/// A window: raised face border, a title bar, then the content.
struct Window98<Content: View, Trailing: View>: View {
    var title: String
    var systemImage: String? = nil
    var active = true
    @ViewBuilder var trailing: Trailing
    @ViewBuilder var content: Content

    var body: some View {
        VStack(spacing: 0) {
            TitleBar(title: title, systemImage: systemImage, active: active) { trailing }
            content
        }
        .padding(3)
        .background(Theme.face)
        .overlay(Bevel(style: .raised))
    }
}

extension Window98 where Trailing == EmptyView {
    init(title: String, systemImage: String? = nil, active: Bool = true, @ViewBuilder content: () -> Content) {
        self.init(title: title, systemImage: systemImage, active: active, trailing: { EmptyView() }, content: content)
    }
}

/// One sunken section of a status bar.
struct StatusPanel: View {
    var text: String
    var systemImage: String? = nil
    var color: Color = Theme.text

    var body: some View {
        HStack(spacing: 4) {
            if let systemImage {
                Image(systemName: systemImage).font(.system(size: 9, weight: .bold)).foregroundStyle(color)
            }
            Text(text).lineLimit(1)
        }
        .font(Theme.small)
        .foregroundStyle(Theme.text)
        .padding(.horizontal, 5)
        .padding(.vertical, 2)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(Bevel(style: .status))
    }
}

/// A Windows 98 group box: an etched frame with its title on the top edge.
struct GroupBox98<Content: View>: View {
    var title: String
    @ViewBuilder var content: Content
    @State private var labelHeight: CGFloat = 14

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(title)
                .font(Theme.chrome)
                .foregroundStyle(Theme.text)
                .padding(.horizontal, 3)
                .background(Theme.face)
                .padding(.leading, 8)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { labelHeight = $0 }
                .zIndex(1)
                .accessibilityAddTraits(.isHeader)
            VStack(alignment: .leading, spacing: 8) {
                content
            }
            .padding(.horizontal, 10)
            .padding(.top, 6)
            .padding(.bottom, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background {
            // Etched: a shadow line with a hilight line 1pt below/right,
            // starting at the middle of the title.
            ZStack {
                Rectangle().stroke(Theme.hilight, lineWidth: 1).offset(x: 1, y: 1)
                Rectangle().stroke(Theme.shadow, lineWidth: 1)
            }
            .padding(.top, labelHeight / 2)
            .padding(.trailing, 1)
            .padding(.bottom, 1)
        }
    }
}

/// Property-sheet tabs: the selected tab is raised and joins the page below.
struct Tabs98<Tab: Hashable>: View {
    var tabs: [(Tab, String)]
    @Binding var selection: Tab

    var body: some View {
        HStack(alignment: .bottom, spacing: 0) {
            ForEach(tabs, id: \.0) { tab, title in
                let selected = tab == selection
                Button {
                    selection = tab
                } label: {
                    Text(title)
                        .font(Theme.chrome)
                        .foregroundStyle(Theme.text)
                        .padding(.horizontal, 10)
                        #if os(iOS)
                        // Fingers: a taller tab (the strip's look is the same).
                        .padding(.vertical, selected ? 11 : 9)
                        #else
                        .padding(.vertical, selected ? 5 : 3)
                        #endif
                        .background(Theme.face)
                        .overlay(TabEdges(selected: selected))
                        .offset(y: selected ? 1 : 0)
                        .zIndex(selected ? 1 : 0)
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(selected ? [.isSelected, .isButton] : .isButton)
            }
            Spacer(minLength: 0)
        }
    }
}

/// A tab's top/left/right edges (no bottom, so it joins the page).
private struct TabEdges: View {
    var selected: Bool

    var body: some View {
        Canvas { context, size in
            let w = size.width, h = size.height + (selected ? 1 : 0)
            context.fill(Path(CGRect(x: 0, y: 1, width: 1, height: h)), with: .color(Theme.hilight))
            context.fill(Path(CGRect(x: 1, y: 0, width: w - 2, height: 1)), with: .color(Theme.hilight))
            context.fill(Path(CGRect(x: w - 1, y: 1, width: 1, height: h)), with: .color(Theme.dkShadow))
            context.fill(Path(CGRect(x: w - 2, y: 1, width: 1, height: h)), with: .color(Theme.shadow))
        }
        .allowsHitTesting(false)
    }
}

/// A radio button: a sunken white circle with a dot when on.
struct Radio98: View {
    var title: String
    var isOn: Bool
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                ZStack {
                    // The sunken bevel, as two rings of half circles: top-left
                    // shadow / dark shadow, bottom-right hilight / light. (A trim
                    // can't wrap past 1, so the bottom-right halves are the
                    // top-left ones turned around.)
                    let topLeft = Circle().trim(from: 0.375, to: 0.875)
                    let innerTopLeft = Circle().inset(by: 1).trim(from: 0.375, to: 0.875)
                    Circle().fill(Theme.window)
                    topLeft.stroke(Theme.shadow, lineWidth: 1)
                    topLeft.stroke(Theme.hilight, lineWidth: 1).rotationEffect(.degrees(180))
                    innerTopLeft.stroke(Theme.dkShadow, lineWidth: 1)
                    innerTopLeft.stroke(Theme.light, lineWidth: 1).rotationEffect(.degrees(180))
                    if isOn { Circle().fill(Theme.text).frame(width: 4, height: 4) }
                }
                .frame(width: 12, height: 12)
                Text(title).font(Theme.chrome).foregroundStyle(Theme.text)
                    .multilineTextAlignment(.leading)
            }
            .padding(.vertical, 4)
            #if os(iOS)
            .frame(minHeight: 44, alignment: .leading)
            #endif
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isOn ? [.isSelected, .isButton] : .isButton)
    }
}

/// A sunken white text field.
struct Field98<Field: View>: View {
    @ViewBuilder var field: Field

    var body: some View {
        field
            .textFieldStyle(.plain)
            .font(Theme.chat)
            .foregroundStyle(Theme.text)
            .padding(.horizontal, 6)
            #if os(iOS)
            .padding(.vertical, 9)
            #else
            .padding(.vertical, 4)
            #endif
            .background(Theme.window)
            .overlay(Bevel(style: .sunken))
    }
}

/// A tooltip-yellow note with a 1pt black border.
struct Tooltip98: View {
    var text: String

    var body: some View {
        Text(text)
            .font(Theme.chrome)
            .foregroundStyle(Theme.tooltipText)
            .multilineTextAlignment(.leading)
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .background(Theme.tooltip)
            .overlay(Rectangle().stroke(Color.black, lineWidth: 1))
    }
}

/// A horizontal etched separator.
struct Separator98: View {
    var body: some View {
        VStack(spacing: 0) {
            Theme.shadow.frame(height: 1)
            Theme.hilight.frame(height: 1)
        }
        .accessibilityHidden(true)
    }
}

extension View {
    /// The navigation title, drawn as a Win98 active title bar on iOS: the
    /// navy → blue gradient under a transparent bar (SwiftUI's
    /// toolbarBackground only takes a flat color) and a bold white title.
    @ViewBuilder
    func win98NavigationBar(_ title: String, subtitle: String? = nil) -> some View {
        #if os(iOS)
        self
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            // The status bar is always white (Info-iOS.plist: every screen
            // has a navy title bar or the teal desktop on top).
            .toolbarBackgroundVisibility(.hidden, for: .navigationBar)
            .toolbar {
                ToolbarItem(placement: .principal) {
                    VStack(spacing: 0) {
                        Text(title)
                            .font(Theme.title)
                            .foregroundStyle(Theme.titleText)
                            .lineLimit(1)
                            .accessibilityAddTraits(.isHeader)
                        if let subtitle {
                            Text(subtitle)
                                .font(Theme.small)
                                .foregroundStyle(Theme.titleText.opacity(0.8))
                                .lineLimit(1)
                        }
                    }
                }
            }
            .overlay(alignment: .top) {
                // From the top of the screen down to where the content starts,
                // edge to edge (in landscape the content sits between the
                // side insets, which are the same on both sides).
                GeometryReader { geo in
                    let frame = geo.frame(in: .global)
                    let top = max(0, frame.minY)
                    let side = max(0, frame.minX)
                    LinearGradient(colors: [Theme.titleStart, Theme.titleEnd], startPoint: .leading, endPoint: .trailing)
                        .overlay(alignment: .bottom) { Color.black.frame(height: 1) }
                        .frame(width: geo.size.width + side * 2, height: top)
                        .offset(x: -side, y: -top)
                }
                .allowsHitTesting(false)
                .accessibilityHidden(true)
            }
        #else
        self.navigationTitle(title)
        #endif
    }

    /// A Win98 sheet's chrome and its ✕ (and ✓). iPhone: the navy
    /// navigation bar with them as bar buttons. Mac: a Win98 window — a
    /// title bar with the ✕ as its close box, the content, and an OK row
    /// with the ✓ — instead of the system's grey title and bottom bar.
    func win98Sheet<Close: View>(_ title: String, subtitle: String? = nil, @ViewBuilder close: () -> Close) -> some View {
        win98Sheet(title, subtitle: subtitle, close: close, done: { EmptyView() })
    }

    @ViewBuilder
    func win98Sheet<Close: View, Done: View>(_ title: String, subtitle: String? = nil, @ViewBuilder close: () -> Close,
                                             @ViewBuilder done: () -> Done) -> some View {
        #if os(iOS)
        let done = done()
        self
            .win98NavigationBar(title, subtitle: subtitle)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { close() }
                    .win98Item()
                if !(done is EmptyView) {
                    ToolbarItem(placement: .confirmationAction) { done }
                        .win98Item()
                }
            }
            // Square, like a window (iOS 26's rounded sheet clipped the title bar).
            .presentationCornerRadius(0)
        #else
        let done = done()
        VStack(spacing: 0) {
            TitleBar(title: subtitle.map { "\(title) — \($0)" } ?? title) {
                close().environment(\.inTitleBar, true)
            }
            self.frame(maxWidth: .infinity, maxHeight: .infinity)
            if !(done is EmptyView) {
                HStack {
                    Spacer()
                    done
                }
                .padding(8)
            }
        }
        .padding(3)
        .background(Theme.face)
        .overlay(Bevel(style: .raised))
        .toolbar(.hidden)
        #endif
    }
}

extension EnvironmentValues {
    /// Mac: the sheet's ✕ is drawn as a title bar's close box.
    @Entry var inTitleBar = false
}

#if os(iOS)
/// iOS navigation bars: transparent (the gradient is drawn by
/// `win98NavigationBar()`), bold white title and white bar buttons.
enum NavigationBarStyle {
    static func install() {
        let appearance = UINavigationBarAppearance()
        appearance.configureWithTransparentBackground()
        appearance.titleTextAttributes = [.foregroundColor: UIColor.white, .font: UIFont.boldSystemFont(ofSize: 15)]
        appearance.largeTitleTextAttributes = [.foregroundColor: UIColor.white]
        let bar = UINavigationBar.appearance()
        bar.standardAppearance = appearance
        bar.scrollEdgeAppearance = appearance
        bar.compactAppearance = appearance
        bar.compactScrollEdgeAppearance = appearance
        bar.tintColor = .white
    }
}
#endif

extension EnvironmentValues {
    /// The app's color scheme, captured at the root, so toolbar content
    /// (which the dark title bar switches to dark) can draw in the real one.
    @Entry var appColorScheme: ColorScheme? = nil
}

extension View {
    /// Inside a title bar: draw with the app's scheme, not the bar's.
    func appScheme() -> some View { modifier(AppSchemeModifier()) }
}

private struct AppSchemeModifier: ViewModifier {
    @AppStorage(Appearance.storageKey) private var appearance: Appearance = .charcoal

    /// The chosen appearance, or the system's (bar content is forced dark,
    /// so the environment can't tell us).
    private var scheme: ColorScheme {
        if let chosen = appearance.colorScheme { return chosen }
        #if os(iOS)
        let scene = UIApplication.shared.connectedScenes.first as? UIWindowScene
        return scene?.traitCollection.userInterfaceStyle == .dark ? .dark : .light
        #else
        return NSApp.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? .dark : .light
        #endif
    }

    func body(content: Content) -> some View {
        content.environment(\.colorScheme, scheme)
    }
}

/// The one "…" menu of a screen, as a Win98 button.
struct ToolbarMenuButton<Items: View>: View {
    @ViewBuilder var items: Items

    var body: some View {
        Menu { items } label: {
            Image(systemName: "ellipsis").font(.system(size: 13, weight: .bold))
        }
        .menuStyle(.button)
        .buttonStyle(.win98Compact)
        .accessibilityLabel("More")
        .appScheme()
    }
}

/// A date or time in a sunken white field (Win98's date picker look); taps
/// open the system's picker (`DrawnDatePicker`).
struct DateField98: View {
    var selection: Binding<Date>
    var range: ClosedRange<Date>? = nil
    var components: DatePickerComponents

    var body: some View {
        DrawnDatePicker(selection: selection, range: range, components: components) { text in
            Text(text)
                .font(Theme.chat)
                .foregroundStyle(Theme.text)
                .lineLimit(1)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Theme.window)
                .overlay(Bevel(style: .sunken))
        }
    }
}

/// A Win98 check box: a sunken white square with a check when on.
struct Check98: View {
    var title: String
    @Binding var isOn: Bool
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        Button { isOn.toggle() } label: {
            HStack(spacing: 6) {
                ZStack {
                    Theme.window
                    if isOn {
                        Image(systemName: "checkmark").font(.system(size: 9, weight: .black)).foregroundStyle(Theme.text)
                    }
                }
                .frame(width: 13, height: 13)
                .overlay(Bevel(style: .sunken))
                Text(title).font(Theme.chrome).foregroundStyle(isEnabled ? Theme.text : Theme.grayText)
                    .multilineTextAlignment(.leading)
            }
            .padding(.vertical, 3)
            #if os(iOS)
            .frame(minHeight: 44, alignment: .leading)
            #endif
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
        .accessibilityAddTraits(isOn ? [.isSelected, .isButton] : .isButton)
    }
}
