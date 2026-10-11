import SwiftUI
#if os(iOS)
import UIKit
#endif

/// The app has one look: Windows 98 / AIM, in Charcoal (dark, the default)
/// or Windows Standard (light), or following the system (`Appearance`).
/// Earlier versions also had an Apple and an Arch skin, stored per device
/// under `yap.skin`; whatever is stored there, the app opens in Windows 98
/// and keeps the light/dark choice.
enum Look {
    /// Where earlier versions kept the skin (`win98` / `apple` / `arch`).
    static let retiredSkinKey = "yap.skin"

    /// At launch: forget a retired skin choice (an Apple or Arch skin simply
    /// becomes Windows 98). Light/dark is a separate setting and stays.
    static func settle(_ defaults: UserDefaults = .standard) {
        if defaults.object(forKey: retiredSkinKey) != nil { defaults.removeObject(forKey: retiredSkinKey) }
    }
}

#if os(iOS)
extension Look {
    private static var installed = false

    /// Installs the Win98 navigation bars' UIKit appearance once, before
    /// the first bar exists (appearance proxies only reach bars created
    /// afterwards).
    static func ensureInstalled() {
        guard !installed else { return }
        installed = true
        NavigationBarStyle.install()
    }

    /// Always white: every screen has a navy title bar or the teal desktop on top.
    static let statusBarStyle: UIStatusBarStyle = .lightContent
}

/// Lets the app pick the status bar style. SwiftUI has no API for it, and
/// on the iOS 27 SDK the app-wide `UIStatusBarStyle` can't change at run
/// time. Wrapping the root hosting controller in a container works but
/// detaches SwiftUI's view from its scene (scenePhase then never reaches
/// `.active`), so instead the window's root hosting controller class gets
/// its own `preferredStatusBarStyle` (and no status-bar child) while it is
/// the root; every other controller keeps UIKit's/SwiftUI's behavior.
struct StatusBarStyler: UIViewRepresentable {
    var style: UIStatusBarStyle

    func makeUIView(context: Context) -> HookView {
        let view = HookView()
        view.style = style
        return view
    }

    func updateUIView(_ view: HookView, context: Context) {
        view.style = style
        view.apply()
    }

    final class HookView: UIView {
        var style: UIStatusBarStyle = .default

        override func didMoveToWindow() {
            super.didMoveToWindow()
            apply()
        }

        func apply() {
            guard let root = window?.rootViewController else { return }
            RootStatusBar.apply(style, to: root)
        }
    }
}

@MainActor
enum RootStatusBar {
    static var style: UIStatusBarStyle = .default
    private static var patched: Set<ObjectIdentifier> = []

    static func apply(_ style: UIStatusBarStyle, to root: UIViewController) {
        self.style = style
        if let cls = object_getClass(root), !patched.contains(ObjectIdentifier(cls)) {
            patched.insert(ObjectIdentifier(cls))
            patchStyle(cls)
            patchChild(cls)
        }
        root.setNeedsStatusBarAppearanceUpdate()
    }

    private static func isRoot(_ controller: UIViewController) -> Bool {
        controller.viewIfLoaded?.window?.rootViewController === controller
    }

    private static func patchStyle(_ cls: AnyClass) {
        let selector = #selector(getter: UIViewController.preferredStatusBarStyle)
        guard let method = class_getInstanceMethod(cls, selector) else { return }
        typealias Getter = @convention(c) (UIViewController, Selector) -> Int
        let original = unsafeBitCast(method_getImplementation(method), to: Getter.self)
        let block: @convention(block) (UIViewController) -> Int = { controller in
            MainActor.assumeIsolated {
                isRoot(controller) ? style.rawValue : original(controller, selector)
            }
        }
        class_replaceMethod(cls, selector, imp_implementationWithBlock(block), method_getTypeEncoding(method))
    }

    private static func patchChild(_ cls: AnyClass) {
        let selector = #selector(getter: UIViewController.childForStatusBarStyle)
        guard let method = class_getInstanceMethod(cls, selector) else { return }
        typealias Getter = @convention(c) (UIViewController, Selector) -> UIViewController?
        let original = unsafeBitCast(method_getImplementation(method), to: Getter.self)
        let block: @convention(block) (UIViewController) -> UIViewController? = { controller in
            MainActor.assumeIsolated {
                isRoot(controller) ? nil : original(controller, selector)
            }
        }
        class_replaceMethod(cls, selector, imp_implementationWithBlock(block), method_getTypeEncoding(method))
    }
}
#endif
