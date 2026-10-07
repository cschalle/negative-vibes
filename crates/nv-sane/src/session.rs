use std::ffi::CStr;
use nv_sane_sys as sys;
use crate::{Result, SaneError, error::check, handle::Handle};

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name: String,
    pub vendor: String,
    pub model: String,
    pub kind: String,
}

/// Owns the SANE library session (sane_init / sane_exit).
///
/// Must be created and used on the SANE worker thread only; it is not Send.
pub struct Session {
    _version: i32,
    // Prevent Send/Sync so the session cannot be moved across threads.
    _not_send: std::marker::PhantomData<*mut ()>,
}

impl Session {
    /// Initialise libsane. Call once per process lifetime, on the SANE thread.
    pub fn new() -> Result<Self> {
        let mut version: sys::SANE_Int = 0;
        let status = unsafe { sys::sane_init(&mut version, None) };
        check(status)?;
        Ok(Self { _version: version, _not_send: std::marker::PhantomData })
    }

    /// Returns all devices SANE can see. Pass `local_only = true` to skip
    /// network backends (much faster, avoids mDNS timeouts).
    pub fn get_devices(&self, local_only: bool) -> Result<Vec<DeviceInfo>> {
        let mut list: *mut *const sys::SANE_Device = std::ptr::null_mut();
        let status = unsafe {
            sys::sane_get_devices(
                &mut list,
                if local_only { sys::SANE_TRUE } else { sys::SANE_FALSE },
            )
        };
        check(status)?;
        if list.is_null() {
            return Ok(vec![]);
        }
        let mut devices = Vec::new();
        let mut i = 0;
        loop {
            let dev_ptr = unsafe { *list.add(i) };
            if dev_ptr.is_null() {
                break;
            }
            let dev = unsafe { &*dev_ptr };
            devices.push(DeviceInfo {
                name: cstr(dev.name)?,
                vendor: cstr(dev.vendor)?,
                model: cstr(dev.model)?,
                kind: cstr(dev.type_)?,
            });
            i += 1;
        }
        Ok(devices)
    }

    /// Open a device by its SANE name and return a handle to it.
    pub fn open(&self, name: &str) -> Result<Handle> {
        let c_name = std::ffi::CString::new(name)?;
        let mut raw: sys::SANE_Handle = std::ptr::null_mut();
        let status = unsafe { sys::sane_open(c_name.as_ptr(), &mut raw) };
        check(status)?;
        if raw.is_null() {
            return Err(SaneError::NullPointer);
        }
        Ok(unsafe { Handle::from_raw(raw) })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe { sys::sane_exit() };
    }
}

fn cstr(ptr: sys::SANE_String_Const) -> Result<String> {
    if ptr.is_null() {
        return Ok(String::new());
    }
    Ok(unsafe { CStr::from_ptr(ptr) }
        .to_str()?
        .to_owned())
}
