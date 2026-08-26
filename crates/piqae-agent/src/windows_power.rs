//! Windows suspend/resume delivery for the durable runtime lifecycle.

#![allow(
    unsafe_code,
    reason = "isolated PowerRegisterSuspendResumeNotification callback boundary"
)]

use anyhow::{Result, bail};
use piqae_node_runtime::{LifecycleEvent, NodeRuntime};
use std::sync::Arc;
use tokio::sync::Notify;
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
    context: Box<CallbackContext>,
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
        let mut handle: HPOWERNOTIFY = std::ptr::null_mut();
        // SAFETY: callback context remains boxed until after successful
        // unregistration; the API copies the subscription parameters.
        let status = unsafe {
            PowerRegisterSuspendResumeNotification(
                DEVICE_NOTIFY_CALLBACK,
                (&raw mut parameters).cast::<core::ffi::c_void>() as HANDLE,
                &raw mut handle,
            )
        };
        if status != 0 || handle.is_null() {
            bail!("register Windows power lifecycle notification: Windows error {status}");
        }
        Ok(Self { handle, context })
    }
}

impl Drop for PowerLifecycleRegistration {
    fn drop(&mut self) {
        // SAFETY: handle is owned and unregistered once. A successful return
        // guarantees no future callbacks reference `context`.
        unsafe {
            PowerUnregisterSuspendResumeNotification(self.handle);
        }
        let _ = &self.context;
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
