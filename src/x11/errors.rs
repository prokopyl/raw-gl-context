use std::ffi::CStr;
use std::fmt::{Debug, Formatter};
use x11::xlib;

use std::cell::Cell;
use std::marker::PhantomData;
use std::panic::AssertUnwindSafe;

thread_local! {
    static CURRENT_ERROR_PTR: Cell<Option<xlib::XErrorEvent>> = Cell::new(None);
}

/// A helper struct for safe X11 error handling
pub struct XErrorHandler<'a> {
    display: *mut xlib::Display,
    _phantom: PhantomData<&'a Cell<Option<xlib::XErrorEvent>>>,
}

impl<'a> XErrorHandler<'a> {
    /// Syncs and checks if any previous X11 calls returned an error
    pub fn check(&mut self) -> Result<(), XLibError> {
        // Flush all possible previous errors
        unsafe {
            xlib::XSync(self.display, 0);
        }

        let error = if let Ok(error) = CURRENT_ERROR_PTR.try_with(|e| e.replace(None)) {
            error
        } else {
            // Silence the error: this can only fail if the thread is being destroyed anyway
            return Ok(());
        };

        match error {
            None => Ok(()),
            Some(inner) => Err(XLibError { inner }),
        }
    }

    /// Sets up a temporary X11 error handler for the duration of the given closure, and allows
    /// that closure to check on the latest X11 error at any time
    pub fn handle<T, F: FnOnce(&mut XErrorHandler) -> T>(
        display: *mut xlib::Display,
        handler: F,
    ) -> T {
        unsafe extern "C" fn error_handler(
            _dpy: *mut xlib::Display,
            err: *mut xlib::XErrorEvent,
        ) -> i32 {
            // SAFETY: the error pointer should be safe to copy
            let err = *err;

            match CURRENT_ERROR_PTR.try_with(|e| e.set(Some(err))) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!(
                        "[FATAL] raw-gl-context: Failed to lock for X11 Error Handler: {e:?}"
                    );
                    1
                }
            }
        }

        // Flush all possible previous errors
        unsafe {
            xlib::XSync(display, 0);
        }

        let _ = CURRENT_ERROR_PTR.try_with(|e| e.set(None));

        let old_handler = unsafe { xlib::XSetErrorHandler(Some(error_handler)) };
        let panic_result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let mut h = XErrorHandler {
                display,
                _phantom: PhantomData,
            };
            handler(&mut h)
        }));
        // Whatever happened, restore old error handler
        unsafe { xlib::XSetErrorHandler(old_handler) };
        let _ = CURRENT_ERROR_PTR.try_with(|e| e.set(None));

        match panic_result {
            Ok(v) => v,
            Err(e) => std::panic::resume_unwind(e),
        }
    }
}

pub struct XLibError {
    inner: xlib::XErrorEvent,
}

impl XLibError {
    pub fn get_display_name(&self, buf: &mut [u8]) -> &CStr {
        unsafe {
            xlib::XGetErrorText(
                self.inner.display,
                self.inner.error_code.into(),
                buf.as_mut_ptr().cast(),
                (buf.len() - 1) as i32,
            );
        }

        *buf.last_mut().unwrap() = 0;
        // SAFETY: whatever XGetErrorText did or not, we guaranteed there is a nul byte at the end of the buffer
        unsafe { CStr::from_ptr(buf.as_mut_ptr().cast()) }
    }
}

impl Debug for XLibError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut buf = [0; 255];
        let display_name = self.get_display_name(&mut buf).to_string_lossy();

        f.debug_struct("XLibError")
            .field("error_code", &self.inner.error_code)
            .field("error_message", &display_name)
            .field("minor_code", &self.inner.minor_code)
            .field("request_code", &self.inner.request_code)
            .field("type", &self.inner.type_)
            .field("resource_id", &self.inner.resourceid)
            .field("serial", &self.inner.serial)
            .finish()
    }
}
