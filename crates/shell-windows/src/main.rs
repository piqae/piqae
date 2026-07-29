#[cfg(windows)]
mod windows_shell {
    use std::mem::size_of;
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NOTIFYICONDATAW, Shell_NotifyIconW},
            WindowsAndMessaging::{
                CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW,
                IDI_APPLICATION, LoadIconW, MSG, PostQuitMessage, RegisterClassW, TranslateMessage,
                WM_APP, WM_DESTROY, WNDCLASSW, WS_OVERLAPPED,
            },
        },
    };

    const TRAY_MESSAGE: u32 = WM_APP + 1;

    pub fn run() -> Result<(), String> {
        // SAFETY: Every Win32 structure is zero-initialized and sized before
        // use, pointers reference process-lifetime UTF-16 buffers, and handles
        // are checked before subsequent API calls.
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            if instance.is_null() {
                return Err("GetModuleHandleW failed".into());
            }
            let class_name = wide("SpoolShellWindow");
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class_name.as_ptr(),
                ..std::mem::zeroed()
            };
            if RegisterClassW(&class) == 0 {
                return Err("RegisterClassW failed".into());
            }
            let window = CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                WS_OVERLAPPED,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if window.is_null() {
                return Err("CreateWindowExW failed".into());
            }
            add_icon(window)?;
            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        Ok(())
    }

    fn add_icon(window: HWND) -> Result<(), String> {
        // SAFETY: The notification structure is initialized and sized before
        // the Win32 call, `window` is a live HWND created by `run`, and the
        // icon resource is a system-owned process-lifetime handle.
        unsafe {
            let mut data: NOTIFYICONDATAW = std::mem::zeroed();
            data.cbSize = u32::try_from(size_of::<NOTIFYICONDATAW>())
                .map_err(|_| "NOTIFYICONDATAW size overflow")?;
            data.hWnd = window;
            data.uID = 1;
            data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            data.uCallbackMessage = TRAY_MESSAGE;
            data.hIcon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);
            let tip = wide("Spool agent");
            for (target, source) in data.szTip.iter_mut().zip(tip) {
                *target = source;
            }
            if Shell_NotifyIconW(NIM_ADD, &data) == 0 {
                return Err("Shell_NotifyIconW failed".into());
            }
        }
        Ok(())
    }

    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_DESTROY {
            // SAFETY: Posting a quit message does not dereference any caller
            // memory and is valid from the window thread.
            unsafe { PostQuitMessage(0) };
            return 0;
        }
        // SAFETY: The arguments are forwarded unchanged from the operating
        // system's window procedure callback.
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(windows)]
fn main() {
    if let Err(error) = windows_shell::run() {
        eprintln!("Spool Windows shell failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("spool-shell-windows is only available on Windows");
}
