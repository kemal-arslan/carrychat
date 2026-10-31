//! The app lock (§10): asks for the device's owner, by Touch ID, Face ID or
//! the device's password, through Apple's LocalAuthentication. Android asks
//! through the app's Kotlin; other systems have no app lock yet.

#[cfg(target_vendor = "apple")]
pub use apple::{authenticate, available};

#[cfg(not(target_vendor = "apple"))]
pub fn available() -> bool {
    false
}

#[cfg(not(target_vendor = "apple"))]
pub async fn authenticate(_reason: String) -> Result<(), String> {
    Err("This system has no app lock".to_owned())
}

#[cfg(target_vendor = "apple")]
mod apple {
    use std::sync::mpsc;

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAPolicy};

    /// Whether this device can ask for its owner: it has Touch ID, Face ID
    /// or a password.
    pub fn available() -> bool {
        let context = unsafe { LAContext::new() };
        unsafe { context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthentication) }.is_ok()
    }

    /// Asks for the device's owner, saying `reason`. Resolves once they
    /// are; fails with the system's words when they are not, or cancel.
    pub async fn authenticate(reason: String) -> Result<(), String> {
        tauri::async_runtime::spawn_blocking(move || ask(&reason))
            .await
            .map_err(|error| error.to_string())?
    }

    /// Asks, and waits for the answer on this thread, which keeps the
    /// context alive until the system replies.
    fn ask(reason: &str) -> Result<(), String> {
        let (sender, receiver) = mpsc::channel();
        let context = unsafe { LAContext::new() };
        let reply = RcBlock::new(move |success: Bool, error: *mut NSError| {
            let answer = if success.as_bool() {
                Ok(())
            } else {
                Err(unsafe { error.as_ref() }
                    .map(|error| error.localizedDescription().to_string())
                    .unwrap_or_default())
            };
            let _ = sender.send(answer);
        });
        unsafe {
            context.evaluatePolicy_localizedReason_reply(
                LAPolicy::DeviceOwnerAuthentication,
                &NSString::from_str(reason),
                &reply,
            );
        }
        let answer = receiver
            .recv()
            .map_err(|_| "The system did not answer".to_owned())?;
        drop(context);
        answer
    }
}
