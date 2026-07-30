use crate::{
    native_profile::{NativeProfileError, WindowsNativeProfileCapture},
    replay::{
        WindowsPdfBackend, apply_public_devmode_overrides, prepare_profile_replay,
        safe_override_names, selected_pages,
    },
    windows_native::{current_fingerprint, normalize_replay_devmode, revalidate_profile_devmode},
};
use piqae_domain::{JobOptions, NativeProfileKind, Rotation};
use piqae_protocol::executor::{ExecutorError, ExecutorResult, NativeProfilePayload};
use std::{
    ffi::{c_char, c_int, c_ulong, c_void},
    io::Read as _,
    mem, path, ptr, slice,
};
use windows_sys::Win32::{
    Foundation::{FreeLibrary, GetLastError, HMODULE},
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDCW, DIB_RGB_COLORS, DeleteDC, GetDeviceCaps,
        HALFTONE, HDC, HORZRES, LOGPIXELSX, LOGPIXELSY, RGBQUAD, SRCCOPY, SetStretchBltMode,
        StretchDIBits, VERTRES,
    },
    Storage::Xps::{AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage},
    System::LibraryLoader::{
        GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
        LoadLibraryExW,
    },
};

const MAX_PDF_BYTES: u64 = 512 * 1024 * 1024;
const MAX_BITMAP_BYTES: usize = 384 * 1024 * 1024;
const DEFAULT_MAX_RENDER_DPI: i32 = 600;
const MAX_CONFIGURED_RENDER_DPI: i32 = 2400;
const FPDF_ANNOT: c_int = 0x01;
const FPDF_PRINTING: c_int = 0x800;

pub fn submit_native_pdf(
    printer: &str,
    title: &str,
    content_path: &str,
    options: &JobOptions,
    profile: &NativeProfilePayload,
) -> Result<ExecutorResult, ExecutorError> {
    let capture = validate_payload(printer, profile)?;
    let current = current_fingerprint(printer).map_err(profile_error)?;
    let safe_overrides = safe_override_names(&profile.safe_overrides);
    let plan = prepare_profile_replay(
        &capture,
        &current,
        options,
        &safe_overrides,
        WindowsPdfBackend::GdiPdfium,
    )
    .map_err(profile_error)?;

    // Revalidate the exact immutable bytes before applying the small,
    // explicitly permitted public-field overlay.
    let exact = revalidate_profile_devmode(printer, &capture).map_err(profile_error)?;
    if exact != plan.devmode_bytes {
        return Err(executor_error(
            "devmode_revalidation_mismatch",
            "revalidated DEVMODE differs from the immutable profile revision",
            false,
            false,
        ));
    }
    let mut candidate = exact;
    apply_public_devmode_overrides(&mut candidate, options).map_err(profile_error)?;
    let normalized =
        normalize_replay_devmode(printer, &capture, &candidate).map_err(profile_error)?;

    let pdf = read_bounded_pdf(content_path)?;
    let pdfium = PdfiumLibrary::load()?;
    let document = pdfium.load_document(&pdf)?;
    let page_count = document.page_count()?;
    let selected = selected_pages(options.pages.as_deref(), page_count).map_err(profile_error)?;
    document.validate_pages(&selected)?;

    let device = PrinterDevice::create(printer, &normalized)?;
    let renderer = Renderer::new(&device)?;
    let mut job = PrintJob::start(device, title)?;
    for page_index in selected {
        let rendered = document
            .render_page(page_index, options.rotate, &renderer)
            .map_err(mark_handoff)?;
        job.print_page(&rendered, options.fit_to_page.unwrap_or(false))?;
    }
    let job_id = job.finish()?;
    Ok(ExecutorResult::Submitted {
        native_job_id: Some(job_id.to_string()),
    })
}

fn validate_payload(
    printer: &str,
    profile: &NativeProfilePayload,
) -> Result<WindowsNativeProfileCapture, ExecutorError> {
    if profile.kind != NativeProfileKind::WindowsDevmode {
        return Err(executor_error(
            "native_profile_kind_unsupported",
            &format!(
                "Windows GDI replay requires windows_devmode, received {:?}",
                profile.kind
            ),
            false,
            false,
        ));
    }
    if profile.schema_version != crate::native_profile::WINDOWS_NATIVE_PROFILE_SCHEMA_VERSION {
        return Err(executor_error(
            "native_profile_schema_unsupported",
            &format!(
                "unsupported Windows profile schema {}",
                profile.schema_version
            ),
            false,
            false,
        ));
    }
    let actual_digest = crate::replay::profile_blob_digest(&profile.blob);
    if !constant_time_equal(actual_digest.as_bytes(), profile.digest.as_bytes()) {
        return Err(executor_error(
            "native_profile_digest_mismatch",
            "native profile payload digest does not match the pinned revision",
            false,
            false,
        ));
    }
    let capture: WindowsNativeProfileCapture =
        serde_json::from_slice(&profile.blob).map_err(|error| {
            executor_error(
                "native_profile_invalid",
                &format!("Windows DEVMODE envelope is invalid: {error}"),
                false,
                false,
            )
        })?;
    capture.validate_envelope().map_err(profile_error)?;
    if capture.fingerprint.native_queue_id != printer {
        return Err(executor_error(
            "destination_mismatch",
            "profile belongs to a different Windows printer queue",
            false,
            false,
        ));
    }
    if profile.driver_fingerprint.platform != "windows"
        || profile.driver_fingerprint.native_queue_id != printer
        || profile.driver_fingerprint.driver_name != capture.fingerprint.driver_name
        || profile.driver_fingerprint.driver_version.as_deref()
            != Some(capture.fingerprint.driver_version.as_str())
        || profile.driver_fingerprint.architecture.as_deref()
            != Some(capture.fingerprint.architecture.as_str())
        || profile.driver_fingerprint.device_fingerprint.as_deref()
            != Some(capture.fingerprint.device_fingerprint.as_str())
    {
        return Err(executor_error(
            "native_profile_metadata_mismatch",
            "pinned profile metadata does not match its Windows capture envelope",
            false,
            false,
        ));
    }
    Ok(capture)
}

fn read_bounded_pdf(content_path: &str) -> Result<Vec<u8>, ExecutorError> {
    let file = std::fs::File::open(content_path).map_err(|error| {
        executor_error(
            "content_unavailable",
            &format!("could not open PDF: {error}"),
            false,
            false,
        )
    })?;
    let metadata = file.metadata().map_err(|error| {
        executor_error(
            "content_unavailable",
            &format!("could not inspect PDF: {error}"),
            false,
            false,
        )
    })?;
    if metadata.len() == 0 || metadata.len() > MAX_PDF_BYTES {
        return Err(executor_error(
            "pdf_size_invalid",
            "PDF must contain between 1 byte and 512 MiB",
            false,
            false,
        ));
    }
    let mut content = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(MAX_PDF_BYTES + 1)
        .read_to_end(&mut content)
        .map_err(|error| {
            executor_error(
                "content_unavailable",
                &format!("could not read PDF: {error}"),
                false,
                false,
            )
        })?;
    if content.len() as u64 > MAX_PDF_BYTES {
        return Err(executor_error(
            "pdf_size_invalid",
            "PDF changed while reading and exceeded the 512 MiB limit",
            false,
            false,
        ));
    }
    Ok(content)
}

struct PdfiumLibrary {
    module: HMODULE,
    api: PdfiumApi,
}

impl PdfiumLibrary {
    fn load() -> Result<Self, ExecutorError> {
        let path = pdfium_path()?;
        let wide_path = wide_os(path.as_os_str());
        // SAFETY: The canonical absolute path is NUL-terminated. Search flags
        // limit dependent DLL resolution to the PDFium directory and System32.
        let module = unsafe {
            LoadLibraryExW(
                wide_path.as_ptr(),
                ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if module.is_null() {
            return Err(last_win32_error(
                "windows_pdfium_load_failed",
                "could not load the packaged PDFium library",
                false,
            ));
        }
        let api = match PdfiumApi::load(module) {
            Ok(api) => api,
            Err(error) => {
                // SAFETY: `module` was returned by LoadLibraryExW above.
                unsafe { FreeLibrary(module) };
                return Err(error);
            }
        };
        // SAFETY: All required exports were resolved from this live module.
        unsafe { (api.init_library)() };
        Ok(Self { module, api })
    }

    fn load_document<'library>(
        &'library self,
        bytes: &'library [u8],
    ) -> Result<PdfDocument<'library>, ExecutorError> {
        // SAFETY: The PDF bytes remain borrowed for the document's lifetime.
        let handle = unsafe {
            (self.api.load_mem_document)(bytes.as_ptr().cast(), bytes.len(), ptr::null::<c_char>())
        };
        if handle.is_null() {
            return Err(executor_error(
                "pdfium_document_invalid",
                &format!(
                    "PDFium could not open the document (error {})",
                    // SAFETY: PDFium is initialized and the call has no inputs.
                    unsafe { (self.api.get_last_error)() }
                ),
                false,
                false,
            ));
        }
        Ok(PdfDocument {
            library: self,
            handle,
            _bytes: bytes,
        })
    }
}

impl Drop for PdfiumLibrary {
    fn drop(&mut self) {
        // SAFETY: Documents borrow this library and therefore drop first.
        unsafe {
            (self.api.destroy_library)();
            FreeLibrary(self.module);
        }
    }
}

#[derive(Clone, Copy)]
struct PdfiumApi {
    init_library: unsafe extern "system" fn(),
    destroy_library: unsafe extern "system" fn(),
    load_mem_document:
        unsafe extern "system" fn(*const c_void, usize, *const c_char) -> *mut c_void,
    close_document: unsafe extern "system" fn(*mut c_void),
    get_last_error: unsafe extern "system" fn() -> c_ulong,
    get_page_count: unsafe extern "system" fn(*mut c_void) -> c_int,
    load_page: unsafe extern "system" fn(*mut c_void, c_int) -> *mut c_void,
    close_page: unsafe extern "system" fn(*mut c_void),
    get_page_width: unsafe extern "system" fn(*mut c_void) -> f32,
    get_page_height: unsafe extern "system" fn(*mut c_void) -> f32,
    bitmap_create: unsafe extern "system" fn(c_int, c_int, c_int) -> *mut c_void,
    bitmap_destroy: unsafe extern "system" fn(*mut c_void),
    bitmap_fill_rect: unsafe extern "system" fn(*mut c_void, c_int, c_int, c_int, c_int, u32),
    bitmap_get_buffer: unsafe extern "system" fn(*mut c_void) -> *mut c_void,
    bitmap_get_stride: unsafe extern "system" fn(*mut c_void) -> c_int,
    render_page_bitmap: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
    ),
}

impl PdfiumApi {
    fn load(module: HMODULE) -> Result<Self, ExecutorError> {
        // SAFETY: Each concrete type exactly matches the documented PDFium C
        // export. Missing exports return a controlled dependency error.
        unsafe {
            Ok(Self {
                init_library: resolve(module, b"FPDF_InitLibrary\0")?,
                destroy_library: resolve(module, b"FPDF_DestroyLibrary\0")?,
                load_mem_document: resolve(module, b"FPDF_LoadMemDocument64\0")?,
                close_document: resolve(module, b"FPDF_CloseDocument\0")?,
                get_last_error: resolve(module, b"FPDF_GetLastError\0")?,
                get_page_count: resolve(module, b"FPDF_GetPageCount\0")?,
                load_page: resolve(module, b"FPDF_LoadPage\0")?,
                close_page: resolve(module, b"FPDF_ClosePage\0")?,
                get_page_width: resolve(module, b"FPDF_GetPageWidthF\0")?,
                get_page_height: resolve(module, b"FPDF_GetPageHeightF\0")?,
                bitmap_create: resolve(module, b"FPDFBitmap_Create\0")?,
                bitmap_destroy: resolve(module, b"FPDFBitmap_Destroy\0")?,
                bitmap_fill_rect: resolve(module, b"FPDFBitmap_FillRect\0")?,
                bitmap_get_buffer: resolve(module, b"FPDFBitmap_GetBuffer\0")?,
                bitmap_get_stride: resolve(module, b"FPDFBitmap_GetStride\0")?,
                render_page_bitmap: resolve(module, b"FPDF_RenderPageBitmap\0")?,
            })
        }
    }
}

unsafe fn resolve<T: Copy>(module: HMODULE, name: &'static [u8]) -> Result<T, ExecutorError> {
    debug_assert_eq!(mem::size_of::<T>(), mem::size_of::<usize>());
    // SAFETY: Name is statically NUL-terminated and module remains loaded.
    let symbol = unsafe { GetProcAddress(module, name.as_ptr()) };
    let Some(symbol) = symbol else {
        return Err(executor_error(
            "windows_pdfium_incompatible",
            &format!(
                "packaged PDFium is missing {}",
                String::from_utf8_lossy(&name[..name.len().saturating_sub(1)])
            ),
            false,
            false,
        ));
    };
    // SAFETY: Caller supplies the exact signature for the named C export.
    Ok(unsafe { mem::transmute_copy(&symbol) })
}

struct PdfDocument<'library> {
    library: &'library PdfiumLibrary,
    handle: *mut c_void,
    _bytes: &'library [u8],
}

impl PdfDocument<'_> {
    fn page_count(&self) -> Result<usize, ExecutorError> {
        // SAFETY: Document handle is live.
        let count = unsafe { (self.library.api.get_page_count)(self.handle) };
        usize::try_from(count).map_err(|_| {
            executor_error(
                "pdfium_page_count_invalid",
                "PDFium returned an invalid page count",
                false,
                false,
            )
        })
    }

    fn validate_pages(&self, pages: &[usize]) -> Result<(), ExecutorError> {
        for index in pages {
            let page = self.open_page(*index)?;
            page.dimensions()?;
        }
        Ok(())
    }

    fn render_page(
        &self,
        index: usize,
        rotation: Option<Rotation>,
        renderer: &Renderer,
    ) -> Result<RenderedPage, ExecutorError> {
        let page = self.open_page(index)?;
        let (width_points, height_points) = page.dimensions()?;
        let rotation = rotation.unwrap_or(Rotation::Deg0);
        let quarter_turn = matches!(rotation, Rotation::Deg90 | Rotation::Deg270);
        let (rotated_width, rotated_height) = if quarter_turn {
            (height_points, width_points)
        } else {
            (width_points, height_points)
        };
        let placement = renderer.placement(rotated_width, rotated_height)?;
        let bitmap = PdfBitmap::new(
            self.library,
            placement.bitmap_width,
            placement.bitmap_height,
        )?;
        // SAFETY: Bitmap and page handles are live and dimensions were bounded.
        unsafe {
            (self.library.api.bitmap_fill_rect)(
                bitmap.handle,
                0,
                0,
                placement.bitmap_width,
                placement.bitmap_height,
                0xffff_ffff,
            );
            (self.library.api.render_page_bitmap)(
                bitmap.handle,
                page.handle,
                0,
                0,
                placement.bitmap_width,
                placement.bitmap_height,
                rotation_index(rotation),
                FPDF_ANNOT | FPDF_PRINTING,
            );
        }
        bitmap.into_rendered(placement)
    }

    fn open_page(&self, index: usize) -> Result<PdfPage<'_>, ExecutorError> {
        let index = c_int::try_from(index).map_err(|_| {
            executor_error(
                "pdfium_page_index_invalid",
                "PDF page index exceeds PDFium limits",
                false,
                false,
            )
        })?;
        // SAFETY: Document is live and index was selected within page count.
        let handle = unsafe { (self.library.api.load_page)(self.handle, index) };
        if handle.is_null() {
            return Err(executor_error(
                "pdfium_page_load_failed",
                &format!("PDFium could not load page {}", index + 1),
                false,
                false,
            ));
        }
        Ok(PdfPage {
            library: self.library,
            handle,
        })
    }
}

impl Drop for PdfDocument<'_> {
    fn drop(&mut self) {
        // SAFETY: Handle is owned by this document wrapper.
        unsafe { (self.library.api.close_document)(self.handle) };
    }
}

struct PdfPage<'library> {
    library: &'library PdfiumLibrary,
    handle: *mut c_void,
}

impl PdfPage<'_> {
    fn dimensions(&self) -> Result<(f32, f32), ExecutorError> {
        // SAFETY: Page handle is live.
        let width = unsafe { (self.library.api.get_page_width)(self.handle) };
        // SAFETY: Page handle is live.
        let height = unsafe { (self.library.api.get_page_height)(self.handle) };
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err(executor_error(
                "pdfium_page_dimensions_invalid",
                "PDF page has invalid physical dimensions",
                false,
                false,
            ));
        }
        Ok((width, height))
    }
}

impl Drop for PdfPage<'_> {
    fn drop(&mut self) {
        // SAFETY: Handle is owned by this page wrapper.
        unsafe { (self.library.api.close_page)(self.handle) };
    }
}

struct PdfBitmap<'library> {
    library: &'library PdfiumLibrary,
    handle: *mut c_void,
    width: c_int,
    height: c_int,
}

impl<'library> PdfBitmap<'library> {
    fn new(
        library: &'library PdfiumLibrary,
        width: c_int,
        height: c_int,
    ) -> Result<Self, ExecutorError> {
        let bytes = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(bitmap_too_large)?;
        if bytes > MAX_BITMAP_BYTES {
            return Err(bitmap_too_large());
        }
        // SAFETY: Dimensions are positive, bounded signed integers.
        let handle = unsafe { (library.api.bitmap_create)(width, height, 0) };
        if handle.is_null() {
            return Err(executor_error(
                "pdfium_bitmap_allocation_failed",
                "PDFium could not allocate the bounded page bitmap",
                false,
                false,
            ));
        }
        Ok(Self {
            library,
            handle,
            width,
            height,
        })
    }

    fn into_rendered(self, placement: RenderPlacement) -> Result<RenderedPage, ExecutorError> {
        // SAFETY: Bitmap handle is live.
        let stride = unsafe { (self.library.api.bitmap_get_stride)(self.handle) };
        if stride <= 0 {
            return Err(executor_error(
                "pdfium_bitmap_invalid",
                "PDFium returned an invalid bitmap stride",
                false,
                false,
            ));
        }
        let length = usize::try_from(stride)
            .ok()
            .and_then(|stride| {
                usize::try_from(self.height)
                    .ok()
                    .and_then(|height| stride.checked_mul(height))
            })
            .filter(|length| *length <= MAX_BITMAP_BYTES)
            .ok_or_else(bitmap_too_large)?;
        // SAFETY: PDFium owns a live buffer of stride * height bytes.
        let pointer = unsafe { (self.library.api.bitmap_get_buffer)(self.handle) };
        if pointer.is_null() {
            return Err(executor_error(
                "pdfium_bitmap_invalid",
                "PDFium returned an empty bitmap buffer",
                false,
                false,
            ));
        }
        // SAFETY: The source is live and bounded; copy gives RenderedPage
        // independent ownership before this bitmap is destroyed.
        let pixels = unsafe { slice::from_raw_parts(pointer.cast::<u8>(), length) }.to_vec();
        Ok(RenderedPage {
            pixels,
            width: self.width,
            height: self.height,
            placement,
        })
    }
}

impl Drop for PdfBitmap<'_> {
    fn drop(&mut self) {
        // SAFETY: Handle is owned by this bitmap wrapper.
        unsafe { (self.library.api.bitmap_destroy)(self.handle) };
    }
}

struct PrinterDevice {
    hdc: HDC,
    printable_width: i32,
    printable_height: i32,
    dpi_x: i32,
    dpi_y: i32,
}

impl PrinterDevice {
    fn create(printer: &str, devmode: &[u8]) -> Result<Self, ExecutorError> {
        let printer = wide(printer);
        let driver = wide("WINSPOOL");
        let aligned = AlignedDevmode::new(devmode);
        // SAFETY: All UTF-16 strings and the aligned complete DEVMODE live
        // through the synchronous CreateDCW call.
        let hdc = unsafe {
            CreateDCW(
                driver.as_ptr(),
                printer.as_ptr(),
                ptr::null(),
                aligned.as_ptr(),
            )
        };
        if hdc.is_null() {
            return Err(last_win32_error(
                "windows_printer_dc_failed",
                "the driver rejected the normalized profile DEVMODE",
                false,
            ));
        }
        // SAFETY: HDC is live.
        let get = |index: u32| unsafe { GetDeviceCaps(hdc, index as i32) };
        let device = Self {
            hdc,
            printable_width: get(HORZRES),
            printable_height: get(VERTRES),
            dpi_x: get(LOGPIXELSX),
            dpi_y: get(LOGPIXELSY),
        };
        if device.printable_width <= 0
            || device.printable_height <= 0
            || device.dpi_x <= 0
            || device.dpi_y <= 0
        {
            return Err(executor_error(
                "windows_printer_metrics_invalid",
                "printer driver returned invalid printable-area metrics",
                false,
                false,
            ));
        }
        Ok(device)
    }
}

impl Drop for PrinterDevice {
    fn drop(&mut self) {
        if !self.hdc.is_null() {
            // SAFETY: HDC is exclusively owned by this wrapper.
            unsafe { DeleteDC(self.hdc) };
        }
    }
}

struct Renderer {
    printable_width: i32,
    printable_height: i32,
    device_dpi_x: i32,
    device_dpi_y: i32,
    render_dpi: i32,
}

impl Renderer {
    fn new(device: &PrinterDevice) -> Result<Self, ExecutorError> {
        let configured = std::env::var("PIQAE_WINDOWS_MAX_RENDER_DPI")
            .ok()
            .map(|value| value.parse::<i32>())
            .transpose()
            .map_err(|_| {
                executor_error(
                    "windows_render_dpi_invalid",
                    "PIQAE_WINDOWS_MAX_RENDER_DPI must be an integer",
                    false,
                    false,
                )
            })?
            .unwrap_or(DEFAULT_MAX_RENDER_DPI);
        if !(72..=MAX_CONFIGURED_RENDER_DPI).contains(&configured) {
            return Err(executor_error(
                "windows_render_dpi_invalid",
                "PIQAE_WINDOWS_MAX_RENDER_DPI must be between 72 and 2400",
                false,
                false,
            ));
        }
        Ok(Self {
            printable_width: device.printable_width,
            printable_height: device.printable_height,
            device_dpi_x: device.dpi_x,
            device_dpi_y: device.dpi_y,
            render_dpi: configured.min(device.dpi_x.max(device.dpi_y)),
        })
    }

    fn placement(
        &self,
        width_points: f32,
        height_points: f32,
    ) -> Result<RenderPlacement, ExecutorError> {
        let physical_width = (width_points * self.device_dpi_x as f32 / 72.0).round();
        let physical_height = (height_points * self.device_dpi_y as f32 / 72.0).round();
        let fit_scale = (self.printable_width as f32 / physical_width)
            .min(self.printable_height as f32 / physical_height);
        let fitted_width = (physical_width * fit_scale).round().max(1.0) as i32;
        let fitted_height = (physical_height * fit_scale).round().max(1.0) as i32;
        let bitmap_width = (width_points * self.render_dpi as f32 / 72.0)
            .round()
            .max(1.0) as i32;
        let bitmap_height = (height_points * self.render_dpi as f32 / 72.0)
            .round()
            .max(1.0) as i32;
        if bitmap_width <= 0 || bitmap_height <= 0 {
            return Err(bitmap_too_large());
        }
        Ok(RenderPlacement {
            bitmap_width,
            bitmap_height,
            natural_width: physical_width.round().max(1.0) as i32,
            natural_height: physical_height.round().max(1.0) as i32,
            fitted_width,
            fitted_height,
            printable_width: self.printable_width,
            printable_height: self.printable_height,
        })
    }
}

#[derive(Clone, Copy)]
struct RenderPlacement {
    bitmap_width: i32,
    bitmap_height: i32,
    natural_width: i32,
    natural_height: i32,
    fitted_width: i32,
    fitted_height: i32,
    printable_width: i32,
    printable_height: i32,
}

struct RenderedPage {
    pixels: Vec<u8>,
    width: i32,
    height: i32,
    placement: RenderPlacement,
}

struct PrintJob {
    device: Option<PrinterDevice>,
    native_job_id: i32,
    active: bool,
}

impl PrintJob {
    fn start(device: PrinterDevice, title: &str) -> Result<Self, ExecutorError> {
        let title = wide(title);
        let info = DOCINFOW {
            cbSize: i32::try_from(mem::size_of::<DOCINFOW>()).unwrap_or(i32::MAX),
            lpszDocName: title.as_ptr(),
            lpszOutput: ptr::null(),
            lpszDatatype: ptr::null(),
            fwType: 0,
        };
        // SAFETY: HDC is live and DOCINFO strings remain valid for the call.
        let native_job_id = unsafe { StartDocW(device.hdc, &info) };
        if native_job_id <= 0 {
            return Err(last_win32_error(
                "windows_start_doc_failed",
                "printer driver did not start the PDF job",
                false,
            ));
        }
        Ok(Self {
            device: Some(device),
            native_job_id,
            active: true,
        })
    }

    fn print_page(&mut self, page: &RenderedPage, fit: bool) -> Result<(), ExecutorError> {
        let Some(device) = self.device.as_ref() else {
            return Err(executor_error(
                "windows_print_state_invalid",
                "printer device was released before page submission",
                false,
                true,
            ));
        };
        // SAFETY: HDC is in an active StartDocW scope.
        if unsafe { StartPage(device.hdc) } <= 0 {
            return Err(last_win32_error(
                "windows_start_page_failed",
                "printer driver did not start a PDF page",
                true,
            ));
        }
        // SAFETY: HDC is active. HALFTONE is the documented high-quality mode
        // for raster scaling into printer device contexts.
        unsafe { SetStretchBltMode(device.hdc, HALFTONE) };
        let (width, height) = if fit {
            (page.placement.fitted_width, page.placement.fitted_height)
        } else {
            (
                page.placement
                    .natural_width
                    .min(page.placement.printable_width),
                page.placement
                    .natural_height
                    .min(page.placement.printable_height),
            )
        };
        // A printer DC's (0, 0) is already the printable-area origin.
        let x = (device.printable_width - width).max(0) / 2;
        let y = (device.printable_height - height).max(0) / 2;
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(mem::size_of::<BITMAPINFOHEADER>()).unwrap_or(u32::MAX),
                biWidth: page.width,
                // Negative height declares the PDFium top-down BGRA buffer.
                biHeight: -page.height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [RGBQUAD {
                rgbBlue: 0,
                rgbGreen: 0,
                rgbRed: 0,
                rgbReserved: 0,
            }],
        };
        // SAFETY: Pixel slice and BITMAPINFO are valid for the synchronous GDI
        // copy. Source dimensions exactly match the PDFium allocation.
        let copied = unsafe {
            StretchDIBits(
                device.hdc,
                x,
                y,
                width,
                height,
                0,
                0,
                page.width,
                page.height,
                page.pixels.as_ptr().cast(),
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            )
        };
        if copied <= 0 {
            return Err(last_win32_error(
                "windows_render_page_failed",
                "GDI did not copy the rendered PDF page to the printer",
                true,
            ));
        }
        // SAFETY: A page was successfully started above.
        if unsafe { EndPage(device.hdc) } <= 0 {
            return Err(last_win32_error(
                "windows_end_page_failed",
                "printer driver did not accept the rendered PDF page",
                true,
            ));
        }
        Ok(())
    }

    fn finish(mut self) -> Result<i32, ExecutorError> {
        let Some(device) = self.device.as_ref() else {
            return Err(executor_error(
                "windows_print_state_invalid",
                "printer device was released before job completion",
                false,
                true,
            ));
        };
        // SAFETY: HDC has a live StartDocW scope.
        if unsafe { EndDoc(device.hdc) } <= 0 {
            return Err(last_win32_error(
                "windows_end_doc_failed",
                "printer driver did not complete the PDF handoff",
                true,
            ));
        }
        self.active = false;
        Ok(self.native_job_id)
    }
}

impl Drop for PrintJob {
    fn drop(&mut self) {
        if self.active
            && let Some(device) = self.device.as_ref()
        {
            // SAFETY: AbortDoc closes the active StartDocW scope after an
            // intermediate failure. The spooler may already have observed it.
            unsafe { AbortDoc(device.hdc) };
        }
    }
}

struct AlignedDevmode {
    words: Vec<usize>,
}

impl AlignedDevmode {
    fn new(bytes: &[u8]) -> Self {
        let word_size = mem::size_of::<usize>();
        let words = bytes.len().div_ceil(word_size);
        let mut storage = vec![0_usize; words];
        // SAFETY: Allocation contains at least bytes.len() writable bytes and
        // source/destination do not overlap.
        unsafe {
            ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                storage.as_mut_ptr().cast::<u8>(),
                bytes.len(),
            );
        }
        Self { words: storage }
    }

    fn as_ptr(&self) -> *const windows_sys::Win32::Graphics::Gdi::DEVMODEW {
        self.words.as_ptr().cast()
    }
}

fn pdfium_path() -> Result<path::PathBuf, ExecutorError> {
    let candidate = if let Some(configured) = std::env::var_os("PIQAE_WINDOWS_PDFIUM_PATH") {
        let configured = path::PathBuf::from(configured);
        if !configured.is_absolute() {
            return Err(executor_error(
                "windows_pdfium_path_invalid",
                "PIQAE_WINDOWS_PDFIUM_PATH must be an absolute path",
                false,
                false,
            ));
        }
        configured
    } else {
        std::env::current_exe()
            .map_err(|error| {
                executor_error(
                    "windows_pdfium_path_invalid",
                    &format!("could not locate executor: {error}"),
                    false,
                    false,
                )
            })?
            .parent()
            .ok_or_else(|| {
                executor_error(
                    "windows_pdfium_path_invalid",
                    "executor has no containing directory",
                    false,
                    false,
                )
            })?
            .join("pdfium.dll")
    };
    let canonical = candidate.canonicalize().map_err(|error| {
        executor_error(
            "windows_pdfium_missing",
            &format!(
                "PDFium library {} is unavailable: {error}",
                candidate.display()
            ),
            false,
            false,
        )
    })?;
    if !canonical.is_file()
        || canonical
            .file_name()
            .is_none_or(|name| !name.eq_ignore_ascii_case("pdfium.dll"))
    {
        return Err(executor_error(
            "windows_pdfium_path_invalid",
            "PDFium dependency must be a regular file named pdfium.dll",
            false,
            false,
        ));
    }
    Ok(canonical)
}

const fn rotation_index(rotation: Rotation) -> c_int {
    match rotation {
        Rotation::Deg0 => 0,
        Rotation::Deg90 => 1,
        Rotation::Deg180 => 2,
        Rotation::Deg270 => 3,
    }
}

fn bitmap_too_large() -> ExecutorError {
    executor_error(
        "windows_page_bitmap_too_large",
        "rendered page exceeds the 384 MiB per-page safety limit; lower PIQAE_WINDOWS_MAX_RENDER_DPI",
        false,
        false,
    )
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_os(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt as _;
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn profile_error(error: NativeProfileError) -> ExecutorError {
    executor_error(&error.code, &error.message, error.retryable, false)
}

fn mark_handoff(mut error: ExecutorError) -> ExecutorError {
    error.handoff_may_have_succeeded = true;
    error
}

fn last_win32_error(code: &str, message: &str, handoff: bool) -> ExecutorError {
    // SAFETY: GetLastError has no preconditions.
    let native = unsafe { GetLastError() };
    executor_error(
        code,
        &format!("{message} (Win32 error {native})"),
        false,
        handoff,
    )
}

fn executor_error(code: &str, message: &str, retryable: bool, handoff: bool) -> ExecutorError {
    ExecutorError {
        code: code.into(),
        message: message.into(),
        retryable,
        handoff_may_have_succeeded: handoff,
    }
}
