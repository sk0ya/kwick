//! Minimal HTTP client on top of WinHTTP, which ships with Windows: no TLS
//! stack or HTTP crate in the binary. Blocking; call it from a worker thread.

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Networking::WinHttp::*;

pub struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    /// "Name: value" lines
    pub headers: &'a [String],
    pub body: &'a [u8],
}

pub struct Response {
    pub status: u32,
    pub body: Vec<u8>,
}

/// Responses larger than this are cut off (a launcher has no business
/// downloading big files).
const MAX_BODY: usize = 8 * 1024 * 1024;

struct Handle(*mut std::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn check(handle: *mut std::ffi::c_void, what: &str) -> Result<Handle, String> {
    if handle.is_null() {
        Err(format!("{what}: {}", windows::core::Error::from_win32()))
    } else {
        Ok(Handle(handle))
    }
}

pub fn get(url: &str) -> Result<Response, String> {
    send(&Request {
        method: "GET",
        url,
        headers: &[],
        body: &[],
    })
}

pub fn send(req: &Request) -> Result<Response, String> {
    let url_w: Vec<u16> = req.url.encode_utf16().collect();
    let mut host = [0u16; 256];
    let mut path = vec![0u16; url_w.len() + 1];
    let mut extra = vec![0u16; url_w.len() + 1];
    let mut parts = URL_COMPONENTS {
        dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
        lpszHostName: windows::core::PWSTR(host.as_mut_ptr()),
        dwHostNameLength: host.len() as u32,
        lpszUrlPath: windows::core::PWSTR(path.as_mut_ptr()),
        dwUrlPathLength: path.len() as u32,
        lpszExtraInfo: windows::core::PWSTR(extra.as_mut_ptr()),
        dwExtraInfoLength: extra.len() as u32,
        ..Default::default()
    };
    unsafe {
        WinHttpCrackUrl(&url_w, 0, &mut parts).map_err(|e| format!("URL を解釈できません: {e}"))?;
    }
    let host = String::from_utf16_lossy(&host[..parts.dwHostNameLength as usize]);
    // Path plus "?query#fragment" (the extra info).
    let mut path_and_query = String::from_utf16_lossy(&path[..parts.dwUrlPathLength as usize]);
    path_and_query.push_str(&String::from_utf16_lossy(
        &extra[..parts.dwExtraInfoLength as usize],
    ));
    if path_and_query.is_empty() {
        path_and_query.push('/');
    }
    let secure = parts.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS;

    unsafe {
        let session = check(
            WinHttpOpen(
                &HSTRING::from("Kwick"),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ),
            "WinHttpOpen",
        )?;
        let _ = WinHttpSetTimeouts(session.0, 5000, 10000, 15000, 30000);
        let connect = check(
            WinHttpConnect(session.0, &HSTRING::from(host.as_str()), parts.nPort, 0),
            "WinHttpConnect",
        )?;
        let request = check(
            WinHttpOpenRequest(
                connect.0,
                &HSTRING::from(req.method),
                &HSTRING::from(path_and_query.as_str()),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                if secure { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) },
            ),
            "WinHttpOpenRequest",
        )?;
        let headers: Vec<u16> = req.headers.join("\r\n").encode_utf16().collect();
        WinHttpSendRequest(
            request.0,
            (!headers.is_empty()).then_some(headers.as_slice()),
            (!req.body.is_empty()).then(|| req.body.as_ptr() as *const _),
            req.body.len() as u32,
            req.body.len() as u32,
            0,
        )
        .map_err(|e| format!("送信できません: {e}"))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .map_err(|e| format!("応答がありません: {e}"))?;

        let mut status: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut _),
            &mut size,
            std::ptr::null_mut(),
        )
        .map_err(|e| format!("ステータスを読めません: {e}"))?;

        let mut body = Vec::new();
        loop {
            let mut available = 0u32;
            if WinHttpQueryDataAvailable(request.0, &mut available).is_err() || available == 0 {
                break;
            }
            let start = body.len();
            body.resize(start + available as usize, 0);
            let mut read = 0u32;
            if WinHttpReadData(
                request.0,
                body[start..].as_mut_ptr() as *mut _,
                available,
                &mut read,
            )
            .is_err()
            {
                body.truncate(start);
                break;
            }
            body.truncate(start + read as usize);
            if body.len() > MAX_BODY {
                body.truncate(MAX_BODY);
                break;
            }
        }
        Ok(Response { status, body })
    }
}
