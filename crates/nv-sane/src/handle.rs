use crate::{
    Result, SaneError,
    error::check,
    option::{OptionConstraint, OptionDescriptor, OptionKind, OptionUnit, OptionValue, SetInfo},
    parameters::Parameters,
};
use nv_sane_sys as sys;
use std::ffi::{CStr, CString};
use std::os::raw::c_void;
use std::ptr;

/// An opened SANE device handle.
///
/// All methods must be called on the SANE worker thread.  The handle is `!Send`.
pub struct Handle {
    raw: sys::SANE_Handle,
    _not_send: std::marker::PhantomData<*mut ()>,
}

impl Handle {
    pub(crate) unsafe fn from_raw(raw: sys::SANE_Handle) -> Self {
        Self { raw, _not_send: std::marker::PhantomData }
    }

    // ── Options ──────────────────────────────────────────────────────────────

    /// Number of options (reads option 0, which always gives the count).
    pub fn option_count(&self) -> Result<usize> {
        let mut count: sys::SANE_Int = 0;
        let status = unsafe { sys::sane_control_option(self.raw, 0, sys::SANE_ACTION_GET_VALUE, &mut count as *mut sys::SANE_Int as *mut c_void, ptr::null_mut()) };
        check(status)?;
        Ok(count as usize)
    }

    /// Descriptor for option at `index`.  Returns `None` if the backend does
    /// not provide a descriptor for that slot (legal per SANE spec).
    pub fn option_descriptor(&self, index: usize) -> Option<OptionDescriptor> {
        let raw = unsafe { sys::sane_get_option_descriptor(self.raw, index as sys::SANE_Int) };
        if raw.is_null() {
            return None;
        }
        let desc = unsafe { &*raw };

        let kind = match desc.type_ {
            sys::SANE_TYPE_BOOL => OptionKind::Bool,
            sys::SANE_TYPE_INT => OptionKind::Int,
            sys::SANE_TYPE_FIXED => OptionKind::Fixed,
            sys::SANE_TYPE_STRING => OptionKind::String,
            sys::SANE_TYPE_BUTTON => OptionKind::Button,
            sys::SANE_TYPE_GROUP => OptionKind::Group,
            _ => return None,
        };

        let is_fixed = kind == OptionKind::Fixed;

        let constraint = match desc.constraint_type {
            sys::SANE_CONSTRAINT_RANGE => {
                let rp = unsafe { desc.constraint.range };
                if rp.is_null() {
                    OptionConstraint::None
                } else {
                    let r = unsafe { &*rp };
                    let (min, max, quant) =
                        if is_fixed { (sys::sane_unfix(r.min), sys::sane_unfix(r.max), sys::sane_unfix(r.quant)) } else { (r.min as f64, r.max as f64, r.quant as f64) };
                    OptionConstraint::Range { min, max, quant }
                }
            }
            sys::SANE_CONSTRAINT_WORD_LIST => {
                let lp = unsafe { desc.constraint.word_list };
                if lp.is_null() {
                    OptionConstraint::None
                } else {
                    let count = unsafe { *lp } as usize;
                    let values: Vec<f64> = (1..=count)
                        .map(|i| {
                            let v = unsafe { *lp.add(i) };
                            if is_fixed { sys::sane_unfix(v) } else { v as f64 }
                        })
                        .collect();
                    OptionConstraint::WordList(values)
                }
            }
            sys::SANE_CONSTRAINT_STRING_LIST => {
                let lp = unsafe { desc.constraint.string_list };
                let mut strings = Vec::new();
                if !lp.is_null() {
                    let mut i = 0usize;
                    loop {
                        let sp = unsafe { *lp.add(i) };
                        if sp.is_null() {
                            break;
                        }
                        let s = unsafe { CStr::from_ptr(sp) }.to_string_lossy().into_owned();
                        strings.push(s);
                        i += 1;
                    }
                }
                OptionConstraint::StringList(strings)
            }
            _ => OptionConstraint::None,
        };

        Some(OptionDescriptor {
            index,
            name: cstr_opt(desc.name),
            title: cstr_opt(desc.title),
            desc: cstr_opt(desc.desc),
            kind,
            unit: OptionUnit::from_raw(desc.unit),
            size: desc.size as usize,
            cap: desc.cap,
            constraint,
        })
    }

    /// Read all option descriptors at once.
    pub fn all_options(&self) -> Result<Vec<OptionDescriptor>> {
        let count = self.option_count()?;
        let mut opts = Vec::with_capacity(count);
        for i in 1..count {
            // slot 0 is the count pseudo-option
            if let Some(d) = self.option_descriptor(i) {
                opts.push(d);
            }
        }
        Ok(opts)
    }

    /// Get the current value of option `index`.
    pub fn get_option(&self, desc: &OptionDescriptor) -> Result<OptionValue> {
        let index = desc.index as sys::SANE_Int;
        match desc.kind {
            OptionKind::Bool => {
                let mut v: sys::SANE_Bool = 0;
                let status = unsafe { sys::sane_control_option(self.raw, index, sys::SANE_ACTION_GET_VALUE, &mut v as *mut sys::SANE_Bool as *mut c_void, ptr::null_mut()) };
                check(status)?;
                Ok(OptionValue::Bool(v != sys::SANE_FALSE))
            }
            OptionKind::Int => {
                let mut v: sys::SANE_Int = 0;
                let status = unsafe { sys::sane_control_option(self.raw, index, sys::SANE_ACTION_GET_VALUE, &mut v as *mut sys::SANE_Int as *mut c_void, ptr::null_mut()) };
                check(status)?;
                Ok(OptionValue::Int(v))
            }
            OptionKind::Fixed => {
                let mut v: sys::SANE_Fixed = 0;
                let status = unsafe { sys::sane_control_option(self.raw, index, sys::SANE_ACTION_GET_VALUE, &mut v as *mut sys::SANE_Fixed as *mut c_void, ptr::null_mut()) };
                check(status)?;
                Ok(OptionValue::Fixed(sys::sane_unfix(v)))
            }
            OptionKind::String => {
                let mut buf = vec![0u8; desc.size.max(256)];
                let status = unsafe { sys::sane_control_option(self.raw, index, sys::SANE_ACTION_GET_VALUE, buf.as_mut_ptr() as *mut c_void, ptr::null_mut()) };
                check(status)?;
                let s = CStr::from_bytes_until_nul(&buf).map(|c| c.to_string_lossy().into_owned()).unwrap_or_default();
                Ok(OptionValue::String(s))
            }
            OptionKind::Button | OptionKind::Group => Ok(OptionValue::Button),
        }
    }

    /// Set option `index` to `value`.  Returns flags indicating side effects.
    pub fn set_option(&self, desc: &OptionDescriptor, value: &OptionValue) -> Result<SetInfo> {
        let index = desc.index as sys::SANE_Int;
        let mut info: sys::SANE_Int = 0;
        let status = match value {
            OptionValue::Bool(b) => {
                let mut v: sys::SANE_Bool = if *b { sys::SANE_TRUE } else { sys::SANE_FALSE };
                unsafe { sys::sane_control_option(self.raw, index, sys::SANE_ACTION_SET_VALUE, &mut v as *mut sys::SANE_Bool as *mut c_void, &mut info) }
            }
            OptionValue::Int(n) => {
                let mut v: sys::SANE_Int = *n;
                unsafe { sys::sane_control_option(self.raw, index, sys::SANE_ACTION_SET_VALUE, &mut v as *mut sys::SANE_Int as *mut c_void, &mut info) }
            }
            OptionValue::Fixed(f) => {
                let mut v: sys::SANE_Fixed = sys::sane_fix(*f);
                unsafe { sys::sane_control_option(self.raw, index, sys::SANE_ACTION_SET_VALUE, &mut v as *mut sys::SANE_Fixed as *mut c_void, &mut info) }
            }
            OptionValue::String(s) => {
                let c = CString::new(s.as_str())?;
                let mut buf = c.into_bytes_with_nul();
                buf.resize(desc.size.max(buf.len()), 0);
                unsafe { sys::sane_control_option(self.raw, index, sys::SANE_ACTION_SET_VALUE, buf.as_mut_ptr() as *mut c_void, &mut info) }
            }
            OptionValue::Button => {
                let mut v: sys::SANE_Int = 0;
                unsafe { sys::sane_control_option(self.raw, index, sys::SANE_ACTION_SET_VALUE, &mut v as *mut sys::SANE_Int as *mut c_void, &mut info) }
            }
        };
        check(status)?;
        Ok(SetInfo::from_raw(info))
    }

    /// Set an option by name. Returns `Ok(None)` if the option does not exist.
    pub fn set_option_by_name(&self, name: &str, value: &OptionValue) -> Result<Option<SetInfo>> {
        let count = self.option_count()?;
        for i in 1..count {
            if let Some(desc) = self.option_descriptor(i) {
                if desc.name == name {
                    return self.set_option(&desc, value).map(Some);
                }
            }
        }
        Ok(None)
    }

    // ── Scan ─────────────────────────────────────────────────────────────────

    /// Query scan parameters (call after `start`, when they are final).
    pub fn get_parameters(&self) -> Result<Parameters> {
        let mut raw: sys::SANE_Parameters = sys::SANE_Parameters { format: 0, last_frame: sys::SANE_FALSE, bytes_per_line: 0, pixels_per_line: 0, lines: 0, depth: 0 };
        let status = unsafe { sys::sane_get_parameters(self.raw, &mut raw) };
        check(status)?;
        Parameters::from_raw(raw).ok_or(SaneError::Status { code: 0, message: "unsupported SANE frame format".into() })
    }

    /// Begin a scan frame. Call `get_parameters` after this to get final dimensions.
    pub fn start(&self) -> Result<()> {
        check(unsafe { sys::sane_start(self.raw) })
    }

    /// Read up to `buf.len()` bytes. Returns the number of bytes actually read.
    /// Returns `Ok(0)` on EOF (end of frame).
    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let mut len: sys::SANE_Int = 0;
        let status = unsafe { sys::sane_read(self.raw, buf.as_mut_ptr(), buf.len() as sys::SANE_Int, &mut len) };
        match status {
            sys::SANE_STATUS_GOOD => Ok(len as usize),
            sys::SANE_STATUS_EOF => Ok(0),
            _ => Err(crate::error::check(status).unwrap_err()),
        }
    }

    /// Cancel a running scan. Safe to call from any thread for backends that
    /// document it (including plustek8200i).
    pub fn cancel(&self) {
        unsafe { sys::sane_cancel(self.raw) }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { sys::sane_close(self.raw) }
    }
}

fn cstr_opt(ptr: sys::SANE_String_Const) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
}
