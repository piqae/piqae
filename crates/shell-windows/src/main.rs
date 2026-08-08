#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod windows_shell {
    use piqae_domain::ProfileCaptureOperation;
    use piqae_shell_windows::{
        LocalAgentClient, LocalApiConfiguration, ShellError, capture_payload, run_profile_host,
        updater::{UpdateConfiguration, WindowsUpdater},
    };
    use std::{
        cell::RefCell,
        mem::size_of,
        path::PathBuf,
        sync::{
            Arc, Mutex, OnceLock,
            atomic::{AtomicBool, AtomicIsize, Ordering},
        },
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{
                NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
                Shell_NotifyIconW, ShellExecuteW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
                DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
                IDI_APPLICATION, LoadIconW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MF_DISABLED,
                MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG, MessageBoxW, PostMessageW,
                PostQuitMessage, RegisterClassW, SW_SHOWNORMAL, SetForegroundWindow, TPM_NONOTIFY,
                TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_CLOSE,
                WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED,
            },
        },
    };

    const TRAY_MESSAGE: u32 = WM_APP + 1;
    const FIRST_ACTION_ID: i32 = 100;
    const TRAY_ICON_ID: u32 = 1;

    static SHELL_STATE: OnceLock<Mutex<ShellState>> = OnceLock::new();
    static UPDATE_CLIENT: OnceLock<Arc<LocalAgentClient>> = OnceLock::new();
    static UPDATE_WINDOW: AtomicIsize = AtomicIsize::new(0);
    static PROFILE_CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);
    thread_local! {
        static UPDATER: RefCell<Option<WindowsUpdater>> = const { RefCell::new(None) };
    }

    #[derive(Clone, Debug)]
    enum MenuAction {
        Capture {
            printer_id: String,
            profile_id: Option<String>,
            revision: Option<u64>,
            operation: ProfileCaptureOperation,
            is_default: bool,
        },
        OpenDashboard,
        CheckForUpdates,
        Refresh,
        Exit,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum UpdateAvailability {
        Disabled,
        Available,
        Unavailable,
    }

    #[derive(Debug)]
    struct ShellState {
        client: Arc<LocalAgentClient>,
        profile_host: PathBuf,
        dashboard_url: String,
        actions: Vec<MenuAction>,
        snapshot: Option<ShellSnapshot>,
        refresh_error: Option<String>,
        refresh_in_progress: bool,
        updates: UpdateAvailability,
    }

    #[derive(Clone, Debug)]
    struct ShellSnapshot {
        status: piqae_local_ipc::LocalStatus,
        printers: Vec<piqae_local_ipc::LocalPrinter>,
    }

    pub fn run() -> Result<(), String> {
        let configuration = LocalApiConfiguration::from_environment().map_err(|e| e.to_string())?;
        let client = Arc::new(LocalAgentClient::new(configuration).map_err(|e| e.to_string())?);
        let profile_host = profile_host_path()?;
        let dashboard_url = dashboard_url();
        SHELL_STATE
            .set(Mutex::new(ShellState {
                client,
                profile_host,
                dashboard_url,
                actions: Vec::new(),
                snapshot: None,
                refresh_error: None,
                refresh_in_progress: false,
                updates: UpdateAvailability::Disabled,
            }))
            .map_err(|_| "Windows shell state was already initialized".to_owned())?;

        // SAFETY: Structures are initialized with their documented sizes,
        // pointers reference live NUL-terminated UTF-16 buffers, and handles
        // are checked before subsequent Win32 calls.
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            if instance.is_null() {
                return Err("GetModuleHandleW failed".into());
            }
            let class_name = wide("PiqaeShellWindow");
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
            initialize_updater(window);
            schedule_refresh();
            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        Ok(())
    }

    fn profile_host_path() -> Result<PathBuf, String> {
        if let Some(path) =
            std::env::var_os("PIQAE_PROFILE_HOST_PATH").filter(|path| !path.is_empty())
        {
            return Ok(PathBuf::from(path));
        }
        let executable = std::env::current_exe()
            .map_err(|error| format!("Cannot find the Piqae installation directory: {error}"))?;
        let directory = executable
            .parent()
            .ok_or_else(|| "Cannot find the Piqae installation directory".to_owned())?;
        Ok(directory.join("piqae-profile-host-windows.exe"))
    }

    fn dashboard_url() -> String {
        let configured = std::env::var("PIQAE_DASHBOARD_URL").unwrap_or_default();
        let configured = configured.trim();
        if !configured.contains(['\r', '\n'])
            && (configured.starts_with("https://") || configured.starts_with("http://"))
        {
            configured.to_owned()
        } else {
            "http://127.0.0.1:5173/dashboard/local".into()
        }
    }

    fn add_icon(window: HWND) -> Result<(), String> {
        // SAFETY: The notification structure is sized before the Win32 call,
        // the window is live, and the system icon remains valid for the
        // process lifetime.
        unsafe {
            let mut data = notification_data(window)?;
            data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            data.uCallbackMessage = TRAY_MESSAGE;
            data.hIcon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);
            let tip = wide("Piqae Node");
            for (target, source) in data.szTip.iter_mut().zip(tip) {
                *target = source;
            }
            if Shell_NotifyIconW(NIM_ADD, &data) == 0 {
                return Err("Shell_NotifyIconW failed".into());
            }
        }
        Ok(())
    }

    fn notification_data(window: HWND) -> Result<NOTIFYICONDATAW, String> {
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = u32::try_from(size_of::<NOTIFYICONDATAW>())
            .map_err(|_| "NOTIFYICONDATAW size overflow")?;
        data.hWnd = window;
        data.uID = TRAY_ICON_ID;
        Ok(data)
    }

    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == TRAY_MESSAGE {
            let event = u32::try_from(lparam).unwrap_or_default();
            if matches!(event, WM_LBUTTONUP | WM_RBUTTONUP | WM_CONTEXTMENU) {
                schedule_refresh();
                show_menu(window);
                return 0;
            }
        }
        if message == WM_DESTROY {
            UPDATER.with(|updater| drop(updater.borrow_mut().take()));
            UPDATE_WINDOW.store(0, Ordering::Release);
            if let Ok(data) = notification_data(window) {
                // SAFETY: Removing the icon uses the same live window and icon
                // ID supplied to NIM_ADD.
                unsafe {
                    Shell_NotifyIconW(NIM_DELETE, &data);
                }
            }
            // SAFETY: Posting a quit message is valid from the window thread.
            unsafe { PostQuitMessage(0) };
            return 0;
        }
        // SAFETY: Arguments are forwarded unchanged from Windows.
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    fn show_menu(window: HWND) {
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return;
        }

        let actions = build_menu(menu);
        if let Some(state) = SHELL_STATE.get() {
            if let Ok(mut state) = state.lock() {
                state.actions = actions;
            }
        }

        let mut point = POINT { x: 0, y: 0 };
        // SAFETY: Menu and window handles are live and POINT is writable.
        let command = unsafe {
            GetCursorPos(&mut point);
            SetForegroundWindow(window);
            TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                0,
                window,
                std::ptr::null(),
            )
        };
        unsafe {
            DestroyMenu(menu);
        }
        if command >= FIRST_ACTION_ID {
            if let Ok(index) = u32::try_from(command - FIRST_ACTION_ID) {
                execute_action(window, index);
            }
        }
    }

    fn build_menu(menu: *mut std::ffi::c_void) -> Vec<MenuAction> {
        let mut actions = Vec::new();
        let Some(state_lock) = SHELL_STATE.get() else {
            return actions;
        };
        let Ok(state) = state_lock.lock() else {
            return actions;
        };
        match state.snapshot.as_ref() {
            Some(ShellSnapshot { status, printers }) => {
                let workspace = status.workspace_name.as_deref().unwrap_or("Local node");
                append_disabled(
                    menu,
                    &format!(
                        "Piqae — {workspace} · {}",
                        connection_label(status.connection)
                    ),
                );
                append_separator(menu);
                if printers.is_empty() {
                    append_disabled(menu, "No printers found");
                }
                for printer in printers {
                    let submenu = unsafe { CreatePopupMenu() };
                    if submenu.is_null() {
                        continue;
                    }
                    append_disabled(submenu, &format!("Status: {}", printer.state));
                    append_separator(submenu);
                    append_action(
                        submenu,
                        "Add native profile…",
                        MenuAction::Capture {
                            printer_id: printer.printer_id.clone(),
                            profile_id: None,
                            revision: None,
                            operation: ProfileCaptureOperation::Create,
                            is_default: false,
                        },
                        &mut actions,
                    );
                    if !printer.profiles.is_empty() {
                        append_separator(submenu);
                        for profile in &printer.profiles {
                            let profile_menu = unsafe { CreatePopupMenu() };
                            if profile_menu.is_null() {
                                continue;
                            }
                            append_disabled(
                                profile_menu,
                                &format!("Revision {} · {:?}", profile.revision, profile.status),
                            );
                            append_action(
                                profile_menu,
                                "Edit driver settings…",
                                MenuAction::Capture {
                                    printer_id: printer.printer_id.clone(),
                                    profile_id: Some(profile.profile_id.clone()),
                                    revision: Some(profile.revision),
                                    operation: ProfileCaptureOperation::Edit,
                                    is_default: profile.is_default,
                                },
                                &mut actions,
                            );
                            append_action(
                                profile_menu,
                                "Clone as new profile…",
                                MenuAction::Capture {
                                    printer_id: printer.printer_id.clone(),
                                    profile_id: Some(profile.profile_id.clone()),
                                    revision: Some(profile.revision),
                                    operation: ProfileCaptureOperation::Clone,
                                    is_default: false,
                                },
                                &mut actions,
                            );
                            append_submenu(submenu, profile_menu, &profile.name);
                        }
                    }
                    append_submenu(menu, submenu, &printer.name);
                }
            }
            None => {
                append_disabled(menu, "Piqae — Connecting…");
                if let Some(error) = state.refresh_error.as_deref() {
                    append_disabled(menu, error);
                } else {
                    append_disabled(menu, "Checking the local agent");
                }
            }
        }
        drop(state);
        append_separator(menu);
        append_action(menu, "Open Piqae", MenuAction::OpenDashboard, &mut actions);
        match SHELL_STATE
            .get()
            .and_then(|state| state.lock().ok())
            .map(|state| state.updates)
        {
            Some(UpdateAvailability::Available) => append_action(
                menu,
                "Check for Piqae updates…",
                MenuAction::CheckForUpdates,
                &mut actions,
            ),
            Some(UpdateAvailability::Unavailable) => {
                append_disabled(menu, "Updates unavailable in this build");
            }
            _ => {}
        }
        append_action(menu, "Refresh", MenuAction::Refresh, &mut actions);
        append_separator(menu);
        append_action(menu, "Quit Piqae Node", MenuAction::Exit, &mut actions);
        actions
    }

    fn execute_action(window: HWND, index: u32) {
        let action = SHELL_STATE
            .get()
            .and_then(|state| state.lock().ok())
            .and_then(|state| state.actions.get(index as usize).cloned());
        match action {
            Some(MenuAction::Capture {
                printer_id,
                profile_id,
                revision,
                operation,
                is_default,
            }) => capture_profile(
                window,
                &printer_id,
                profile_id.as_deref(),
                revision,
                operation,
                is_default,
            ),
            Some(MenuAction::OpenDashboard) => open_dashboard(window),
            Some(MenuAction::CheckForUpdates) => {
                UPDATER.with(|updater| {
                    if let Some(updater) = updater.borrow().as_ref() {
                        updater.check_with_ui();
                    }
                });
            }
            Some(MenuAction::Refresh) => schedule_refresh(),
            Some(MenuAction::Exit) => unsafe {
                DestroyWindow(window);
            },
            None => {}
        }
    }

    fn capture_profile(
        window: HWND,
        printer_id: &str,
        profile_id: Option<&str>,
        revision: Option<u64>,
        operation: ProfileCaptureOperation,
        is_default: bool,
    ) {
        PROFILE_CAPTURE_ACTIVE.store(true, Ordering::Release);
        let result = (|| {
            let state_lock = SHELL_STATE
                .get()
                .ok_or_else(|| ShellError::Configuration("shell is not initialized".into()))?;
            let (client, profile_host) = {
                let state = state_lock
                    .lock()
                    .map_err(|_| ShellError::Configuration("shell state is unavailable".into()))?;
                (Arc::clone(&state.client), state.profile_host.clone())
            };
            let session =
                client.begin_profile_capture(printer_id, operation, profile_id, revision)?;
            let captured = match run_profile_host(&profile_host, &session, Some(window as isize)) {
                Ok(Some(captured)) => captured,
                Ok(None) => {
                    client.cancel_profile_capture(&session);
                    return Ok(None);
                }
                Err(error) => {
                    client.cancel_profile_capture(&session);
                    return Err(error);
                }
            };
            let payload = capture_payload(&session, &captured, is_default)?;
            match client.complete_profile_capture(&session, &payload) {
                Ok(profile) => Ok(Some(profile)),
                Err(error) => {
                    client.cancel_profile_capture(&session);
                    Err(error)
                }
            }
        })();
        PROFILE_CAPTURE_ACTIVE.store(false, Ordering::Release);

        match result {
            Ok(Some(profile)) => message(
                window,
                "Profile saved",
                &format!(
                    "{} is available as revision {}.",
                    profile.name, profile.revision
                ),
                MB_ICONINFORMATION,
            ),
            Ok(None) => {}
            Err(error) => message(
                window,
                "Piqae could not save the profile",
                &compact_error(&error),
                MB_ICONERROR,
            ),
        }
        schedule_refresh();
    }

    fn initialize_updater(window: HWND) {
        let requested = std::env::var("PIQAE_UPDATE_POLICY")
            .is_ok_and(|policy| matches!(policy.trim(), "notify" | "automatic"));
        let configuration = match UpdateConfiguration::from_environment() {
            Ok(Some(configuration)) => configuration,
            Ok(None) => return,
            Err(_) => {
                set_update_availability(UpdateAvailability::Unavailable);
                return;
            }
        };
        let executable = match std::env::current_exe() {
            Ok(executable) => executable,
            Err(_) => {
                set_update_availability(UpdateAvailability::Unavailable);
                return;
            }
        };
        let client = SHELL_STATE
            .get()
            .and_then(|state| state.lock().ok())
            .map(|state| Arc::clone(&state.client));
        let Some(client) = client else {
            set_update_availability(UpdateAvailability::Unavailable);
            return;
        };
        if UPDATE_CLIENT.set(client).is_err() {
            set_update_availability(UpdateAvailability::Unavailable);
            return;
        }
        UPDATE_WINDOW.store(window as isize, Ordering::Release);
        match WindowsUpdater::initialize(
            &configuration,
            &executable,
            updater_can_shutdown,
            updater_shutdown_requested,
        ) {
            Ok(updater) => {
                UPDATER.with(|slot| {
                    *slot.borrow_mut() = Some(updater);
                });
                set_update_availability(UpdateAvailability::Available);
            }
            Err(_) if requested => set_update_availability(UpdateAvailability::Unavailable),
            Err(_) => {}
        }
    }

    fn set_update_availability(availability: UpdateAvailability) {
        if let Some(state) = SHELL_STATE.get() {
            if let Ok(mut state) = state.lock() {
                state.updates = availability;
            }
        }
    }

    unsafe extern "C" fn updater_can_shutdown() -> i32 {
        if PROFILE_CAPTURE_ACTIVE.load(Ordering::Acquire) {
            return 0;
        }
        let Some(client) = UPDATE_CLIENT.get() else {
            return 0;
        };
        match client.status() {
            Ok(status) if status.paused && status.active_jobs == 0 => 1,
            _ => 0,
        }
    }

    unsafe extern "C" fn updater_shutdown_requested() {
        let window = UPDATE_WINDOW.load(Ordering::Acquire) as HWND;
        if !window.is_null() {
            // SAFETY: The handle belongs to this process. Posting WM_CLOSE is
            // asynchronous and lets the UI thread perform normal cleanup.
            unsafe {
                PostMessageW(window, WM_CLOSE, 0, 0);
            }
        }
    }

    fn schedule_refresh() {
        let Some(state_lock) = SHELL_STATE.get() else {
            return;
        };
        let client = {
            let Ok(mut state) = state_lock.lock() else {
                return;
            };
            if state.refresh_in_progress {
                return;
            }
            state.refresh_in_progress = true;
            Arc::clone(&state.client)
        };
        std::thread::spawn(move || {
            let refreshed = client.status().and_then(|status| {
                client
                    .printers()
                    .map(|printers| ShellSnapshot { status, printers })
            });
            let Some(state_lock) = SHELL_STATE.get() else {
                return;
            };
            let Ok(mut state) = state_lock.lock() else {
                return;
            };
            state.refresh_in_progress = false;
            match refreshed {
                Ok(snapshot) => {
                    state.snapshot = Some(snapshot);
                    state.refresh_error = None;
                }
                Err(error) => {
                    state.refresh_error = Some(compact_error(&error));
                }
            }
        });
    }

    fn open_dashboard(window: HWND) {
        let url = SHELL_STATE
            .get()
            .and_then(|state| state.lock().ok())
            .map(|state| state.dashboard_url.clone());
        if let Some(url) = url {
            let operation = wide("open");
            let url = wide(&url);
            // SAFETY: Strings are NUL-terminated and the owner window is live.
            unsafe {
                ShellExecuteW(
                    window,
                    operation.as_ptr(),
                    url.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOWNORMAL,
                );
            }
        }
    }

    fn append_action(
        menu: *mut std::ffi::c_void,
        label: &str,
        action: MenuAction,
        actions: &mut Vec<MenuAction>,
    ) {
        let Ok(offset) = i32::try_from(actions.len()) else {
            return;
        };
        let id = FIRST_ACTION_ID + offset;
        let label = wide(label);
        unsafe {
            AppendMenuW(menu, MF_STRING, id as usize, label.as_ptr());
        }
        actions.push(action);
    }

    fn append_disabled(menu: *mut std::ffi::c_void, label: &str) {
        let label = wide(label);
        unsafe {
            AppendMenuW(menu, MF_STRING | MF_DISABLED | MF_GRAYED, 0, label.as_ptr());
        }
    }

    fn append_submenu(menu: *mut std::ffi::c_void, submenu: *mut std::ffi::c_void, label: &str) {
        let label = wide(label);
        unsafe {
            AppendMenuW(menu, MF_POPUP, submenu as usize, label.as_ptr());
        }
    }

    fn append_separator(menu: *mut std::ffi::c_void) {
        unsafe {
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        }
    }

    fn message(window: HWND, title: &str, body: &str, style: u32) {
        let title = wide(title);
        let body = wide(body);
        unsafe {
            MessageBoxW(window, body.as_ptr(), title.as_ptr(), MB_OK | style);
        }
    }

    fn compact_error(error: &ShellError) -> String {
        let message = error.to_string();
        const MAX_CHARS: usize = 180;
        if message.chars().count() <= MAX_CHARS {
            return message;
        }
        let mut compact = message.chars().take(MAX_CHARS - 1).collect::<String>();
        compact.push('…');
        compact
    }

    fn connection_label(connection: piqae_local_ipc::ConnectionState) -> &'static str {
        use piqae_local_ipc::ConnectionState::{
            Connected, Connecting, Degraded, LocalOnly, Offline, Unauthorized,
        };
        match connection {
            Connected => "Connected",
            LocalOnly => "Local only",
            Connecting => "Connecting",
            Offline => "Offline",
            Degraded => "Degraded",
            Unauthorized => "Revoked — pair this node again",
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(windows)]
fn main() {
    use std::io::Write as _;

    let mut log = std::env::var_os("PIQAE_SHELL_LOG_FILE")
        .filter(|path| !path.is_empty())
        .and_then(|path| piqae_native_logging::BoundedLogWriter::open_with_defaults(path).ok());
    if let Some(panic_log) = log.clone() {
        let default_panic_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            let mut panic_log = panic_log.clone();
            let location = panic
                .location()
                .map_or_else(|| "unknown".to_owned(), |location| location.to_string());
            let _ = writeln!(panic_log, "Piqae Windows shell panicked at {location}");
            let _ = panic_log.flush();
            default_panic_hook(panic);
        }));
    }
    if let Err(error) = windows_shell::run() {
        if let Some(log) = log.as_mut() {
            let _ = writeln!(log, "Piqae Windows shell failed: {error}");
            let _ = log.flush();
        }
        eprintln!("Piqae Node for Windows failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("piqae-shell-windows is only available on Windows");
}
