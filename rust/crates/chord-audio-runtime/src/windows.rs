use std::io;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;

use once_cell::sync::OnceCell;
use windows_sys::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows_sys::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

struct Apartment(PhantomData<Rc<()>>);

impl Apartment {
    fn enter(model: u32) -> Result<Self, i32> {
        // SAFETY: The reserved argument is null. Drop balances successful calls on this thread.
        let status = unsafe { CoInitializeEx(std::ptr::null(), model) };
        if status < 0 {
            Err(status)
        } else {
            Ok(Self(PhantomData))
        }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: This guard can only be dropped on the thread that initialized COM.
        unsafe { CoUninitialize() };
    }
}

type Work = Box<dyn FnOnce() + Send>;

fn apartment_error(status: i32) -> io::Error {
    io::Error::other(format!("WASAPI COM initialization failed (0x{status:08X})"))
}

fn start_worker() -> io::Result<mpsc::Sender<Work>> {
    let (work, commands) = mpsc::channel::<Work>();
    let (ready, initialized) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("chord-wasapi-mta".to_owned())
        .spawn(move || {
            let apartment = Apartment::enter(COINIT_MULTITHREADED as u32);
            if ready
                .send(apartment.as_ref().map(|_| ()).map_err(|&err| err))
                .is_ok()
            {
                if let Ok(_apartment) = apartment {
                    for operation in commands {
                        operation();
                    }
                }
            }
        })?;
    initialized
        .recv()
        .map_err(io::Error::other)?
        .map_err(apartment_error)?;
    Ok(work)
}

pub(super) fn run<T: Send + 'static>(
    operation: impl FnOnce() -> T + Send + 'static,
) -> io::Result<T> {
    // CPAL caches the WASAPI enumerator for the process lifetime. Keep its MTA alive.
    // See https://github.com/RustAudio/cpal/issues/1302.
    static WORKER: OnceCell<mpsc::Sender<Work>> = OnceCell::new();
    let worker = WORKER.get_or_try_init(start_worker)?;

    match Apartment::enter(COINIT_MULTITHREADED as u32) {
        Ok(_apartment) => Ok(operation()),
        Err(RPC_E_CHANGED_MODE) => {
            // Preserve the calling thread's apartment (for example Tauri's GUI STA).
            let (send, receive) = mpsc::sync_channel(1);
            worker
                .send(Box::new(move || {
                    let _ = send.send(catch_unwind(AssertUnwindSafe(operation)));
                }))
                .map_err(|_| io::Error::other("WASAPI worker stopped"))?;
            let result = receive.recv().map_err(io::Error::other)?;
            Ok(result.unwrap_or_else(|panic| resume_unwind(panic)))
        }
        Err(status) => Err(apartment_error(status)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Com::COINIT_APARTMENTTHREADED;

    #[test]
    fn apartment_threaded_caller_uses_the_worker() {
        thread::spawn(|| {
            let _sta = Apartment::enter(COINIT_APARTMENTTHREADED as u32).expect("STA");
            let caller = thread::current().id();
            let executing = run(|| thread::current().id()).expect("shared audio apartment");
            assert_ne!(caller, executing);
            let _still_sta = Apartment::enter(COINIT_APARTMENTTHREADED as u32)
                .expect("caller's apartment is preserved");
        })
        .join()
        .expect("STA test thread");
    }
}
