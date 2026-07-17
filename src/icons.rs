use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

/// Shell icons for result rows, extracted via SHGetFileInfoW on a worker
/// thread and cached as egui textures keyed by the source path.
///
/// `get` never blocks: unknown paths are queued for the worker and return
/// None (the caller draws a fallback); the worker repaints when done.
pub struct IconCache {
    ready: Arc<Mutex<HashMap<String, Option<egui::TextureHandle>>>>,
    requested: HashSet<String>,
    tx: Sender<String>,
}

impl IconCache {
    pub fn new(ctx: egui::Context) -> Self {
        let ready: Arc<Mutex<HashMap<String, Option<egui::TextureHandle>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        {
            let ready = ready.clone();
            std::thread::spawn(move || {
                // SHGetFileInfoW wants COM initialized on its thread.
                unsafe {
                    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
                    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                }
                while let Ok(path) = rx.recv() {
                    let texture = extract_rgba(&path).map(|(pixels, w, h)| {
                        let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels);
                        ctx.load_texture(&path, image, egui::TextureOptions::LINEAR)
                    });
                    ready.lock().unwrap().insert(path, texture);
                    ctx.request_repaint();
                }
            });
        }
        Self {
            ready,
            requested: HashSet::new(),
            tx,
        }
    }

    pub fn get(&mut self, path: &str) -> Option<egui::TextureHandle> {
        if let Some(cached) = self.ready.lock().unwrap().get(path) {
            return cached.clone();
        }
        if self.requested.insert(path.to_string()) {
            let _ = self.tx.send(path.to_string());
        }
        None
    }
}

/// Ask the shell for the file's icon and convert it to RGBA pixels.
fn extract_rgba(path: &str) -> Option<(Vec<u8>, usize, usize)> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
    use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;

    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut info = SHFILEINFOW::default();
    unsafe {
        let res = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
        if res == 0 || info.hIcon.is_invalid() {
            return None;
        }
        let rgba = icon_to_rgba(info.hIcon);
        let _ = DestroyIcon(info.hIcon);
        rgba
    }
}

unsafe fn icon_to_rgba(
    hicon: windows::Win32::UI::WindowsAndMessaging::HICON,
) -> Option<(Vec<u8>, usize, usize)> {
    use windows::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};

    let mut icon_info = ICONINFO::default();
    if GetIconInfo(hicon, &mut icon_info).is_err() {
        return None;
    }
    let hbm_color = icon_info.hbmColor;
    let hbm_mask = icon_info.hbmMask;

    let result = (|| {
        if hbm_color.is_invalid() {
            return None; // monochrome icon; not worth handling
        }
        let mut bmp = BITMAP::default();
        if GetObjectW(
            hbm_color.into(),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bmp as *mut BITMAP as *mut _),
        ) == 0
        {
            return None;
        }
        let (w, h) = (bmp.bmWidth, bmp.bmHeight);
        if w <= 0 || h <= 0 {
            return None;
        }

        let hdc = GetDC(None);
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let got = GetDIBits(
            hdc,
            hbm_color,
            0,
            h as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        // Icons without an alpha channel report all-zero alpha; recover
        // transparency from the AND mask instead.
        let mask_pixels = if got != 0 && pixels.chunks_exact(4).all(|px| px[3] == 0) {
            let mut mask = vec![0u8; (w * h * 4) as usize];
            let mut mask_bmi = bmi;
            mask_bmi.bmiHeader.biHeight = -h;
            let ok = GetDIBits(
                hdc,
                hbm_mask,
                0,
                h as u32,
                Some(mask.as_mut_ptr() as *mut _),
                &mut mask_bmi,
                DIB_RGB_COLORS,
            );
            (ok != 0).then_some(mask)
        } else {
            None
        };
        ReleaseDC(None, hdc);
        if got == 0 {
            return None;
        }

        // BGRA -> RGBA
        for px in pixels.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        if let Some(mask) = mask_pixels {
            for (px, m) in pixels.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                px[3] = if m[0] == 0 { 255 } else { 0 };
            }
        }
        Some((pixels, w as usize, h as usize))
    })();

    let _ = DeleteObject(hbm_color.into());
    let _ = DeleteObject(hbm_mask.into());
    result
}
