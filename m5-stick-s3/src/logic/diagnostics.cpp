#include "diagnostics.h"
namespace diagnostics {
const char* message(Event event) {
  switch (event) {
    case Event::Power: return "Power state changed";
    case Event::Input: return "Input action";
    case Event::Pairing: return "Pairing state changed";
    case Event::PairingRequested: return "Pairing confirmation requested on the stick";
    case Event::Link: return "Mac connection state changed";
    case Event::Device: return "ready device";
    case Event::Settings: return "Settings changed";
    case Event::MicFailed: return "Microphone failed to start";
    case Event::Mic: return "Microphone state changed";
    case Event::Prompt: return "Request shown on the stick";
    case Event::PromptAnswered: return "Request answered on the stick";
    case Event::Event: return "Event shown on the stick";
    case Event::Reply: return "Reply received";
    case Event::Mode: return "Mode changed";
    case Event::Display: return "Display state changed";
    case Event::SoundFailed: return "Speaker failed to start";
    case Event::Sound: return "Sound playback state changed";
    case Event::Ota: return "Firmware update state changed";
    case Event::OtaFailed: return "Firmware update failed or refused";
    case Event::OtaStarted: return "Firmware update started";
    case Event::OtaDone: return "Firmware update completed";
    case Event::OtaCancelled: return "Firmware update cancelled";
    case Event::Face: return "Face state changed";
    case Event::FaceFailed: return "Face operation failed or refused";
    case Event::FaceInstall: return "Face install state changed";
    case Event::StorageFailed: return "Storage failed to mount";
    case Event::Storage: return "Storage state changed";
    case Event::NetworkFailed: return "Network connection failed";
    case Event::Network: return "Network state changed";
    case Event::Ir: return "IR operation state changed";
    case Event::IrFailed: return "IR operation failed";
    case Event::Game: return "Game state changed";
    case Event::Launcher: return "Launcher state changed";
    case Event::Battery: return "Battery state changed";
    case Event::Hid: return "HID state changed";
    case Event::HidInput: return "HID input activity";
    case Event::Suppressed: return "Unstructured diagnostic payload omitted";
    default: return "Unknown diagnostic event";
  }
}
const char* name(Counter counter) {
  switch (counter) {
    case Counter::ReplyBytes: return "Reply bytes";
    case Counter::AudioFrames: return "Audio frames sent";
    case Counter::AudioDropped: return "Audio frames dropped";
    case Counter::AudioRetries: return "Audio send retries";
    case Counter::AudioPeak: return "Microphone peak";
    case Counter::ClipFrames: return "Clip frames received";
    case Counter::ClipExpectedFrames: return "Clip frames expected";
    case Counter::StorageUsedKb: return "Storage used KB";
    case Counter::StorageTotalKb: return "Storage total KB";
    case Counter::StorageMountMs: return "Storage mount ms";
    case Counter::FirmwareBytes: return "Firmware image bytes";
    case Counter::FirmwareOffset: return "Firmware offset";
    case Counter::FirmwareVerifyMs: return "Firmware verification ms";
    case Counter::FaceBytes: return "Face package bytes";
    case Counter::FaceMemoryKb: return "Face memory KB";
    case Counter::FaceLoadMs: return "Face load ms";
    case Counter::FaceCount: return "Installed face count";
    case Counter::FaceTier: return "Face rendering tier";
    case Counter::FrameMicros: return "Frame render us";
    case Counter::FrameRate: return "Frame rate";
    default: return "Unknown counter";
  }
}
}  // namespace diagnostics
