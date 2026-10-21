import AVFoundation
import Foundation
#if os(macOS)
import CoreAudio
#endif

/// Settings › Voice › Mic volume: the mic's own input level, where the
/// device lets an app set it. Not a gain in Yap's audio (the WebRTC SDK has
/// no hook on the send path, TD9), so it's the hardware's level:
///
/// - iPhone/iPad: the audio session's input gain, 0–100 %, on routes that
///   allow it (`isInputGainSettable`). Untouched until you move it; then
///   put back at the start of each call and whenever the route changes.
/// - Mac: the default input device's volume, the same as System Settings ›
///   Sound › Input (so it's the Mac's level, for every app), where the
///   device has one.
@MainActor
enum MicVolume {
    #if os(iOS)
    static let key = "yap.micGain"

    /// The level you picked (0…1); nil until you've moved it (iOS's own level).
    static var chosen: Float? {
        get { UserDefaults.standard.object(forKey: key).flatMap { ($0 as? NSNumber)?.floatValue }.map(clamp) }
        set {
            if let newValue { UserDefaults.standard.set(clamp(newValue), forKey: key) } else { UserDefaults.standard.removeObject(forKey: key) }
        }
    }

    /// This route lets Yap set the mic's level (only known while a call holds the mic).
    static var settable: Bool { AVAudioSession.sharedInstance().isInputGainSettable }

    /// The level the session has now.
    static var current: Float { AVAudioSession.sharedInstance().inputGain }

    /// Put the level you picked on the mic (in a call; nothing if you never picked one).
    static func apply() {
        guard let chosen, settable else { return }
        try? AVAudioSession.sharedInstance().setInputGain(chosen)
    }

    /// The slider moved.
    static func pick(_ level: Float) {
        chosen = level
        apply()
    }
    #else
    /// The default input device, if its volume can be set.
    static var settable: Bool {
        guard let device = defaultInput else { return false }
        return !settableElements(of: device).isEmpty
    }

    /// The input device's volume now (0…1), nil without one.
    static var current: Float? {
        guard let device = defaultInput, let element = settableElements(of: device).first else { return nil }
        var address = volumeAddress(element)
        var value: Float32 = 0
        var size = UInt32(MemoryLayout<Float32>.size)
        guard AudioObjectGetPropertyData(device, &address, 0, nil, &size, &value) == noErr else { return nil }
        return clamp(value)
    }

    /// The input device's name ("MacBook Pro Microphone").
    static var deviceName: String? {
        guard let device = defaultInput else { return nil }
        var address = AudioObjectPropertyAddress(mSelector: kAudioObjectPropertyName, mScope: kAudioObjectPropertyScopeGlobal,
                                                 mElement: kAudioObjectPropertyElementMain)
        var name: Unmanaged<CFString>?
        var size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
        guard AudioObjectGetPropertyData(device, &address, 0, nil, &size, &name) == noErr, let name else { return nil }
        return name.takeRetainedValue() as String
    }

    /// The slider moved: every channel of the device gets the level.
    static func pick(_ level: Float) {
        guard let device = defaultInput else { return }
        var value = Float32(clamp(level))
        for element in settableElements(of: device) {
            var address = volumeAddress(element)
            AudioObjectSetPropertyData(device, &address, 0, nil, UInt32(MemoryLayout<Float32>.size), &value)
        }
    }

    private static var defaultInput: AudioObjectID? {
        var address = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDefaultInputDevice, mScope: kAudioObjectPropertyScopeGlobal,
                                                 mElement: kAudioObjectPropertyElementMain)
        var device = AudioObjectID(kAudioObjectUnknown)
        var size = UInt32(MemoryLayout<AudioObjectID>.size)
        guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &device) == noErr,
              device != kAudioObjectUnknown else { return nil }
        return device
    }

    private static func volumeAddress(_ element: AudioObjectPropertyElement) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyVolumeScalar, mScope: kAudioDevicePropertyScopeInput, mElement: element)
    }

    /// The device's own volume (element 0), or else each channel's (1, 2, …).
    private static func settableElements(of device: AudioObjectID) -> [AudioObjectPropertyElement] {
        func settable(_ element: AudioObjectPropertyElement) -> Bool {
            var address = volumeAddress(element)
            guard AudioObjectHasProperty(device, &address) else { return false }
            var can: DarwinBoolean = false
            return AudioObjectIsPropertySettable(device, &address, &can) == noErr && can.boolValue
        }
        if settable(kAudioObjectPropertyElementMain) { return [kAudioObjectPropertyElementMain] }
        return (1...8).map(AudioObjectPropertyElement.init).filter(settable)
    }
    #endif

    private static func clamp(_ value: Float) -> Float { min(1, max(0, value)) }
}
