//! Windows suspend/resume delivery for the durable runtime lifecycle.

#![allow(
    unsafe_code,
    reason = "isolated PowerRegisterSuspendResumeNotification callback boundary"
)]

use anyhow::{Result, bail};
use piqae_node_runtime::{LifecycleEvent, NodeRuntime};
use std::sync::Arc;
use tokio::sync::Notify;
use tracing::error;
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::Power::{
        DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, HPOWERNOTIFY, PowerRegisterSuspendResumeNotification,
        PowerUnregisterSuspendResumeNotification,
    },
    UI::WindowsAndMessaging::{
        DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMECRITICAL,
        PBT_APMRESUMESTANDBY, PBT_APMRESUMESUSPEND, PBT_APMSTANDBY, PBT_APMSUSPEND,
    },
};

struct CallbackContext {
    runtime: Arc<NodeRuntime>,
    wakeup: Arc<Notify>,
}

pub struct PowerLifecycleRegistration {
    handle: HPOWERNOTIFY,
    context: Option<Box<CallbackContext>>,
}

impl std::fmt::Debug for PowerLifecycleRegistration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PowerLifecycleRegistration(<registered>)")
    }
}

impl PowerLifecycleRegistration {
    pub fn register(runtime: Arc<NodeRuntime>, wakeup: Arc<Notify>) -> Result<Self> {
        let mut context = Box::new(CallbackContext { runtime, wakeup });
        let mut parameters = DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(power_callback),
            Context: (&raw mut *context).cast(),
        };
        let mut registration_handle = std::ptr::null_mut::<core::ffi::c_void>();
        // SAFETY: callback context remains boxed until after successful
        // unregistration; the API copies the subscription parameters.
        let status = unsafe {
            PowerRegisterSuspendResumeNotification(
                DEVICE_NOTIFY_CALLBACK,
                (&raw mut parameters).cast::<core::ffi::c_void>() as HANDLE,
                &raw mut registration_handle,
            )
        };
        if status != 0 || registration_handle.is_null() {
            bail!("register Windows power lifecycle notification: Windows error {status}");
        }
        let handle = registration_handle as HPOWERNOTIFY;
        Ok(Self {
            handle,
            context: Some(context),
        })
    }
}

impl Drop for PowerLifecycleRegistration {
    fn drop(&mut self) {
        // SAFETY: handle is owned and unregistered once. A successful return
        // guarantees no future callbacks reference `context`.
        let status = unsafe { PowerUnregisterSuspendResumeNotification(self.handle) };
        if let Some(context) = self.context.take() {
            release_callback_context(status, context);
        }
    }
}

fn release_callback_context<T>(unregister_status: u32, context: Box<T>) {
    if unregister_status == 0 {
        drop(context);
    } else {
        // Windows does not guarantee callbacks have stopped after a failed
        // unregister. Intentionally retain this small context for the process
        // lifetime rather than permit a callback-after-free.
        error!(
            windows_status = unregister_status,
            "Windows power callback could not be unregistered; retaining callback context"
        );
        let _ = Box::into_raw(context);
    }
}

unsafe extern "system" fn power_callback(
    context: *const core::ffi::c_void,
    event: u32,
    _setting: *const core::ffi::c_void,
) -> u32 {
    if context.is_null() {
        return 0;
    }
    // SAFETY: context points to the live CallbackContext owned by the active
    // registration and is accessed immutably by callbacks.
    let context = unsafe { &*context.cast::<CallbackContext>() };
    match event {
        PBT_APMSUSPEND | PBT_APMSTANDBY => {
            let _ = context
                .runtime
                .apply_lifecycle(LifecycleEvent::SuspendImminent);
            let _ = context.runtime.apply_lifecycle(LifecycleEvent::Sleeping);
        }
        PBT_APMRESUMEAUTOMATIC
        | PBT_APMRESUMECRITICAL
        | PBT_APMRESUMESTANDBY
        | PBT_APMRESUMESUSPEND => {
            let _ = context.runtime.apply_lifecycle(LifecycleEvent::Woke);
            context.wakeup.notify_waiters();
        }
        _ => {}
    }
    0
}

#[cfg(test)]
mod tests {
    use super::release_callback_context;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct DropProbe(Arc<AtomicUsize>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn failed_unregister_retains_callback_context() {
        let dropped = Arc::new(AtomicUsize::new(0));
        release_callback_context(5, Box::new(DropProbe(Arc::clone(&dropped))));
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        release_callback_context(0, Box::new(DropProbe(Arc::clone(&dropped))));
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }
}
