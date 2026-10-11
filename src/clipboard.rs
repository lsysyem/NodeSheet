//! 系统剪贴板：放文本（复制选中的单元格）、取文本（create 节点的“粘贴剪贴板”），直接调 Win32，不引入依赖。

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn OpenClipboard(owner: *mut c_void) -> i32;
        pub fn CloseClipboard() -> i32;
        pub fn EmptyClipboard() -> i32;
        pub fn SetClipboardData(format: u32, mem: *mut c_void) -> *mut c_void;
        pub fn GetClipboardData(format: u32) -> *mut c_void;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
        pub fn GlobalLock(mem: *mut c_void) -> *mut c_void;
        pub fn GlobalUnlock(mem: *mut c_void) -> i32;
        pub fn GlobalFree(mem: *mut c_void) -> *mut c_void;
    }
    pub const CF_UNICODETEXT: u32 = 13;
    pub const GMEM_MOVEABLE: u32 = 2;

    /// 别的程序可能正占着剪贴板：等一等再试。
    pub fn open() -> Result<(), String> {
        for _ in 0..20 {
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Err(String::from("剪贴板正被别的程序占用"))
    }
}

/// 把文本放进剪贴板（换行用 \r\n，Excel / 记事本都认）。
#[cfg(windows)]
pub fn set_text(text: &str) -> Result<(), String> {
    use win::*;
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    open()?;
    // 数据交给剪贴板后归系统所有；没交出去才自己释放
    let placed = unsafe {
        EmptyClipboard();
        let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        let target = if mem.is_null() { std::ptr::null_mut() } else { GlobalLock(mem) as *mut u16 };
        let placed = if target.is_null() {
            false
        } else {
            std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
            GlobalUnlock(mem);
            !SetClipboardData(CF_UNICODETEXT, mem).is_null()
        };
        if !placed && !mem.is_null() {
            GlobalFree(mem);
        }
        CloseClipboard();
        placed
    };
    if placed { Ok(()) } else { Err(String::from("写入剪贴板失败（内容太大？）")) }
}

#[cfg(not(windows))]
pub fn set_text(_text: &str) -> Result<(), String> {
    Err(String::from("这个系统上还不支持复制到剪贴板"))
}

/// 剪贴板里的文本（换行原样，Excel 复制的是 CRLF）。剪贴板里不是文字时报错。
#[cfg(windows)]
pub fn get_text() -> Result<String, String> {
    use win::*;
    open()?;
    let text = unsafe {
        let mem = GetClipboardData(CF_UNICODETEXT);
        let ptr = if mem.is_null() { std::ptr::null() } else { GlobalLock(mem) as *const u16 };
        let text = (!ptr.is_null()).then(|| {
            let len = (0..).take_while(|&i| *ptr.add(i) != 0).count();
            String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
        });
        if !ptr.is_null() {
            GlobalUnlock(mem);
        }
        CloseClipboard();
        text
    };
    text.ok_or_else(|| String::from("剪贴板里没有文字"))
}

#[cfg(not(windows))]
pub fn get_text() -> Result<String, String> {
    Err(String::from("这个系统上还不支持读剪贴板"))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// 会动到系统剪贴板（结束时把原来的文本放回去），所以默认不跑：
    /// cargo test clipboard -- --ignored
    #[test]
    #[ignore]
    fn text_round_trips() {
        let before = get_text().ok();
        let text = "产品\t数量\r\n\"带\t制表符\"\t3\r\n";
        set_text(text).unwrap();
        assert_eq!(get_text().as_deref(), Ok(text));
        if let Some(before) = before {
            set_text(&before).unwrap();
        }
    }
}
