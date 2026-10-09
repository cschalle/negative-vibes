use nv_sane_sys as sys;

#[derive(Debug, thiserror::Error)]
pub enum SaneError {
    #[error("SANE status {code}: {message}")]
    Status { code: u32, message: String },
    #[error("SANE returned a null pointer")]
    NullPointer,
    #[error("invalid UTF-8 in SANE string")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("string contains interior NUL byte")]
    Nul(#[from] std::ffi::NulError),
}

pub type Result<T> = std::result::Result<T, SaneError>;

pub(crate) fn check(status: sys::SANE_Status) -> Result<()> {
    if status == sys::SANE_STATUS_GOOD {
        return Ok(());
    }
    let message = unsafe {
        let ptr = sys::sane_strstatus(status);
        if ptr.is_null() { "(unknown)".to_owned() } else { std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned() }
    };
    Err(SaneError::Status { code: status, message })
}
