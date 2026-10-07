use nv_sane_sys as sys;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionKind {
    Bool,
    Int,
    Fixed,
    String,
    Button,
    Group,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionUnit {
    None,
    Pixel,
    Bit,
    Mm,
    Dpi,
    Percent,
    Microsecond,
}

impl OptionUnit {
    pub(crate) fn from_raw(raw: sys::SANE_Unit) -> Self {
        match raw {
            sys::SANE_UNIT_PIXEL => Self::Pixel,
            sys::SANE_UNIT_BIT => Self::Bit,
            sys::SANE_UNIT_MM => Self::Mm,
            sys::SANE_UNIT_DPI => Self::Dpi,
            sys::SANE_UNIT_PERCENT => Self::Percent,
            sys::SANE_UNIT_MICROSECOND => Self::Microsecond,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum OptionConstraint {
    None,
    Range { min: f64, max: f64, quant: f64 },
    WordList(Vec<f64>),
    StringList(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct OptionDescriptor {
    pub index: usize,
    pub name: String,
    pub title: String,
    pub desc: String,
    pub kind: OptionKind,
    pub unit: OptionUnit,
    /// Size in bytes of the option value buffer.
    pub size: usize,
    /// Raw SANE_CAP_* flags.
    pub cap: i32,
    pub constraint: OptionConstraint,
}

impl OptionDescriptor {
    pub fn is_active(&self) -> bool {
        self.cap & sys::SANE_CAP_INACTIVE == 0
    }
    pub fn is_settable(&self) -> bool {
        self.cap & sys::SANE_CAP_SOFT_SELECT != 0
    }
    pub fn is_hardware_select(&self) -> bool {
        self.cap & sys::SANE_CAP_HARD_SELECT != 0
    }
    pub fn is_advanced(&self) -> bool {
        self.cap & sys::SANE_CAP_ADVANCED != 0
    }
}

#[derive(Debug, Clone)]
pub enum OptionValue {
    Bool(bool),
    Int(i32),
    Fixed(f64),
    String(String),
    Button,
}

/// Flags returned by `sane_control_option` SET.
#[derive(Debug, Default)]
pub struct SetInfo {
    pub inexact: bool,
    pub reload_options: bool,
    pub reload_params: bool,
}

impl SetInfo {
    pub(crate) fn from_raw(info: sys::SANE_Int) -> Self {
        Self {
            inexact: info & sys::SANE_INFO_INEXACT != 0,
            reload_options: info & sys::SANE_INFO_RELOAD_OPTIONS != 0,
            reload_params: info & sys::SANE_INFO_RELOAD_PARAMS != 0,
        }
    }
}
