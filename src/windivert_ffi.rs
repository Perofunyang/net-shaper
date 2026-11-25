use std::ffi::{c_char, c_void, CStr, CString};
use std::io;
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

pub const WINDIVERT_MTU_MAX: usize = 65575;
const WINDIVERT_LAYER_NETWORK: u32 = 0;
const WINDIVERT_SHUTDOWN_RECV: u32 = 1;
const WINDIVERT_SHUTDOWN_BOTH: u32 = 3;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WindivertAddress {
    pub timestamp: i64,
    pub layer: u8,
    pub event: u8,
    pub flags: u8, // bit 0: Sniffed, bit 1: Outbound, bit 2: Loopback
    pub reserved1: u8,
    pub reserved2: u32,
    pub data: [u8; 64],
}

// WinDivert 2.x WINDIVERT_ADDRESS는 정확히 80바이트여야 함
const _: () = assert!(std::mem::size_of::<WindivertAddress>() == 80);

impl Default for WindivertAddress {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

impl WindivertAddress {
    #[inline]
    pub fn is_outbound(&self) -> bool {
        (self.flags & 0x02) != 0
    }
}

extern "system" {
    fn WinDivertOpen(
        filter: *const c_char,
        layer: u32,
        priority: i16,
        flags: u64,
    ) -> *mut c_void;

    fn WinDivertRecv(
        handle: *mut c_void,
        p_packet: *mut c_void,
        packet_len: u32,
        p_recv_len: *mut u32,
        p_addr: *mut WindivertAddress,
    ) -> i32;

    fn WinDivertSend(
        handle: *mut c_void,
        p_packet: *const c_void,
        packet_len: u32,
        p_send_len: *mut u32,
        p_addr: *const WindivertAddress,
    ) -> i32;

    fn WinDivertShutdown(handle: *mut c_void, how: u32) -> i32;
    fn WinDivertClose(handle: *mut c_void) -> i32;

    fn WinDivertHelperCompileFilter(
        filter: *const c_char,
        layer: u32,
        object: *mut c_char,
        obj_len: u32,
        error_str: *mut *const c_char,
        error_pos: *mut u32,
    ) -> i32;
}

pub fn validate_filter(filter: &str) -> Result<(), String> {
    let trimmed = filter.trim();
    if trimmed.is_empty() {
        return Err("필터 식을 입력해 주세요. (예: outbound)".to_string());
    }
    let c_filter =
        CString::new(trimmed).map_err(|_| "필터 식에 잘못된 문자가 포함되었습니다.".to_string())?;

    let mut err_str: *const c_char = ptr::null();
    let mut err_pos: u32 = 0;
    let ok = unsafe {
        WinDivertHelperCompileFilter(
            c_filter.as_ptr(),
            WINDIVERT_LAYER_NETWORK,
            ptr::null_mut(),
            0,
            &mut err_str,
            &mut err_pos,
        )
    };

    if ok != 0 {
        Ok(())
    } else {
        let msg = if !err_str.is_null() {
            unsafe { CStr::from_ptr(err_str) }
                .to_string_lossy()
                .into_owned()
        } else {
            "알 수 없는 필터 문법 오류".to_string()
        };
        Err(format!("필터 문법 오류 (위치 {}): {}", err_pos, msg))
    }
}

pub struct DivertHandle {
    raw: AtomicPtr<c_void>,
}

impl DivertHandle {
    pub fn open(filter: &str) -> Result<Self, String> {
        validate_filter(filter)?;
        let c_filter = CString::new(filter.trim()).unwrap();

        let handle = unsafe { WinDivertOpen(c_filter.as_ptr(), WINDIVERT_LAYER_NETWORK, 0, 0) };
        let invalid = (-1isize) as *mut c_void;
        if handle.is_null() || handle == invalid {
            let code = io::Error::last_os_error().raw_os_error().unwrap_or(-1);
            let reason = match code {
                2 => "WinDivert64.sys 드라이버 파일을 찾을 수 없습니다. (exe와 같은 폴더 확인)",
                5 => "관리자 권한이 필요합니다. VS Code 또는 프로그램을 관리자 권한으로 실행하세요.",
                87 => "잘못된 필터 표현식 또는 파라미터입니다.",
                577 => "WinDivert64.sys 드라이버 서명 검증에 실패했습니다.",
                1275 => "보안 프로그램(백신/안티치트)이 WinDivert 드라이버 로드를 차단했습니다.",
                _ => "WinDivert 장치를 여는 데 실패했습니다.",
            };
            return Err(format!("{} (OS Error: {})", reason, code));
        }

        Ok(Self {
            raw: AtomicPtr::new(handle),
        })
    }

    pub fn recv(&self, buf: &mut [u8], addr: &mut WindivertAddress) -> Result<usize, i32> {
        let h = self.raw.load(Ordering::Acquire);
        if h.is_null() {
            return Err(232);
        }
        let mut recv_len: u32 = 0;
        let ok = unsafe {
            WinDivertRecv(
                h,
                buf.as_mut_ptr() as *mut c_void,
                buf.len() as u32,
                &mut recv_len,
                addr as *mut WindivertAddress,
            )
        };
        if ok != 0 {
            Ok(recv_len as usize)
        } else {
            Err(io::Error::last_os_error().raw_os_error().unwrap_or(-1))
        }
    }

    pub fn send(&self, data: &[u8], addr: &WindivertAddress) -> Result<usize, i32> {
        let h = self.raw.load(Ordering::Acquire);
        if h.is_null() {
            return Err(6);
        }
        let mut send_len: u32 = 0;
        let ok = unsafe {
            WinDivertSend(
                h,
                data.as_ptr() as *const c_void,
                data.len() as u32,
                &mut send_len,
                addr as *const WindivertAddress,
            )
        };
        if ok != 0 {
            Ok(send_len as usize)
        } else {
            Err(io::Error::last_os_error().raw_os_error().unwrap_or(-1))
        }
    }

    pub fn shutdown_recv(&self) {
        let h = self.raw.load(Ordering::Acquire);
        if !h.is_null() {
            unsafe {
                WinDivertShutdown(h, WINDIVERT_SHUTDOWN_RECV);
            }
        }
    }
}

impl Drop for DivertHandle {
    fn drop(&mut self) {
        let h = self.raw.swap(ptr::null_mut(), Ordering::AcqRel);
        if !h.is_null() {
            unsafe {
                WinDivertShutdown(h, WINDIVERT_SHUTDOWN_BOTH);
                WinDivertClose(h);
            }
        }
    }
}