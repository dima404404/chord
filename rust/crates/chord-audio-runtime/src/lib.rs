//! Native audio operations share a Windows COM apartment across callers.
#![deny(unsafe_code)]

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;

/// Executes a native audio operation. Keep device and stream handles inside the operation.
pub fn with_audio_backend<T: Send + 'static>(
    operation: impl FnOnce() -> T + Send + 'static,
) -> std::io::Result<T> {
    #[cfg(windows)]
    {
        windows::run(operation)
    }
    #[cfg(not(windows))]
    {
        Ok(operation())
    }
}
