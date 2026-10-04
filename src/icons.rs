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
                    let pixels = match path.strip_prefix(THUMB) {
                        Some(file) => extract_thumbnail(file),
                        None => extract_rgba(&path),
                    };
                    let texture = pixels.map(|(pixels, w, h)| {
                        let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels);
                        ctx.load_texture(&path, image, egui::TextureOptions::LINEAR)
                    });
                    let mut ready = ready.lock().unwrap();
                    if path.starts_with(THUMB) {
                        // Thumbnails are big; keep only the latest one.
                        ready.retain(|k, _| !k.starts_with(THUMB));
                    }
                    ready.insert(path, texture);
                    crate::winctl::wake(&ctx);
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

    /// Shell thumbnail of a picture/video/PDF for the preview pane.
    pub fn thumbnail(&mut self, file: &str) -> Option<egui::TextureHandle> {
        let key = format!("{THUMB}{file}");
        if !self.requested.contains(&key) {
            // The worker drops older thumbnails; let them be requested again.
            self.requested.retain(|k| !k.starts_with(THUMB));
        }
        self.get(&key)
    }
}

const THUMB: &str = "thumb:";

/// A 256px thumbnail from the shell's thumbnail cache (only real
/// thumbnails: files without one return None rather than their icon).
fn extract_thumbnail(path: &str) -> Option<(Vec<u8>, usize, usize)> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT,
        SIIGBF_THUMBNAILONLY,
    };
    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        let hbm = factory
            .GetImage(SIZE { cx: 256, cy: 256 }, SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY)
            .ok()?;
        let result = (|| {
            let mut bmp = BITMAP::default();
            if GetObjectW(
                hbm.into(),
                std::mem::size_of::<BITMAP>() as i32,
                Some(&mut bmp as *mut BITMAP as *mut _),
            ) == 0
            {
                return None;
            }
            let (w, h) = (bmp.bmWidth, bmp.bmHeight.abs());
            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = vec![0u8; (w * h * 4) as usize];
            let hdc = GetDC(None);
            let got = GetDIBits(
                hdc,
                hbm,
                0,
                h as u32,
                Some(pixels.as_mut_ptr() as *mut _),
                &mut bmi,
                DIB_RGB_COLORS,
            );
            ReleaseDC(None, hdc);
            if got == 0 {
                return None;
            }
            // Photos come without alpha; treat all-zero alpha as opaque.
            let opaque = pixels.chunks_exact(4).all(|px| px[3] == 0);
            for px in pixels.chunks_exact_mut(4) {
                px.swap(0, 2);
                if opaque {
                    px[3] = 255;
                }
            }
            Some((pixels, w as usize, h as usize))
        })();
        let _ = DeleteObject(hbm.into());
        result
    }
}

/// Load the app's own embedded icon (see build.rs) and convert it to RGBA
/// pixels, for use as the tray icon and window icon.
pub fn app_icon_rgba() -> Option<(Vec<u8>, usize, usize)> {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyIcon, LoadImageW, HICON, IMAGE_ICON, LR_DEFAULTCOLOR,
    };
    use windows::core::PCWSTR;

    unsafe {
        let hinstance = GetModuleHandleW(None).ok()?;
        // winres embeds the .ico set via `set_icon` as resource id 1.
        let handle = LoadImageW(
            Some(hinstance.into()),
            PCWSTR(1 as *const u16),
            IMAGE_ICON,
            32,
            32,
            LR_DEFAULTCOLOR,
        )
        .ok()?;
        let hicon = HICON(handle.0);
        let rgba = icon_to_rgba(hicon);
        let _ = DestroyIcon(hicon);
        rgba
    }
}

/// Ask the shell for the file's icon and convert it to RGBA pixels.
/// `path` may also be an icon resource location like `C:\x\Vault.dll,-1`
/// (the registry's DefaultIcon format; negative = resource id).
fn extract_rgba(path: &str) -> Option<(Vec<u8>, usize, usize)> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Shell::{
        SHDefExtractIconW, SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON,
    };
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};

    if let Some((file, index)) = parse_icon_location(path) {
        let wide: Vec<u16> = file.encode_utf16().chain(std::iter::once(0)).collect();
        let mut hicon = HICON::default();
        unsafe {
            // Unlike ExtractIconExW this follows MUI redirection (Vault.dll...).
            let res =
                SHDefExtractIconW(PCWSTR(wide.as_ptr()), index, 0, Some(&mut hicon), None, 32);
            if res.is_err() || hicon.is_invalid() {
                return None;
            }
            let rgba = icon_to_rgba(hicon);
            let _ = DestroyIcon(hicon);
            return rgba;
        }
    }

    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut info = SHFILEINFOW::default();
    unsafe {
        // Shell namespace items (shell:AppsFolder\<AUMID>) have no file
        // path; ask by ID list instead.
        let res = if path.starts_with("shell:") {
            use windows::Win32::System::Com::CoTaskMemFree;
            use windows::Win32::UI::Shell::{SHParseDisplayName, SHGFI_PIDL};
            let mut pidl = std::ptr::null_mut();
            if SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None).is_err() {
                return None;
            }
            let res = SHGetFileInfoW(
                PCWSTR(pidl as *const u16),
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_LARGEICON | SHGFI_PIDL,
            );
            CoTaskMemFree(Some(pidl as *const _));
            res
        } else {
            SHGetFileInfoW(
                PCWSTR(wide.as_ptr()),
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_LARGEICON,
            )
        };
        if res == 0 || info.hIcon.is_invalid() {
            return None;
        }
        let rgba = icon_to_rgba(info.hIcon);
        let _ = DestroyIcon(info.hIcon);
        rgba
    }
}

/// Split `file,index` when `file` exists; plain paths return None.
fn parse_icon_location(path: &str) -> Option<(&str, i32)> {
    let (file, index) = path.rsplit_once(',')?;
    let index = index.trim().parse().ok()?;
    std::path::Path::new(file)
        .is_file()
        .then_some((file, index))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_icon_from_resource_location() {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        let path = format!(r"{windir}\System32\Vault.dll,-1");
        assert!(parse_icon_location(&path).is_some());
        assert!(extract_rgba(&path).is_some());
        assert!(parse_icon_location(r"C:\no\such.dll,-1").is_none());
        assert!(parse_icon_location(&format!(r"{windir}\notepad.exe")).is_none());
    }
}
