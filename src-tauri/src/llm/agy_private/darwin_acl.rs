//! Descriptor-based ACL admission, reused from the previous transport.
use super::*;
use std::ffi::c_void;
use std::os::unix::io::AsRawFd;
const FILESEC_ACL: i32 = 5;
extern "C" {
    fn acl_free(acl: *mut c_void) -> i32;
    fn acl_valid(acl: *mut c_void) -> i32;
    fn acl_get_entry(acl: *mut c_void, entry: i32, result: *mut *mut c_void) -> i32;
    fn filesec_init() -> *mut c_void;
    fn filesec_free(security: *mut c_void);
    fn filesec_query_property(security: *mut c_void, property: i32, present: *mut i32) -> i32;
    fn filesec_get_property(security: *mut c_void, property: i32, result: *mut c_void) -> i32;
    #[cfg_attr(target_arch = "x86_64", link_name = "fstatx_np$INODE64")]
    fn fstatx_np(fd: i32, stat: *mut libc::stat, security: *mut c_void) -> i32;
}
struct Acl(*mut c_void);
impl Drop for Acl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                acl_free(self.0);
            }
        }
    }
}
struct Security(*mut c_void);
impl Drop for Security {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                filesec_free(self.0);
            }
        }
    }
}
pub(super) fn require_empty(file: &File) -> io::Result<()> {
    // Query successful descriptor metadata explicitly. A NULL ACL or
    // ENOENT alone is not accepted as evidence of absence.
    // SAFETY: live fd; initialized output storage and owned opaque
    // allocations. An ACL from filesec_get_property is separately owned.
    unsafe {
        let security = Security(filesec_init());
        if security.0.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        if fstatx_np(file.as_raw_fd(), metadata.as_mut_ptr(), security.0) != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut present = 0;
        if filesec_query_property(security.0, FILESEC_ACL, &mut present) != 0 {
            return Err(io::Error::last_os_error());
        }
        if present == 0 {
            return Ok(());
        }
        let mut raw: *mut c_void = std::ptr::null_mut();
        if filesec_get_property(security.0, FILESEC_ACL, &mut raw as *mut _ as *mut c_void) != 0 {
            return Err(io::Error::last_os_error());
        }
        if raw.is_null() || raw as usize == 1 {
            return Err(denied());
        }
        let acl = Acl(raw);
        if acl_valid(acl.0) != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut entry = std::ptr::null_mut();
        let result = acl_get_entry(acl.0, 0, &mut entry);
        let error = io::Error::last_os_error();
        if result == -1 && error.raw_os_error() == Some(libc::EINVAL) {
            // Darwin's valid empty ACL has no ACL_FIRST_ENTRY.
            Ok(())
        } else if result == 0 {
            Err(denied())
        } else {
            Err(error)
        }
    }
}
