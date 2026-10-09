//! Raw FFI bindings for libsane.
//!
//! The bindings are pre-generated and committed to source; to regenerate them
//! from the installed sane-backends headers, run:
//!
//! ```text
//! cargo build -p nv-sane-sys --features regenerate-bindings
//! ```
//!
//! Requires `sane-backends-devel` (or equivalent) to be installed.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

// Pre-generated bindings are included here.  Until the first regeneration this
// file contains only the handful of constants and types needed for compilation.

pub const SANE_TRUE: SANE_Bool = 1;
pub const SANE_FALSE: SANE_Bool = 0;

pub const SANE_STATUS_GOOD: SANE_Status = 0;
pub const SANE_STATUS_UNSUPPORTED: SANE_Status = 1;
pub const SANE_STATUS_CANCELLED: SANE_Status = 2;
pub const SANE_STATUS_DEVICE_BUSY: SANE_Status = 3;
pub const SANE_STATUS_INVAL: SANE_Status = 4;
pub const SANE_STATUS_EOF: SANE_Status = 5;
pub const SANE_STATUS_JAMMED: SANE_Status = 6;
pub const SANE_STATUS_NO_DOCS: SANE_Status = 7;
pub const SANE_STATUS_COVER_OPEN: SANE_Status = 8;
pub const SANE_STATUS_IO_ERROR: SANE_Status = 9;
pub const SANE_STATUS_NO_MEM: SANE_Status = 10;
pub const SANE_STATUS_ACCESS_DENIED: SANE_Status = 11;

pub const SANE_FRAME_GRAY: SANE_Frame = 0;
pub const SANE_FRAME_RGB: SANE_Frame = 1;
pub const SANE_FRAME_RED: SANE_Frame = 2;
pub const SANE_FRAME_GREEN: SANE_Frame = 3;
pub const SANE_FRAME_BLUE: SANE_Frame = 4;
/// RGBI frame (extension, used by some backends).
pub const SANE_FRAME_RGBI: SANE_Frame = 0x10;

pub const SANE_INFO_INEXACT: SANE_Int = 1;
pub const SANE_INFO_RELOAD_OPTIONS: SANE_Int = 2;
pub const SANE_INFO_RELOAD_PARAMS: SANE_Int = 4;

pub const SANE_CAP_SOFT_SELECT: SANE_Int = 1;
pub const SANE_CAP_HARD_SELECT: SANE_Int = 2;
pub const SANE_CAP_SOFT_DETECT: SANE_Int = 4;
pub const SANE_CAP_EMULATED: SANE_Int = 8;
pub const SANE_CAP_AUTOMATIC: SANE_Int = 16;
pub const SANE_CAP_INACTIVE: SANE_Int = 32;
pub const SANE_CAP_ADVANCED: SANE_Int = 64;

pub const SANE_CONSTRAINT_NONE: SANE_Constraint_Type = 0;
pub const SANE_CONSTRAINT_RANGE: SANE_Constraint_Type = 1;
pub const SANE_CONSTRAINT_WORD_LIST: SANE_Constraint_Type = 2;
pub const SANE_CONSTRAINT_STRING_LIST: SANE_Constraint_Type = 3;

pub const SANE_TYPE_BOOL: SANE_Value_Type = 0;
pub const SANE_TYPE_INT: SANE_Value_Type = 1;
pub const SANE_TYPE_FIXED: SANE_Value_Type = 2;
pub const SANE_TYPE_STRING: SANE_Value_Type = 3;
pub const SANE_TYPE_BUTTON: SANE_Value_Type = 4;
pub const SANE_TYPE_GROUP: SANE_Value_Type = 5;

pub const SANE_UNIT_NONE: SANE_Unit = 0;
pub const SANE_UNIT_PIXEL: SANE_Unit = 1;
pub const SANE_UNIT_BIT: SANE_Unit = 2;
pub const SANE_UNIT_MM: SANE_Unit = 3;
pub const SANE_UNIT_DPI: SANE_Unit = 4;
pub const SANE_UNIT_PERCENT: SANE_Unit = 5;
pub const SANE_UNIT_MICROSECOND: SANE_Unit = 6;

pub const SANE_ACTION_GET_VALUE: SANE_Action = 0;
pub const SANE_ACTION_SET_VALUE: SANE_Action = 1;
pub const SANE_ACTION_SET_AUTO: SANE_Action = 2;

// --- Primitive typedefs -------------------------------------------------------

pub type SANE_Byte = u8;
pub type SANE_Word = i32;
pub type SANE_Bool = SANE_Word;
pub type SANE_Int = SANE_Word;
pub type SANE_Fixed = SANE_Word;
pub type SANE_String = *mut std::os::raw::c_char;
pub type SANE_String_Const = *const std::os::raw::c_char;
pub type SANE_Handle = *mut std::os::raw::c_void;
pub type SANE_Status = std::os::raw::c_uint;
pub type SANE_Frame = std::os::raw::c_uint;
pub type SANE_Value_Type = std::os::raw::c_uint;
pub type SANE_Unit = std::os::raw::c_uint;
pub type SANE_Constraint_Type = std::os::raw::c_uint;
pub type SANE_Action = std::os::raw::c_uint;

// --- Structures ---------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone)]
pub struct SANE_Device {
    pub name: SANE_String_Const,
    pub vendor: SANE_String_Const,
    pub model: SANE_String_Const,
    pub type_: SANE_String_Const,
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct SANE_Parameters {
    pub format: SANE_Frame,
    pub last_frame: SANE_Bool,
    pub bytes_per_line: SANE_Int,
    pub pixels_per_line: SANE_Int,
    pub lines: SANE_Int,
    pub depth: SANE_Int,
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct SANE_Range {
    pub min: SANE_Word,
    pub max: SANE_Word,
    pub quant: SANE_Word,
}

#[repr(C)]
pub union SANE_Option_Descriptor_constraint {
    pub string_list: *mut SANE_String_Const,
    pub word_list: *const SANE_Word,
    pub range: *const SANE_Range,
}

#[repr(C)]
pub struct SANE_Option_Descriptor {
    pub name: SANE_String_Const,
    pub title: SANE_String_Const,
    pub desc: SANE_String_Const,
    pub type_: SANE_Value_Type,
    pub unit: SANE_Unit,
    pub size: SANE_Int,
    pub cap: SANE_Int,
    pub constraint_type: SANE_Constraint_Type,
    pub constraint: SANE_Option_Descriptor_constraint,
}

// --- Function declarations ----------------------------------------------------

unsafe extern "C" {
    pub fn sane_init(
        version_code: *mut SANE_Int,
        authorize: Option<unsafe extern "C" fn(resource: SANE_String_Const, username: SANE_String, password: SANE_String)>,
    ) -> SANE_Status;

    pub fn sane_exit();

    pub fn sane_get_devices(device_list: *mut *mut *const SANE_Device, local_only: SANE_Bool) -> SANE_Status;

    pub fn sane_open(name: SANE_String_Const, handle: *mut SANE_Handle) -> SANE_Status;

    pub fn sane_close(handle: SANE_Handle);

    pub fn sane_get_option_descriptor(handle: SANE_Handle, option: SANE_Int) -> *const SANE_Option_Descriptor;

    pub fn sane_control_option(handle: SANE_Handle, option: SANE_Int, action: SANE_Action, value: *mut std::os::raw::c_void, info: *mut SANE_Int) -> SANE_Status;

    pub fn sane_get_parameters(handle: SANE_Handle, params: *mut SANE_Parameters) -> SANE_Status;

    pub fn sane_start(handle: SANE_Handle) -> SANE_Status;

    pub fn sane_read(handle: SANE_Handle, data: *mut SANE_Byte, max_length: SANE_Int, length: *mut SANE_Int) -> SANE_Status;

    pub fn sane_cancel(handle: SANE_Handle);

    pub fn sane_strstatus(status: SANE_Status) -> SANE_String_Const;
}

// SANE_FIX / SANE_UNFIX helpers (1 fixed-point unit = 1/65536)
pub const fn sane_fix(v: f64) -> SANE_Fixed {
    (v * 65536.0) as SANE_Fixed
}

pub const fn sane_unfix(v: SANE_Fixed) -> f64 {
    v as f64 / 65536.0
}
