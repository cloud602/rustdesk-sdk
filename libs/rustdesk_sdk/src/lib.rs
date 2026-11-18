use libc::c_char;
use librustdesk as rustdesk;
use std::ffi::CString;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

static SERVER_STARTED: AtomicBool = AtomicBool::new(false);

fn init_app_name_from_env() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let Ok(app_name) = std::env::var("RUSTDESK_APPNAME") else {
            return;
        };
        let app_name = app_name.trim();
        if !app_name.is_empty() {
            *hbb_common::config::APP_NAME.write().unwrap() = app_name.to_owned();
        }
    });
}

fn to_c_string(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(cs) => cs.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

fn with_unwind_default<T: Default, F: FnOnce() -> T>(f: F) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_default()
}

fn with_unwind_null<T, F: FnOnce() -> *mut T>(f: F) -> *mut T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(ptr::null_mut())
}

fn c_str_to_string(s: *const c_char) -> String {
    if s.is_null() {
        return String::new();
    }
    unsafe { std::ffi::CStr::from_ptr(s).to_string_lossy().into_owned() }
}

fn run_async<T, F>(f: F) -> Option<T>
where
    F: std::future::Future<Output = T>,
{
    let rt = hbb_common::tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    Some(rt.block_on(f))
}

#[no_mangle]
pub extern "C" fn rd_global_init() -> bool {
    with_unwind_default(|| {
        init_app_name_from_env();
        rustdesk::common::global_init()
    })
}

#[no_mangle]
pub extern "C" fn rd_is_cm() -> bool {
    with_unwind_default(|| rustdesk::common::is_cm())
}

#[no_mangle]
pub extern "C" fn rd_start_server(is_server: bool, no_server: bool) -> bool {
    with_unwind_default(|| {
        std::thread::spawn(move || {
            rustdesk::start_server(is_server, no_server);
        });
        true
    })
}

// Safe start using the same pattern as the example:
// global_init -> if not is_cm then start_server(false, false)
#[no_mangle]
pub extern "C" fn rd_start_server_safe() -> bool {
    with_unwind_default(|| {
        init_app_name_from_env();
        if !rustdesk::common::global_init() {
            return false;
        }
        if rustdesk::common::is_cm() {
            return true;
        }
        if SERVER_STARTED.load(Ordering::SeqCst) {
            return true;
        }
        hbb_common::config::Config::set_option("stop-service".into(), "".into());
        std::thread::spawn(move || {
            rustdesk::start_server(false, false);
        });
        SERVER_STARTED.store(true, Ordering::SeqCst);
        true
    })
}

// Start the connection manager IPC listener inside this process (headless).
// The controlled side hands file transfer / file clipboard to the CM over `_cm`;
// embedded hosts have no UI process, so the listener has to live here.
#[no_mangle]
pub extern "C" fn rd_start_cm() -> bool {
    with_unwind_default(|| {
        init_app_name_from_env();
        if !rustdesk::common::global_init() {
            return false;
        }
        rustdesk::start_headless_cm()
    })
}

#[no_mangle]
pub extern "C" fn rd_stop_server() -> bool {
    with_unwind_default(|| {
        if !SERVER_STARTED.load(Ordering::SeqCst) {
            return true;
        }
        hbb_common::config::Config::set_option("stop-service".into(), "Y".into());
        rustdesk::RendezvousMediator::restart();
        SERVER_STARTED.store(false, Ordering::SeqCst);
        true
    })
}

#[no_mangle]
pub extern "C" fn rd_is_server_running() -> bool {
    SERVER_STARTED.load(Ordering::SeqCst)
}

#[no_mangle]
pub extern "C" fn rd_update_temporary_password() -> bool {
    with_unwind_default(|| {
        hbb_common::password_security::update_temporary_password();
        true
    })
}

#[no_mangle]
pub extern "C" fn rd_get_permanent_password() -> *mut c_char {
    with_unwind_null(|| to_c_string(hbb_common::config::Config::get_permanent_password()))
}

#[no_mangle]
pub extern "C" fn rd_set_permanent_password(password: *const c_char) -> bool {
    with_unwind_default(|| {
        let v = c_str_to_string(password);
        hbb_common::config::Config::set_permanent_password(&v);
        true
    })
}

#[no_mangle]
pub extern "C" fn rd_get_option(key: *const c_char) -> *mut c_char {
    with_unwind_null(|| {
        let k = c_str_to_string(key);
        to_c_string(hbb_common::config::Config::get_option(&k))
    })
}

#[no_mangle]
pub extern "C" fn rd_set_option(key: *const c_char, value: *const c_char) -> bool {
    with_unwind_default(|| {
        init_app_name_from_env();
        let k = c_str_to_string(key);
        let v = c_str_to_string(value);
        hbb_common::config::Config::set_option(k, v);
        true
    })
}

#[no_mangle]
pub extern "C" fn rd_get_all_options() -> *mut c_char {
    with_unwind_null(|| {
        let opts = hbb_common::config::Config::get_options();
        let json = serde_json::to_string(&opts).unwrap_or_else(|_| "{}".to_string());
        to_c_string(json)
    })
}

#[no_mangle]
pub extern "C" fn rd_get_rendezvous_server(ms_timeout: u64) -> *mut c_char {
    with_unwind_null(|| {
        let res = run_async(rustdesk::common::get_rendezvous_server(ms_timeout))
            .unwrap_or_else(|| ("".to_string(), Vec::new(), false));
        let json = serde_json::json!({
            "server": res.0,
            "servers": res.1,
            "is_public": res.2
        });
        to_c_string(json.to_string())
    })
}

#[no_mangle]
pub extern "C" fn rd_get_nat_type(ms_timeout: u64) -> i32 {
    with_unwind_default(|| run_async(rustdesk::common::get_nat_type(ms_timeout)).unwrap_or(-1))
}

#[no_mangle]
pub extern "C" fn rd_test_rendezvous_server() -> bool {
    with_unwind_default(|| {
        rustdesk::common::test_rendezvous_server();
        true
    })
}

#[no_mangle]
pub extern "C" fn rd_get_id() -> *mut c_char {
    with_unwind_null(|| {
        init_app_name_from_env();
        to_c_string(hbb_common::config::Config::get_id())
    })
}

#[no_mangle]
pub extern "C" fn rd_get_temporary_password() -> *mut c_char {
    with_unwind_null(|| {
        init_app_name_from_env();
        to_c_string(hbb_common::password_security::temporary_password())
    })
}

#[no_mangle]
pub extern "C" fn rd_free_c_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(s));
    }
}

/// 当前「已认证」的连接数（0 = 没有人在远程本机）。
///
/// 宿主用它判断远程协助会话是否真的结束：RustDesk SDK 没有会话结束回调，
/// 而 `AUTHED_CONNS` 由 RAII 维护（认证成功时 push，连接对象 Drop 时 retain 移除），
/// 是「此刻还有没有人连着」的权威来源。断线后 1~2 秒内即可读到 0。
///
/// 返回 -1 表示查询失败（panic / 锁中毒）：调用方必须当成「未知」，
/// 不能当成「已断开」，否则会在会话进行中把横幅误收起。
#[no_mangle]
pub extern "C" fn rd_authed_conn_count() -> i32 {
    match std::panic::catch_unwind(|| match rustdesk::AUTHED_CONNS.lock() {
        Ok(guard) => guard.len() as i32,
        Err(_) => -1,
    }) {
        Ok(count) => count,
        Err(_) => -1,
    }
}
