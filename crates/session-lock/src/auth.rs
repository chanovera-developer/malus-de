//! Autenticación real del usuario mediante Linux-PAM (`libpam.so.0`) cargada dinámicamente.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::OnceLock;

const PAM_SUCCESS: c_int = 0;
const PAM_AUTH_ERR: c_int = 7;
const PAM_CRED_INSUFFICIENT: c_int = 8;
const PAM_AUTHINFO_UNAVAIL: c_int = 9;
const PAM_USER_UNKNOWN: c_int = 10;
const PAM_MAXTRIES: c_int = 11;
const PAM_CONV_ERR: c_int = 19;
const PAM_SILENT: c_int = 0x8000;

const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

type PamConvFn = unsafe extern "C" fn(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata_ptr: *mut c_void,
) -> c_int;

#[repr(C)]
struct PamConv {
    conv: Option<PamConvFn>,
    appdata_ptr: *mut c_void,
}

type PamStartFn = unsafe extern "C" fn(
    service_name: *const c_char,
    user: *const c_char,
    pam_conversation: *const PamConv,
    pamh: *mut *mut c_void,
) -> c_int;
type PamAuthenticateFn = unsafe extern "C" fn(pamh: *mut c_void, flags: c_int) -> c_int;
type PamAcctMgmtFn = unsafe extern "C" fn(pamh: *mut c_void, flags: c_int) -> c_int;
type PamEndFn = unsafe extern "C" fn(pamh: *mut c_void, pam_status: c_int) -> c_int;

struct PamApi {
    _handle: usize,
    pam_start: PamStartFn,
    pam_authenticate: PamAuthenticateFn,
    pam_acct_mgmt: PamAcctMgmtFn,
    pam_end: PamEndFn,
}

static PAM_API: OnceLock<Result<PamApi, String>> = OnceLock::new();

fn load_pam_api() -> Result<&'static PamApi, String> {
    let res = PAM_API.get_or_init(|| unsafe {
        let mut handle = std::ptr::null_mut();
        for soname in [b"libpam.so.0\0".as_slice(), b"libpam.so\0".as_slice()] {
            handle = libc::dlopen(soname.as_ptr().cast(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if !handle.is_null() {
                break;
            }
        }
        if handle.is_null() {
            return Err("No se pudo cargar libpam.so.0".to_string());
        }

        macro_rules! load_sym {
            ($name:literal, $ty:ty) => {{
                let ptr = libc::dlsym(handle, concat!($name, "\0").as_ptr().cast());
                if ptr.is_null() {
                    return Err(format!("Símbolo PAM faltante: {}", $name));
                }
                std::mem::transmute::<*mut c_void, $ty>(ptr)
            }};
        }

        Ok(PamApi {
            _handle: handle as usize,
            pam_start: load_sym!("pam_start", PamStartFn),
            pam_authenticate: load_sym!("pam_authenticate", PamAuthenticateFn),
            pam_acct_mgmt: load_sym!("pam_acct_mgmt", PamAcctMgmtFn),
            pam_end: load_sym!("pam_end", PamEndFn),
        })
    });
    res.as_ref().map_err(Clone::clone)
}

/// Sobrescribe un búfer en memoria con ceros usando escrituras volátiles para evitar que el compilador las elimine.
fn zeroize_bytes(bytes: &mut [u8]) {
    for b in bytes.iter_mut() {
        unsafe {
            std::ptr::write_volatile(b, 0);
        }
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

unsafe fn free_partial_pam_replies(replies: *mut PamResponse, count: isize) {
    for j in 0..count {
        let resp_ptr = (*replies.offset(j)).resp;
        if !resp_ptr.is_null() {
            let len = libc::strlen(resp_ptr);
            if len > 0 {
                let slice = std::slice::from_raw_parts_mut(resp_ptr.cast::<u8>(), len);
                zeroize_bytes(slice);
            }
            libc::free(resp_ptr.cast());
        }
    }
    libc::free(replies.cast());
}

struct ConvPayload {
    username: CString,
    /// Contraseña terminada en byte nulo, borrada explícitamente con ceros en `Drop`.
    password_nul: Vec<u8>,
}

impl Drop for ConvPayload {
    fn drop(&mut self) {
        zeroize_bytes(&mut self.password_nul);
    }
}

unsafe extern "C" fn pam_conv_cb(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata_ptr: *mut c_void,
) -> c_int {
    if num_msg <= 0 || msg.is_null() || resp.is_null() || appdata_ptr.is_null() {
        return PAM_CONV_ERR;
    }
    let payload = &*(appdata_ptr as *const ConvPayload);
    let replies =
        libc::calloc(num_msg as usize, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
    if replies.is_null() {
        return PAM_CONV_ERR;
    }

    for i in 0..(num_msg as isize) {
        let m = *msg.offset(i);
        if m.is_null() {
            free_partial_pam_replies(replies, i);
            return PAM_CONV_ERR;
        }
        let slot = &mut *replies.offset(i);
        slot.resp_retcode = 0;
        match (*m).msg_style {
            PAM_PROMPT_ECHO_OFF => {
                let dup = libc::strdup(payload.password_nul.as_ptr().cast());
                if dup.is_null() {
                    free_partial_pam_replies(replies, i);
                    return PAM_CONV_ERR;
                }
                slot.resp = dup;
            }
            PAM_PROMPT_ECHO_ON => {
                let dup = libc::strdup(payload.username.as_ptr());
                if dup.is_null() {
                    free_partial_pam_replies(replies, i);
                    return PAM_CONV_ERR;
                }
                slot.resp = dup;
            }
            PAM_ERROR_MSG | PAM_TEXT_INFO => {
                slot.resp = std::ptr::null_mut();
            }
            _ => {
                slot.resp = std::ptr::null_mut();
            }
        }
    }

    *resp = replies;
    PAM_SUCCESS
}

/// Obtiene el nombre del usuario real del UID actual mediante NSS (`getpwuid_r`, seguro frente a hilos),
/// evitando depender de la variable de entorno `USER`.
pub fn current_username() -> String {
    unsafe {
        let uid = libc::getuid();
        let mut pwd: libc::passwd = std::mem::zeroed();
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let mut buf = vec![0u8; 4096];
        let rc = libc::getpwuid_r(
            uid,
            &mut pwd,
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut result,
        );
        if rc == 0 && !result.is_null() && !pwd.pw_name.is_null() {
            if let Ok(name) = CStr::from_ptr(pwd.pw_name).to_str() {
                if !name.is_empty() {
                    return name.to_string();
                }
            }
        }
    }
    std::env::var("USER").unwrap_or_else(|_| "Usuario".to_string())
}

/// Verifica la contraseña del usuario usando PAM (`system-auth` / `login`).
/// Devuelve `Ok(true)` si es válida, `Ok(false)` si las credenciales son incorrectas.
pub fn verify_user_password(username: &str, password: &str) -> Result<bool, String> {
    let api = load_pam_api()?;
    let c_user = CString::new(username).map_err(|_| "Nombre de usuario inválido".to_string())?;
    if password.as_bytes().contains(&0) {
        return Err("La contraseña contiene bytes nulos".to_string());
    }
    let mut password_nul = Vec::with_capacity(password.len() + 1);
    password_nul.extend_from_slice(password.as_bytes());
    password_nul.push(0);

    let payload = ConvPayload {
        username: c_user.clone(),
        password_nul,
    };
    let conv = PamConv {
        conv: Some(pam_conv_cb),
        appdata_ptr: (&payload as *const ConvPayload).cast_mut().cast(),
    };

    let mut last_err = None;
    for service in ["system-auth", "login", "passwd"] {
        let Ok(c_service) = CString::new(service) else {
            continue;
        };
        let mut pamh: *mut c_void = std::ptr::null_mut();
        let rc_start =
            unsafe { (api.pam_start)(c_service.as_ptr(), c_user.as_ptr(), &conv, &mut pamh) };
        if rc_start != PAM_SUCCESS || pamh.is_null() {
            if !pamh.is_null() {
                unsafe { (api.pam_end)(pamh, rc_start) };
            }
            last_err = Some(format!("pam_start({service}) devolvió código {rc_start}"));
            continue;
        }

        let rc_auth = unsafe { (api.pam_authenticate)(pamh, PAM_SILENT) };
        let result = if rc_auth == PAM_SUCCESS {
            let rc_acct = unsafe { (api.pam_acct_mgmt)(pamh, PAM_SILENT) };
            unsafe { (api.pam_end)(pamh, rc_acct) };
            match rc_acct {
                PAM_SUCCESS => Ok(true),
                PAM_AUTH_ERR
                | PAM_CRED_INSUFFICIENT
                | PAM_AUTHINFO_UNAVAIL
                | PAM_USER_UNKNOWN
                | PAM_MAXTRIES => Ok(false),
                code => Err(format!("pam_acct_mgmt devolvió código {code}")),
            }
        } else {
            unsafe { (api.pam_end)(pamh, rc_auth) };
            match rc_auth {
                PAM_AUTH_ERR
                | PAM_CRED_INSUFFICIENT
                | PAM_AUTHINFO_UNAVAIL
                | PAM_USER_UNKNOWN
                | PAM_MAXTRIES => Ok(false),
                code => Err(format!(
                    "pam_authenticate({service}) devolvió código {code}"
                )),
            }
        };

        // Borrado en memoria del buffer temporal de contraseña al terminar
        drop(payload);
        return result;
    }

    Err(last_err.unwrap_or_else(|| "No se pudo iniciar ningún servicio PAM".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_username_returns_non_empty() {
        assert!(!current_username().is_empty());
    }

    #[test]
    fn libpam_loads_cleanly() {
        assert!(load_pam_api().is_ok());
    }

    #[test]
    fn zeroize_bytes_overwrites_buffer_with_zeros() {
        let mut secret = b"super-secret-password".to_vec();
        zeroize_bytes(&mut secret);
        assert!(secret.iter().all(|&b| b == 0));
    }
}
