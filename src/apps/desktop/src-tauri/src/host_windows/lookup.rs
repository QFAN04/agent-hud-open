//! Win32 known-folder and NSIS registry lookup for installed Windows builds.

use std::{
    ffi::{c_void, OsStr, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
};

#[repr(C)]
#[allow(dead_code)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

// FOLDERID_LocalAppData: {F1B32785-6FBA-4FCF-9D55-7B8E7F157091}
const FOLDERID_LOCAL_APP_DATA: Guid = Guid {
    data1: 0xF1B3_2785,
    data2: 0x6FBA,
    data3: 0x4FCF,
    data4: [0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91],
};
const KF_FLAG_NO_ALIAS: u32 = 0x0000_1000;
const S_OK: i32 = 0;
const HKEY_CURRENT_USER: *mut c_void = (-2_147_483_647i32 as isize) as *mut c_void;
const RRF_RT_REG_SZ: u32 = 0x0000_0002;
const RRF_RT_REG_EXPAND_SZ: u32 = 0x0000_0004;
const ERROR_SUCCESS: u32 = 0;

#[link(name = "shell32")]
unsafe extern "system" {
    fn SHGetKnownFolderPath(
        rfid: *const Guid,
        dw_flags: u32,
        h_token: *mut c_void,
        ppsz_path: *mut *mut u16,
    ) -> i32;
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoTaskMemFree(pv: *mut c_void);
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn RegGetValueW(
        hkey: *mut c_void,
        lp_sub_key: *const u16,
        lp_value: *const u16,
        dw_flags: u32,
        pdw_type: *mut u32,
        pv_data: *mut c_void,
        pcb_data: *mut u32,
    ) -> u32;
}

pub(super) fn approved_install_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(local_appdata) = real_local_appdata() {
        dirs.push(local_appdata.join("Agent HUD"));
    }
    for (subkey, value) in [
        (r"Software\agenthud\Agent HUD", None),
        (
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Agent HUD",
            Some("InstallLocation"),
        ),
    ] {
        if let Some(dir) = read_hkcu_string(subkey, value)
            .as_deref()
            .and_then(parse_registry_install_dir)
        {
            dirs.push(dir);
        }
    }
    dirs
}

pub(super) fn parse_registry_install_dir(raw: &str) -> Option<PathBuf> {
    let clean = raw.trim().trim_matches('"').trim();
    (!clean.is_empty()).then(|| PathBuf::from(clean))
}

fn wide_null(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain(Some(0)).collect()
}

fn read_hkcu_string(subkey: &str, value_name: Option<&str>) -> Option<String> {
    let subkey_w = wide_null(subkey);
    let value_w = value_name.map(wide_null);
    let value_ptr = value_w.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());
    for flags in [RRF_RT_REG_SZ, RRF_RT_REG_EXPAND_SZ] {
        let mut bytes: u32 = 0;
        // SAFETY: Valid NUL-terminated UTF-16 pointers query the value size in bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey_w.as_ptr(),
                value_ptr,
                flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut bytes,
            )
        };
        if status != ERROR_SUCCESS || bytes < 2 {
            continue;
        }
        let mut buf = vec![0u16; (bytes as usize).div_ceil(2) + 1];
        let mut actual = (buf.len() * 2) as u32;
        // SAFETY: `buf` fits `actual` bytes and receives a NUL-terminated UTF-16 string.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey_w.as_ptr(),
                value_ptr,
                flags,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut actual,
            )
        };
        if status == ERROR_SUCCESS {
            let len = buf.iter().position(|&ch| ch == 0).unwrap_or(buf.len());
            return Some(
                OsString::from_wide(&buf[..len])
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    None
}

fn real_local_appdata() -> Option<PathBuf> {
    let mut raw_path: *mut u16 = std::ptr::null_mut();
    // SAFETY: FOLDERID_LOCAL_APP_DATA is static and raw_path receives a CoTaskMem-allocated string.
    let hr = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_LOCAL_APP_DATA,
            KF_FLAG_NO_ALIAS,
            std::ptr::null_mut(),
            &mut raw_path,
        )
    };
    if raw_path.is_null() {
        return None;
    }
    // SAFETY: Non-null raw_path is a NUL-terminated UTF-16 buffer owned until CoTaskMemFree.
    let path = unsafe {
        let mut len = 0usize;
        while *raw_path.add(len) != 0 {
            len += 1;
        }
        let wide = std::slice::from_raw_parts(raw_path, len);
        let converted = PathBuf::from(OsString::from_wide(wide));
        CoTaskMemFree(raw_path.cast());
        converted
    };
    (hr == S_OK && !path.as_os_str().is_empty()).then_some(path)
}
