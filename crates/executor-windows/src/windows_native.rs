use crate::native_profile::{
    NativeProfileError, WindowsDriverFingerprint, WindowsNativeProfileCapture,
    validate_devmode_bytes,
};
use piqae_domain::PrinterCapabilities;
use sha2::{Digest, Sha256};
use std::{ffi::c_void, mem, ptr, slice};
use windows_sys::Win32::{
    Foundation::{ERROR_INSUFFICIENT_BUFFER, GetLastError, HANDLE, HWND},
    Graphics::{
        Gdi::{DEVMODEW, DM_IN_BUFFER, DM_IN_PROMPT, DM_OUT_BUFFER},
        Printing::{
            ClosePrinter, DRIVER_INFO_6W, DocumentPropertiesW, GetPrinterDriverW, GetPrinterW,
            OpenPrinterW, PRINTER_INFO_5W,
        },
    },
    Storage::Xps::{DC_COLORDEVICE, DC_COPIES, DC_DUPLEX, DC_ENUMRESOLUTIONS, DeviceCapabilitiesW},
};

const DOCUMENT_PROPERTIES_OK: i32 = 1;
const DOCUMENT_PROPERTIES_CANCEL: i32 = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaptureOutcome {
    Captured(Box<WindowsNativeProfileCapture>),
    Cancelled,
}

pub fn capture_profile(
    native_printer_id: &str,
    owner_window: Option<isize>,
    existing: Option<&WindowsNativeProfileCapture>,
) -> Result<CaptureOutcome, NativeProfileError> {
    let printer = PrinterHandle::open(native_printer_id)?;
    let before = fingerprint(&printer, native_printer_id)?;
    let initial = if let Some(existing) = existing {
        if let Some(error) = existing.fingerprint.compatibility_error(&before) {
            return Err(error);
        }
        normalize_bytes(&printer, native_printer_id, &existing.validate_envelope()?)?
    } else {
        default_devmode(&printer, native_printer_id)?
    };
    let prompted = prompt_for_devmode(&printer, native_printer_id, owner_window, &initial)?;
    let Some(prompted) = prompted else {
        return Ok(CaptureOutcome::Cancelled);
    };
    let normalized = normalize_bytes(&printer, native_printer_id, &prompted)?;
    let after = fingerprint(&printer, native_printer_id)?;
    if let Some(error) = before.compatibility_error(&after) {
        return Err(NativeProfileError::new(
            "driver_changed_during_capture",
            error.message,
        ));
    }
    Ok(CaptureOutcome::Captured(Box::new(
        WindowsNativeProfileCapture::new(after, &normalized)?,
    )))
}

pub fn validate_profile(
    native_printer_id: &str,
    capture: &WindowsNativeProfileCapture,
) -> Result<WindowsNativeProfileCapture, NativeProfileError> {
    let printer = PrinterHandle::open(native_printer_id)?;
    let current = fingerprint(&printer, native_printer_id)?;
    if let Some(error) = capture.fingerprint.compatibility_error(&current) {
        return Err(error);
    }
    let captured = capture.validate_envelope()?;
    let normalized = normalize_bytes(&printer, native_printer_id, &captured)?;
    if normalized != captured {
        return Err(NativeProfileError::new(
            "devmode_normalization_changed",
            "the installed driver no longer normalizes this profile to the captured DEVMODE; create a new profile revision",
        ));
    }
    Ok(capture.clone())
}

pub fn current_fingerprint(
    native_printer_id: &str,
) -> Result<WindowsDriverFingerprint, NativeProfileError> {
    let printer = PrinterHandle::open(native_printer_id)?;
    fingerprint(&printer, native_printer_id)
}

/// Discovers portable capabilities through documented Winspool APIs without
/// opening a driver UI or changing queue defaults. Vendor-private controls are
/// intentionally absent until a trusted mapping/profile exists.
pub fn portable_capabilities(
    native_printer_id: &str,
) -> Result<PrinterCapabilities, NativeProfileError> {
    let printer = PrinterHandle::open(native_printer_id)?;
    let printer_buffer = get_printer_info(&printer)?;
    let queue = read_record::<PRINTER_INFO_5W>(&printer_buffer)?;
    let port = bounded_wide(queue.pPortName, &printer_buffer).ok_or_else(|| {
        NativeProfileError::new(
            "printer_port_missing",
            "Windows printer metadata did not contain a bounded port name",
        )
    })?;
    let device = wide(native_printer_id);
    let port = wide(&port);
    let devmode = AlignedBuffer::from_bytes(&default_devmode(&printer, native_printer_id)?)?;
    let scalar = |capability| {
        // SAFETY: Device and port are NUL-terminated and the validated default
        // DEVMODE remains alive for this non-mutating capability query.
        unsafe {
            DeviceCapabilitiesW(
                device.as_ptr(),
                port.as_ptr(),
                capability,
                ptr::null_mut(),
                devmode.as_devmode(),
            )
        }
    };
    let copies = scalar(DC_COPIES);
    let mut capabilities = PrinterCapabilities {
        color: scalar(DC_COLORDEVICE) == 1,
        copies: u32::try_from(copies).unwrap_or_default(),
        duplex: scalar(DC_DUPLEX) == 1,
        ..Default::default()
    };
    capabilities.dpis = resolution_capabilities(&device, &port, &devmode)?;
    Ok(capabilities)
}

fn resolution_capabilities(
    device: &[u16],
    port: &[u16],
    devmode: &AlignedBuffer,
) -> Result<Vec<String>, NativeProfileError> {
    // SAFETY: Inputs are valid for the duration of this count query.
    let count = unsafe {
        DeviceCapabilitiesW(
            device.as_ptr(),
            port.as_ptr(),
            DC_ENUMRESOLUTIONS,
            ptr::null_mut(),
            devmode.as_devmode(),
        )
    };
    if count == -1 {
        return Ok(Vec::new());
    }
    let count = usize::try_from(count).map_err(|_| {
        NativeProfileError::new("windows_capability_invalid", "invalid resolution count")
    })?;
    if count > 4_096 {
        return Err(NativeProfileError::new(
            "windows_capability_too_large",
            "driver advertised more than 4096 resolutions",
        ));
    }
    let mut pairs = vec![0_i32; count.saturating_mul(2)];
    if count > 0 {
        // SAFETY: DC_ENUMRESOLUTIONS writes two i32 values per advertised
        // resolution; `pairs` has exactly that bounded capacity.
        let written = unsafe {
            DeviceCapabilitiesW(
                device.as_ptr(),
                port.as_ptr(),
                DC_ENUMRESOLUTIONS,
                pairs.as_mut_ptr().cast(),
                devmode.as_devmode(),
            )
        };
        if written != i32::try_from(count).unwrap_or(-1) {
            return Err(last_error(
                "windows_capability_changed",
                "resolution capabilities changed during discovery",
            ));
        }
    }
    Ok(pairs
        .chunks_exact(2)
        .filter(|pair| pair[0] > 0 && pair[1] > 0)
        .map(|pair| {
            if pair[0] == pair[1] {
                pair[0].to_string()
            } else {
                format!("{}x{}", pair[0], pair[1])
            }
        })
        .collect())
}

/// Revalidates the immutable capture against the currently installed queue and
/// asks the same vendor driver to normalize the DEVMODE. A byte change means
/// the saved private settings no longer have their original meaning, so replay
/// fails closed and requires a new profile revision.
pub fn revalidate_profile_devmode(
    native_printer_id: &str,
    capture: &WindowsNativeProfileCapture,
) -> Result<Vec<u8>, NativeProfileError> {
    let printer = PrinterHandle::open(native_printer_id)?;
    let current = fingerprint(&printer, native_printer_id)?;
    if let Some(error) = capture.fingerprint.compatibility_error(&current) {
        return Err(error);
    }
    let captured = capture.validate_envelope()?;
    let normalized = normalize_bytes(&printer, native_printer_id, &captured)?;
    if normalized != captured {
        return Err(NativeProfileError::new(
            "devmode_normalization_changed",
            "the installed driver changed the captured DEVMODE; create and test a new profile revision before printing",
        ));
    }
    Ok(normalized)
}

/// Normalizes explicitly permitted public-field overrides while retaining the
/// captured vendor-private data. Driver identity is checked again so a queue
/// update between profile validation and submission cannot silently reinterpret
/// the opaque bytes.
pub fn normalize_replay_devmode(
    native_printer_id: &str,
    capture: &WindowsNativeProfileCapture,
    candidate: &[u8],
) -> Result<Vec<u8>, NativeProfileError> {
    validate_devmode_bytes(candidate)?;
    let printer = PrinterHandle::open(native_printer_id)?;
    let current = fingerprint(&printer, native_printer_id)?;
    if let Some(error) = capture.fingerprint.compatibility_error(&current) {
        return Err(error);
    }
    normalize_bytes(&printer, native_printer_id, candidate)
}

struct PrinterHandle(HANDLE);

impl PrinterHandle {
    fn open(native_printer_id: &str) -> Result<Self, NativeProfileError> {
        let name = wide(native_printer_id);
        let mut handle: HANDLE = ptr::null_mut();
        // SAFETY: `name` is NUL-terminated and lives for this synchronous
        // call. A successful handle is owned by `PrinterHandle`.
        if unsafe { OpenPrinterW(name.as_ptr(), &mut handle, ptr::null()) } == 0 {
            return Err(last_error(
                "winspool_open_failed",
                format!("could not open Windows printer {native_printer_id}"),
            ));
        }
        Ok(Self(handle))
    }
}

impl Drop for PrinterHandle {
    fn drop(&mut self) {
        // SAFETY: The constructor only stores successful OpenPrinterW handles
        // and Drop runs once for the owning value.
        unsafe {
            ClosePrinter(self.0);
        }
    }
}

struct AlignedBuffer {
    words: Vec<usize>,
    byte_len: usize,
}

impl AlignedBuffer {
    fn zeroed(byte_len: usize) -> Result<Self, NativeProfileError> {
        if byte_len == 0 || byte_len > 16 * 1024 * 1024 {
            return Err(NativeProfileError::new(
                "devmode_size_invalid",
                format!("driver requested an invalid DEVMODE buffer size of {byte_len} bytes"),
            ));
        }
        let word_size = mem::size_of::<usize>();
        let word_len = byte_len
            .checked_add(word_size - 1)
            .and_then(|value| value.checked_div(word_size))
            .ok_or_else(|| {
                NativeProfileError::new("devmode_size_overflow", "DEVMODE allocation overflow")
            })?;
        Ok(Self {
            words: vec![0; word_len],
            byte_len,
        })
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, NativeProfileError> {
        let mut buffer = Self::zeroed(bytes.len())?;
        buffer.as_mut_bytes().copy_from_slice(bytes);
        Ok(buffer)
    }

    fn as_devmode(&self) -> *const DEVMODEW {
        self.words.as_ptr().cast()
    }

    fn as_mut_devmode(&mut self) -> *mut DEVMODEW {
        self.words.as_mut_ptr().cast()
    }

    fn as_ptr(&self) -> *const u8 {
        self.words.as_ptr().cast()
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.words.as_mut_ptr().cast()
    }

    fn as_bytes(&self) -> &[u8] {
        // SAFETY: The word allocation is at least `byte_len` bytes and remains
        // live for the returned borrow.
        unsafe { slice::from_raw_parts(self.as_ptr(), self.byte_len) }
    }

    fn as_mut_bytes(&mut self) -> &mut [u8] {
        // SAFETY: The word allocation is at least `byte_len` bytes, uniquely
        // borrowed, and remains live for the returned borrow.
        unsafe { slice::from_raw_parts_mut(self.as_mut_ptr(), self.byte_len) }
    }

    fn captured_bytes(&self) -> Result<Vec<u8>, NativeProfileError> {
        if self.byte_len < 72 {
            return Err(NativeProfileError::new(
                "devmode_truncated",
                "driver returned a truncated DEVMODE",
            ));
        }
        let bytes = self.as_bytes();
        let public = u16::from_le_bytes([bytes[68], bytes[69]]);
        let private = u16::from_le_bytes([bytes[70], bytes[71]]);
        let total = usize::from(public)
            .checked_add(usize::from(private))
            .ok_or_else(|| {
                NativeProfileError::new("devmode_size_overflow", "DEVMODE size overflow")
            })?;
        if total > bytes.len() {
            return Err(NativeProfileError::new(
                "devmode_size_mismatch",
                format!(
                    "driver returned a {total}-byte DEVMODE in a {}-byte buffer",
                    bytes.len()
                ),
            ));
        }
        let captured = bytes[..total].to_vec();
        validate_devmode_bytes(&captured)?;
        Ok(captured)
    }
}

fn queried_devmode_size(
    printer: &PrinterHandle,
    native_printer_id: &str,
) -> Result<usize, NativeProfileError> {
    let name = wide(native_printer_id);
    // SAFETY: Handle and NUL-terminated name are valid; zero mode with null
    // buffers is the documented size query.
    let result = unsafe {
        DocumentPropertiesW(
            ptr::null_mut(),
            printer.0,
            name.as_ptr(),
            ptr::null_mut(),
            ptr::null(),
            0,
        )
    };
    usize::try_from(result).map_err(|_| {
        last_error(
            "document_properties_size_failed",
            "printer driver did not return a valid DEVMODE size",
        )
    })
}

fn default_devmode(
    printer: &PrinterHandle,
    native_printer_id: &str,
) -> Result<Vec<u8>, NativeProfileError> {
    let name = wide(native_printer_id);
    let mut output = AlignedBuffer::zeroed(queried_devmode_size(printer, native_printer_id)?)?;
    // SAFETY: Output is aligned and has the exact driver-requested capacity.
    let result = unsafe {
        DocumentPropertiesW(
            ptr::null_mut(),
            printer.0,
            name.as_ptr(),
            output.as_mut_devmode(),
            ptr::null(),
            DM_OUT_BUFFER,
        )
    };
    document_properties_success(result, "document_properties_default_failed")?;
    output.captured_bytes()
}

fn normalize_bytes(
    printer: &PrinterHandle,
    native_printer_id: &str,
    input: &[u8],
) -> Result<Vec<u8>, NativeProfileError> {
    validate_devmode_bytes(input)?;
    let name = wide(native_printer_id);
    let input = AlignedBuffer::from_bytes(input)?;
    let mut output = AlignedBuffer::zeroed(queried_devmode_size(printer, native_printer_id)?)?;
    // SAFETY: Both buffers are aligned and live through this synchronous call;
    // the output uses the current driver's queried size.
    let result = unsafe {
        DocumentPropertiesW(
            ptr::null_mut(),
            printer.0,
            name.as_ptr(),
            output.as_mut_devmode(),
            input.as_devmode(),
            DM_IN_BUFFER | DM_OUT_BUFFER,
        )
    };
    document_properties_success(result, "document_properties_normalize_failed")?;
    output.captured_bytes()
}

fn prompt_for_devmode(
    printer: &PrinterHandle,
    native_printer_id: &str,
    owner_window: Option<isize>,
    input: &[u8],
) -> Result<Option<Vec<u8>>, NativeProfileError> {
    validate_devmode_bytes(input)?;
    let name = wide(native_printer_id);
    let input = AlignedBuffer::from_bytes(input)?;
    let mut output = AlignedBuffer::zeroed(queried_devmode_size(printer, native_printer_id)?)?;
    let owner: HWND = owner_window.map_or(ptr::null_mut(), |window| window as *mut c_void);
    // SAFETY: The owner is either null or supplied by the interactive shell;
    // buffers are aligned and remain valid until the modal property sheet
    // returns.
    let result = unsafe {
        DocumentPropertiesW(
            owner,
            printer.0,
            name.as_ptr(),
            output.as_mut_devmode(),
            input.as_devmode(),
            DM_IN_PROMPT | DM_IN_BUFFER | DM_OUT_BUFFER,
        )
    };
    if result == DOCUMENT_PROPERTIES_CANCEL {
        return Ok(None);
    }
    document_properties_success(result, "document_properties_prompt_failed")?;
    output.captured_bytes().map(Some)
}

fn document_properties_success(result: i32, code: &'static str) -> Result<(), NativeProfileError> {
    if result == DOCUMENT_PROPERTIES_OK {
        Ok(())
    } else {
        Err(last_error(
            code,
            format!("printer driver returned DocumentProperties result {result}"),
        ))
    }
}

fn fingerprint(
    printer: &PrinterHandle,
    native_printer_id: &str,
) -> Result<WindowsDriverFingerprint, NativeProfileError> {
    let driver_buffer = get_driver_info(printer)?;
    let printer_buffer = get_printer_info(printer)?;
    let driver = read_record::<DRIVER_INFO_6W>(&driver_buffer)?;
    let queue = read_record::<PRINTER_INFO_5W>(&printer_buffer)?;
    let driver_name =
        bounded_wide(driver.pName, &driver_buffer).unwrap_or_else(|| "Unknown driver".into());
    let environment =
        bounded_wide(driver.pEnvironment, &driver_buffer).unwrap_or_else(|| "Windows".into());
    let port = bounded_wide(queue.pPortName, &printer_buffer).unwrap_or_default();
    let hardware_id = bounded_wide(driver.pszHardwareID, &driver_buffer).unwrap_or_default();
    let manufacturer = bounded_wide(driver.pszMfgName, &driver_buffer).unwrap_or_default();
    let provider = bounded_wide(driver.pszProvider, &driver_buffer).unwrap_or_default();
    let mut device_hasher = Sha256::new();
    for value in [&port, &hardware_id, &manufacturer, &provider] {
        device_hasher.update(value.as_bytes());
        device_hasher.update([0]);
    }
    let driver_date = (u64::from(driver.ftDriverDate.dwHighDateTime) << 32)
        | u64::from(driver.ftDriverDate.dwLowDateTime);
    Ok(WindowsDriverFingerprint {
        platform: "windows".into(),
        driver_name,
        driver_version: format_driver_version(driver.dwlDriverVersion),
        driver_environment: environment,
        architecture: std::env::consts::ARCH.into(),
        native_queue_id: native_printer_id.into(),
        device_fingerprint: format!("sha256:{}", hex::encode(device_hasher.finalize())),
        driver_date_filetime: (driver_date != 0).then_some(driver_date),
    })
}

fn get_driver_info(printer: &PrinterHandle) -> Result<AlignedBuffer, NativeProfileError> {
    let mut needed = 0_u32;
    // SAFETY: Null-buffer call obtains the required level-6 allocation.
    unsafe {
        GetPrinterDriverW(printer.0, ptr::null(), 6, ptr::null_mut(), 0, &mut needed);
    }
    require_buffer_size("get_printer_driver_size_failed", needed)?;
    let mut buffer = AlignedBuffer::zeroed(needed as usize)?;
    // SAFETY: Buffer is aligned and exactly `needed` bytes.
    if unsafe {
        GetPrinterDriverW(
            printer.0,
            ptr::null(),
            6,
            buffer.as_mut_ptr(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(last_error(
            "get_printer_driver_failed",
            "could not read Windows printer driver metadata",
        ));
    }
    Ok(buffer)
}

fn get_printer_info(printer: &PrinterHandle) -> Result<AlignedBuffer, NativeProfileError> {
    let mut needed = 0_u32;
    // SAFETY: Null-buffer call obtains the required level-5 allocation.
    unsafe {
        GetPrinterW(printer.0, 5, ptr::null_mut(), 0, &mut needed);
    }
    require_buffer_size("get_printer_size_failed", needed)?;
    let mut buffer = AlignedBuffer::zeroed(needed as usize)?;
    // SAFETY: Buffer is aligned and exactly `needed` bytes.
    if unsafe { GetPrinterW(printer.0, 5, buffer.as_mut_ptr(), needed, &mut needed) } == 0 {
        return Err(last_error(
            "get_printer_failed",
            "could not read Windows printer destination metadata",
        ));
    }
    Ok(buffer)
}

fn require_buffer_size(code: &'static str, needed: u32) -> Result<(), NativeProfileError> {
    // SAFETY: GetLastError has no preconditions and is read immediately after
    // the size-query API.
    let error = unsafe { GetLastError() };
    if needed == 0 || error != ERROR_INSUFFICIENT_BUFFER {
        return Err(NativeProfileError::new(
            code,
            format!("Windows metadata size query failed with error {error}"),
        ));
    }
    Ok(())
}

fn read_record<T: Copy>(buffer: &AlignedBuffer) -> Result<T, NativeProfileError> {
    if buffer.byte_len < mem::size_of::<T>() {
        return Err(NativeProfileError::new(
            "winspool_metadata_truncated",
            "Windows printer metadata record was truncated",
        ));
    }
    // SAFETY: `AlignedBuffer` is suitably aligned and contains at least one
    // complete T. T is copied before the buffer can be dropped.
    Ok(unsafe { buffer.as_ptr().cast::<T>().read() })
}

fn bounded_wide(pointer: *mut u16, buffer: &AlignedBuffer) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    let start = buffer.as_ptr() as usize;
    let end = start.checked_add(buffer.byte_len)?;
    let address = pointer as usize;
    if address < start || address >= end || !address.is_multiple_of(mem::size_of::<u16>()) {
        return None;
    }
    let units = (end - address) / mem::size_of::<u16>();
    // SAFETY: Pointer containment and alignment are checked above and the
    // resulting slice is bounded to the live Winspool allocation.
    let values = unsafe { slice::from_raw_parts(pointer, units) };
    let length = values.iter().position(|value| *value == 0)?;
    Some(String::from_utf16_lossy(&values[..length]))
}

fn format_driver_version(version: u64) -> String {
    format!(
        "{}.{}.{}.{}",
        (version >> 48) & 0xffff,
        (version >> 32) & 0xffff,
        (version >> 16) & 0xffff,
        version & 0xffff
    )
}

fn last_error(code: &'static str, context: impl Into<String>) -> NativeProfileError {
    // SAFETY: GetLastError has no preconditions.
    let error = unsafe { GetLastError() };
    NativeProfileError::new(code, format!("{} (Win32 error {error})", context.into()))
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
