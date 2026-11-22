#![cfg(windows)]

use chord_platform::audio_engine::NativeAudioEngine;
use cpal::traits::HostTrait;

#[test]
fn audio_device_enumeration_survives_the_first_caller_exiting() {
    for _ in 0..8 {
        let inventory = std::thread::spawn(|| NativeAudioEngine::new().enumerate_devices())
            .join()
            .expect("audio enumeration must complete on each worker");
        assert!(!inventory.inputs.is_empty());
        assert!(!inventory.outputs.is_empty());
        assert!(!inventory.default_input.is_empty());
        assert!(!inventory.default_output.is_empty());

        // Read the real backend after the previous caller exits, even with no devices attached.
        std::thread::spawn(|| {
            chord_audio_runtime::with_audio_backend(|| {
                cpal::default_host()
                    .devices()
                    .map(|devices| devices.count())
            })
        })
        .join()
        .expect("the backend worker must finish")
        .expect("the audio apartment must initialize")
        .expect("WASAPI must enumerate endpoints");
    }
}
