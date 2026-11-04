//! Reading a web address with WinHTTP, the system's own HTTP client (it
//! follows the system's proxy settings and needs no TLS library in the
//! executable). Used only for the update check.

use std::ffi::c_void;

use anyhow::Context as _;
use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect,
    WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::core::{HSTRING, PCWSTR, w};

/// The most that is read (GitHub's answer about a release is a few KB).
const MAX_BYTES: usize = 1 << 20;

/// Timeouts of each step (resolving, connecting, sending, receiving), in
/// milliseconds.
const TIMEOUT_MS: i32 = 10_000;

/// A WinHTTP handle, closed when dropped.
struct Handle(*mut c_void);

impl Handle {
    /// Wraps what a WinHTTP function returned (null on failure).
    fn new(raw: *mut c_void, what: &str) -> anyhow::Result<Self> {
        if raw.is_null() {
            Err(windows::core::Error::from_thread()).context(what.to_owned())
        } else {
            Ok(Self(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: a handle that WinHTTP gave us and nothing else closes.
        let _ = unsafe { WinHttpCloseHandle(self.0) };
    }
}

/// Reads `url` (https only) as text. Fails unless the server answers 200.
pub fn get(url: &str) -> anyhow::Result<String> {
    let rest = url
        .strip_prefix("https://")
        .context("not an https address")?;
    let (host, path) = match rest.split_once('/') {
        Some((host, path)) => (host, format!("/{path}")),
        None => (rest, "/".to_owned()),
    };
    let agent = HSTRING::from(crate::update::user_agent());
    let host = HSTRING::from(host);
    let path = HSTRING::from(path);
    let headers: Vec<u16> = "Accept: application/vnd.github+json\r\n"
        .encode_utf16()
        .collect();
    // SAFETY: WinHTTP calls with valid strings and buffers; every handle
    // is closed by its guard, the request before the connection before the
    // session (they drop in reverse order).
    unsafe {
        let session = Handle::new(
            WinHttpOpen(
                &agent,
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ),
            "WinHttpOpen",
        )?;
        WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS)
            .context("WinHttpSetTimeouts")?;
        let connection = Handle::new(
            WinHttpConnect(session.0, &host, INTERNET_DEFAULT_HTTPS_PORT, 0),
            "WinHttpConnect",
        )?;
        let request = Handle::new(
            WinHttpOpenRequest(
                connection.0,
                w!("GET"),
                &path,
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ),
            "WinHttpOpenRequest",
        )?;
        WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0)
            .context("cannot send the request")?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).context("no answer")?;
        let mut status = 0u32;
        let mut size = size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(std::ptr::from_mut(&mut status).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .context("no status")?;
        anyhow::ensure!(status == 200, "HTTP status {status}");
        let mut body = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            let mut read = 0u32;
            WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut read,
            )
            .context("cannot read the answer")?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buffer[..read as usize]);
            anyhow::ensure!(body.len() <= MAX_BYTES, "the answer is too long");
        }
        String::from_utf8(body).context("the answer is not UTF-8")
    }
}
