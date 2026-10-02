#![forbid(unsafe_op_in_unsafe_fn)]
#![allow(non_snake_case)]
use std::ffi::c_void;

/// Stable ABI version for dynamically loaded backend plugins.
///
/// ABI V2 introduces an explicit per-call cooperative cancellation signal. Every potentially
/// cancellable plugin operation receives a [`PluginCancellationV2`] pointer and must observe it at
/// appropriate safe interruption points. The host never forcefully terminates plugin code.
pub const PLUGIN_ABI_VERSION_V2: u32 = 2;

/// Exported constructor symbol for ABI V2.
pub const PLUGIN_CONSTRUCTOR_SYMBOL_V2: &[u8] = b"InstantiateV2\0";
/// Maximum byte payload accepted by any ABI buffer/slice conversion.
pub const MAX_ABI_BUFFER_BYTES: usize = 64 * 1024 * 1024;

/// ABI operation status codes. Zero means success; negative values are framework-defined errors.
pub const PLUGIN_STATUS_OK: i32 = 0;
pub const PLUGIN_STATUS_INVALID_ARGUMENT: i32 = -1;
pub const PLUGIN_STATUS_PANIC: i32 = -2;
pub const PLUGIN_STATUS_NOT_INITIALIZED: i32 = -3;
pub const PLUGIN_STATUS_SERIALIZATION: i32 = -4;
pub const PLUGIN_STATUS_PLUGIN_ERROR: i32 = -5;
pub const PLUGIN_STATUS_CANCELLED: i32 = -6;

/// A borrowed byte slice crossing the ABI boundary.
///
/// `ptr` is never owned by the callee and is valid only for the duration documented by the
/// calling function. The layout is equivalent to a C `pointer + size_t` pair.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AbiSlice {
    pub ptr: *const u8,
    pub len: usize,
}

impl AbiSlice {
    pub const EMPTY: Self = Self { ptr: std::ptr::null(), len: 0 };

    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self { ptr: bytes.as_ptr(), len: bytes.len() }
    }

    /// Copies the borrowed bytes into owned host/plugin memory.
    ///
    /// # Safety
    /// The pointer must be null when `len == 0`, or point to `len` readable bytes for the
    /// duration of this call.
    pub unsafe fn to_vec(self) -> Option<Vec<u8>> {
        if self.len == 0 {
            return Some(Vec::new());
        }
        if self.len > MAX_ABI_BUFFER_BYTES || self.ptr.is_null() {
            return None;
        }
        let bytes = unsafe { std::slice::from_raw_parts(self.ptr, self.len) };
        Some(bytes.to_vec())
    }
}

/// An owned byte buffer crossing the ABI boundary.
///
/// The allocator remains on the side that produced the buffer. The consumer must call the
/// producer's explicit `free_buffer` function instead of attempting to deallocate the memory
/// directly. `capacity` is retained so the producing side can reconstruct its original `Vec<u8>`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AbiBuffer {
    pub ptr: *mut u8,
    pub len: usize,
    pub capacity: usize,
}

impl AbiBuffer {
    pub const EMPTY: Self = Self {
        ptr: std::ptr::null_mut(),
        len: 0,
        capacity: 0,
    };

    pub fn from_vec(mut bytes: Vec<u8>) -> Self {
        if bytes.is_empty() {
            return Self::EMPTY;
        }
        let buffer = Self {
            ptr: bytes.as_mut_ptr(),
            len: bytes.len(),
            capacity: bytes.capacity(),
        };
        std::mem::forget(bytes);
        buffer
    }

    /// Copies the buffer into consumer-owned memory.
    ///
    /// # Safety
    /// The buffer must refer to `len` readable bytes. Ownership is not consumed by this function.
    pub unsafe fn to_vec(self) -> Option<Vec<u8>> {
        if self.len == 0 {
            return Some(Vec::new());
        }
        if self.len > MAX_ABI_BUFFER_BYTES || self.len > self.capacity || self.ptr.is_null() {
            return None;
        }
        let bytes = unsafe { std::slice::from_raw_parts(self.ptr, self.len) };
        Some(bytes.to_vec())
    }

    /// Reclaims a buffer using the allocator that created it.
    ///
    /// # Safety
    /// The buffer must have been created by `AbiBuffer::from_vec` on the same ABI side and must
    /// not have been freed before.
    pub unsafe fn free(self) {
        if self.ptr.is_null() {
            return;
        }
        if self.len == 0 && self.capacity == 0 {
            return;
        }
        if self.len > self.capacity || self.capacity > MAX_ABI_BUFFER_BYTES {
            return;
        }
        let _ = unsafe { Vec::from_raw_parts(self.ptr, 0, self.capacity) };
    }
}

/// Called by plugin code to query whether the host has requested cooperative cancellation.
pub type PluginIsCancelledFnV2 = unsafe extern "C" fn(context: *const c_void) -> bool;

/// Per-operation cooperative cancellation state.
///
/// The object is borrowed for the duration of exactly one ABI call. Plugins may retain their own
/// application-level cancellation state, but must not retain this ABI pointer after the callback
/// returns.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginCancellationV2 {
    pub context: *const c_void,
    pub is_cancelled: PluginIsCancelledFnV2,
}

// The cancellation token is an immutable ABI descriptor. Its context pointer is only valid
// while the associated plugin operation is active; the host keeps that state alive until the
// worker/lifecycle callback has actually returned.
unsafe impl Send for PluginCancellationV2 {}
unsafe impl Sync for PluginCancellationV2 {}

impl PluginCancellationV2 {
    /// # Safety
    /// `context` and the callback are supplied by the ABI caller and remain valid for this call.
    pub unsafe fn is_cancelled(self) -> bool {
        unsafe { (self.is_cancelled)(self.context) }
    }
}

pub type PluginLogFnV2 = unsafe extern "C" fn(level: u8, message: AbiSlice);

pub type PluginDatabaseExecuteFnV2 = unsafe extern "C" fn(
    request: AbiSlice,
    cancellation: PluginCancellationV2,
    response_out: *mut AbiBuffer
) -> i32;

pub type PluginFreeBufferFnV2 = unsafe extern "C" fn(buffer: AbiBuffer);
pub type PluginNameFnV2 = unsafe extern "C" fn(
    context: *mut c_void,
    cancellation: PluginCancellationV2,
    response_out: *mut AbiBuffer
) -> i32;
pub type PluginManifestFnV2 = unsafe extern "C" fn(
    context: *mut c_void,
    cancellation: PluginCancellationV2,
    response_out: *mut AbiBuffer
) -> i32;
pub type PluginLoadFnV2 = unsafe extern "C" fn(
    context: *mut c_void,
    cancellation: PluginCancellationV2
) -> i32;
pub type PluginLoadedFnV2 = unsafe extern "C" fn(
    context: *mut c_void,
    cancellation: PluginCancellationV2
) -> i32;
pub type PluginEndpointFnV2 = unsafe extern "C" fn(
    context: *mut c_void,
    request: AbiSlice,
    cancellation: PluginCancellationV2,
    response_out: *mut AbiBuffer
) -> i32;
pub type PluginDatabaseFnV2 = unsafe extern "C" fn(
    context: *mut c_void,
    request: AbiSlice,
    cancellation: PluginCancellationV2,
    response_out: *mut AbiBuffer
) -> i32;
pub type PluginSessionClosedFnV2 = unsafe extern "C" fn(
    context: *mut c_void,
    request: AbiSlice,
    cancellation: PluginCancellationV2,
    response_out: *mut AbiBuffer
) -> i32;
pub type PluginDestroyFnV2 = unsafe extern "C" fn(
    instance: *mut PluginInstanceV2,
    cancellation: PluginCancellationV2
) -> i32;

/// Host capabilities supplied to a plugin during construction.
///
/// The constructor receives a pointer rather than this struct by value so future fields may be
/// appended without changing the calling convention. `struct_size` allows a consumer to reject
/// truncated payloads and accept compatible newer payloads.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginHostServicesV2 {
    pub abi_version: u32,
    pub struct_size: u32,
    pub logger: PluginLoggerV2,
    pub database: Option<PluginDatabaseExecuteFnV2>,
    pub database_free: Option<PluginFreeBufferFnV2>,
    pub plugin_directory: AbiSlice,
}

impl PluginHostServicesV2 {
    pub const fn new(
        logger: PluginLogFnV2,
        database: Option<PluginDatabaseExecuteFnV2>,
        database_free: Option<PluginFreeBufferFnV2>,
        plugin_directory: AbiSlice
    ) -> Self {
        Self {
            abi_version: PLUGIN_ABI_VERSION_V2,
            struct_size: std::mem::size_of::<Self>() as u32,
            logger: PluginLoggerV2 { emit: logger },
            database,
            database_free,
            plugin_directory,
        }
    }

    pub const fn is_compatible(&self) -> bool {
        self.abi_version == PLUGIN_ABI_VERSION_V2 &&
            self.struct_size >= (std::mem::size_of::<Self>() as u32)
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginLoggerV2 {
    pub emit: PluginLogFnV2,
}

/// Function table implemented by a plugin library.
///
/// No Rust references, `Box`, `Vec`, `String`, trait objects, or Rust enums cross this boundary.
/// All payloads use primitive C-compatible fields or explicitly borrowed/owned byte buffers.
#[repr(C)]
pub struct PluginVTableV2 {
    pub abi_version: u32,
    pub struct_size: u32,
    pub name: PluginNameFnV2,
    pub manifest: PluginManifestFnV2,
    pub on_load: PluginLoadFnV2,
    pub is_loaded: PluginLoadedFnV2,
    pub handle_endpoint: PluginEndpointFnV2,
    pub execute_database: PluginDatabaseFnV2,
    pub on_session_closed: PluginSessionClosedFnV2,
    pub on_unload: PluginLoadFnV2,
    pub free_buffer: PluginFreeBufferFnV2,
    pub destroy: PluginDestroyFnV2,
}

impl PluginVTableV2 {
    pub const fn is_compatible(&self) -> bool {
        self.abi_version == PLUGIN_ABI_VERSION_V2 &&
            self.struct_size >= (std::mem::size_of::<Self>() as u32)
    }
}

/// Opaque plugin instance returned from the dynamic library.
#[repr(C)]
pub struct PluginInstanceV2 {
    pub abi_version: u32,
    pub struct_size: u32,
    pub context: *mut c_void,
    pub vtable: *const PluginVTableV2,
}

impl PluginInstanceV2 {
    pub const fn abi_version() -> u32 {
        PLUGIN_ABI_VERSION_V2
    }
}

/// Constructor function exported by a plugin library for ABI V2.
pub type PluginCreateFnV2 = unsafe extern "C" fn(
    host_services: *const PluginHostServicesV2
) -> *mut PluginInstanceV2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_roundtrip_is_allocator_neutral() {
        let buffer = AbiBuffer::from_vec(b"hello".to_vec());
        let copied = (unsafe { buffer.to_vec() }).expect("buffer must be readable");
        assert_eq!(copied, b"hello");
        unsafe { buffer.free() }
    }

    #[test]
    fn host_services_are_pointer_sized_and_versioned() {
        let size = std::mem::size_of::<PluginHostServicesV2>();
        let host = PluginHostServicesV2::new(dummy_log, None, None, AbiSlice::EMPTY);
        assert_eq!(host.abi_version, PLUGIN_ABI_VERSION_V2);
        assert_eq!(host.struct_size as usize, size);
        assert!(host.is_compatible());
    }

    #[test]
    fn cancellation_callback_is_cooperative() {
        let value = true;
        let cancellation = PluginCancellationV2 {
            context: &value as *const bool as *const c_void,
            is_cancelled: test_is_cancelled,
        };
        assert!(unsafe { cancellation.is_cancelled() });
    }

    unsafe extern "C" fn dummy_log(_level: u8, _message: AbiSlice) {}

    unsafe extern "C" fn test_is_cancelled(context: *const c_void) -> bool {
        unsafe { *(context as *const bool) }
    }
}
