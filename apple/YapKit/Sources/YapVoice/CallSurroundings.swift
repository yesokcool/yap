import AVFoundation
import Foundation
import Network
#if os(iOS)
import UIKit
#else
import CoreAudio
#endif

/// The device and OS, for the call report: "iPhone17,1, iOS 26.2 (23C55)".
enum DeviceInfo {
    @MainActor static var description: String {
        let v = ProcessInfo.processInfo.operatingSystemVersion
        let version = v.patchVersion > 0 ? "\(v.majorVersion).\(v.minorVersion).\(v.patchVersion)" : "\(v.majorVersion).\(v.minorVersion)"
        let full = ProcessInfo.processInfo.operatingSystemVersionString
        let build = full.range(of: "Build ").map { String(full[$0.upperBound...].prefix { $0 != ")" }) }
        #if os(iOS)
        let os = UIDevice.current.userInterfaceIdiom == .pad ? "iPadOS" : "iOS"
        let simulated = ProcessInfo.processInfo.environment["SIMULATOR_MODEL_IDENTIFIER"]
        let model = simulated.map { "\($0) (simulator)" } ?? sysctl("hw.machine") ?? "iPhone"
        #else
        let os = "macOS"
        let model = sysctl("hw.model") ?? "Mac"
        #endif
        return "\(model), \(os) \(version)\(build.map { " (\($0))" } ?? "")"
    }

    private static func sysctl(_ name: String) -> String? {
        var size = 0
        guard sysctlbyname(name, nil, &size, nil, 0) == 0, size > 0 else { return nil }
        var bytes = [CChar](repeating: 0, count: size)
        guard sysctlbyname(name, &bytes, &size, nil, 0) == 0 else { return nil }
        return String(decoding: bytes.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
    }
}

/// The audio devices or route, as the OS names them: "in iPhone Microphone
/// (MicrophoneBuiltIn), out Speaker (Speaker)".
enum AudioRouteInfo {
    @MainActor static func describe() -> String? {
        #if os(iOS)
        let route = AVAudioSession.sharedInstance().currentRoute
        func ports(_ list: [AVAudioSessionPortDescription]) -> String {
            list.isEmpty ? "none" : list.map { "\($0.portName) (\($0.portType.rawValue))" }.joined(separator: " + ")
        }
        return "in \(ports(route.inputs)), out \(ports(route.outputs))"
        #else
        let input = defaultDeviceName(kAudioHardwarePropertyDefaultInputDevice) ?? "none"
        let output = defaultDeviceName(kAudioHardwarePropertyDefaultOutputDevice) ?? "none"
        return "in \(input), out \(output) (the system's default devices)"
        #endif
    }

    #if os(iOS)
    static func reason(_ reason: AVAudioSession.RouteChangeReason) -> String {
        switch reason {
        case .newDeviceAvailable: "a device was connected"
        case .oldDeviceUnavailable: "a device went away"
        case .categoryChange: "the audio category changed"
        case .override: "the output was switched"
        case .wakeFromSleep: "woke from sleep"
        case .noSuitableRouteForCategory: "no suitable route"
        case .routeConfigurationChange: "the route's configuration changed"
        default: "unknown reason"
        }
    }
    #else
    private static func defaultDeviceName(_ selector: AudioObjectPropertySelector) -> String? {
        var device = AudioObjectID(kAudioObjectUnknown)
        var size = UInt32(MemoryLayout<AudioObjectID>.size)
        var address = AudioObjectPropertyAddress(mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &device) == noErr,
              device != kAudioObjectUnknown else { return nil }
        var name: Unmanaged<CFString>?
        size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
        address.mSelector = kAudioObjectPropertyName
        guard AudioObjectGetPropertyData(device, &address, 0, nil, &size, &name) == noErr, let name else { return nil }
        return name.takeRetainedValue() as String
    }
    #endif
}

/// Wi‑Fi ↔ cellular while in a call (the call report's timeline). No
/// interface names or addresses: the kind of network only.
final class NetworkWatch: @unchecked Sendable {
    private let monitor = NWPathMonitor()

    init(onChange: @escaping @MainActor (String) -> Void) {
        monitor.pathUpdateHandler = { path in
            let kind = Self.kind(path)
            DispatchQueue.main.async { MainActor.assumeIsolated { onChange(kind) } }
        }
        monitor.start(queue: DispatchQueue(label: "yap.call.network"))
    }

    static func kind(_ path: NWPath) -> String {
        guard path.status == .satisfied else { return "no network" }
        if path.usesInterfaceType(.wifi) { return "Wi‑Fi" }
        if path.usesInterfaceType(.cellular) { return "cellular" }
        if path.usesInterfaceType(.wiredEthernet) { return "wired" }
        return "other network"
    }

    func cancel() { monitor.cancel() }
}

/// Apple's mic modes (Voice Isolation, Wide Spectrum): the person picks
/// them in Control Center; an app can only read them and open the picker
/// (`AVCaptureDevice.showSystemUserInterface(.microphoneModes)`). They
/// apply to apps that use Apple's voice-processing I/O unit (AUVoiceIO), so
/// on iPhone and iPad, whose calls run through it (WebRTC's audio unit,
/// in `.voiceChat` and, with high-quality headphone audio, `.default`
/// mode), but not on the Mac, where WebRTC's Core Audio device doesn't.
public enum MicModes {
    /// Shown at all on this platform.
    public static var offered: Bool {
        #if os(iOS)
        true
        #else
        false
        #endif
    }

    /// What iOS uses right now (it may differ from the choice if the route can't do it).
    public static var active: String { title(AVCaptureDevice.activeMicrophoneMode) }
    /// What the person chose in Control Center.
    public static var preferred: String { title(AVCaptureDevice.preferredMicrophoneMode) }

    public static func title(_ mode: AVCaptureDevice.MicrophoneMode) -> String {
        switch mode {
        case .standard: "Standard"
        case .voiceIsolation: "Voice Isolation"
        case .wideSpectrum: "Wide Spectrum"
        @unknown default: "Standard"
        }
    }

    /// Opens Apple's own picker (Control Center's mic modes).
    @MainActor public static func showPicker() {
        AVCaptureDevice.showSystemUserInterface(.microphoneModes)
    }

    /// "Standard", or "Standard (you chose Voice Isolation)" when the route can't do the choice.
    public static var reported: String {
        let active = active, preferred = preferred
        return active == preferred ? active : "\(active) (you chose \(preferred))"
    }
}
