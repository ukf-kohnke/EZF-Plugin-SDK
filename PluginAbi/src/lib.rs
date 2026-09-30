#![forbid(unsafe_op_in_unsafe_fn)]
#![allow(non_snake_case)]
use std::ffi::c_void;

/// Stable ABI version for dynamically loaded backend plugins.
///
/// This version is intentionally independent from the logical plugin contract version. Changing
/// the layout or calling convention of the C-compatible types/functions requires a new ABI
/// version and therefore a new exported constructor symbol.
pub const PLUGIN_ABI_VERSION_V1: u32 = 1;

/// Exported constructor symbol for ABI v1.
pub const PLUGIN_CONSTRUCTOR_SYMBOL_V1: &[u8] = b"InstantiateV1\0";
/// Maximum byte payload accepted by any ABI buffer/slice conversion.
pub const MAX_ABI_BUFFER_BYTES: usize = 64 * 1024 * 1024;

/// ABI operation status codes. Zero means success; negative values are framework-defined errors.
pub const PLUGIN_STATUS_OK: i32 = 0;
pub const PLUGIN_STATUS_INVALID_ARGUMENT: i32 = -1;
pub const PLUGIN_STATUS_PANIC: i32 = -2;
pub const PLUGIN_STATUS_NOT_INITIALIZED: i32 = -3;
pub const PLUGIN_STATUS_SERIALIZATION: i32 = -4;
pub const PLUGIN_STATUS_PLUGIN_ERROR: i32 = -5;

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
        let _ = unsafe { Vec::from_raw_parts(self.ptr, 0, self.capacity) };
    }
}

pub type PluginLogFnV1 = unsafe extern "C" fn(level: u8, message: AbiSlice);

pub type PluginDatabaseExecuteFnV1 = unsafe extern "C" fn(
    request: AbiSlice,
    response_out: *mut AbiBuffer
) -> i32;

pub type PluginFreeBufferFnV1 = unsafe extern "C" fn(buffer: AbiBuffer);
pub type PluginNameFnV1 = unsafe extern "C" fn(
    context: *mut c_void,
    response_out: *mut AbiBuffer
) -> i32;
pub type PluginManifestFnV1 = unsafe extern "C" fn(
    context: *mut c_void,
    response_out: *mut AbiBuffer
) -> i32;
pub type PluginLoadFnV1 = unsafe extern "C" fn(context: *mut c_void) -> i32;
pub type PluginLoadedFnV1 = unsafe extern "C" fn(context: *mut c_void) -> i32;
pub type PluginEndpointFnV1 = unsafe extern "C" fn(
    context: *mut c_void,
    request: AbiSlice,
    response_out: *mut AbiBuffer
) -> i32;
pub type PluginDatabaseFnV1 = unsafe extern "C" fn(
    context: *mut c_void,
    request: AbiSlice,
    response_out: *mut AbiBuffer
) -> i32;
pub type PluginSessionClosedFnV1 = unsafe extern "C" fn(
    context: *mut c_void,
    request: AbiSlice,
    response_out: *mut AbiBuffer
) -> i32;
pub type PluginDestroyFnV1 = unsafe extern "C" fn(instance: *mut PluginInstanceV1) -> i32;

/// Host capabilities supplied to a plugin during construction.
///
/// The constructor receives a pointer rather than this struct by value so future fields may be
/// appended without changing the calling convention. `struct_size` allows a consumer to reject
/// truncated payloads and accept compatible newer payloads.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginHostServicesV1 {
    pub abi_version: u32,
    pub struct_size: u32,
    pub logger: PluginLoggerV1,
    pub database: Option<PluginDatabaseExecuteFnV1>,
    pub database_free: Option<PluginFreeBufferFnV1>,
    pub plugin_directory: AbiSlice,
}

impl PluginHostServicesV1 {
    pub const fn new(
        logger: PluginLogFnV1,
        database: Option<PluginDatabaseExecuteFnV1>,
        database_free: Option<PluginFreeBufferFnV1>,
        plugin_directory: AbiSlice
    ) -> Self {
        Self {
            abi_version: PLUGIN_ABI_VERSION_V1,
            struct_size: std::mem::size_of::<Self>() as u32,
            logger: PluginLoggerV1 { emit: logger },
            database,
            database_free,
            plugin_directory,
        }
    }

    pub const fn is_compatible(&self) -> bool {
        self.abi_version == PLUGIN_ABI_VERSION_V1 &&
            self.struct_size >= (std::mem::size_of::<Self>() as u32)
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginLoggerV1 {
    pub emit: PluginLogFnV1,
}

/// Function table implemented by a plugin library.
///
/// No Rust references, `Box`, `Vec`, `String`, trait objects, or Rust enums cross this boundary.
/// All payloads use primitive C-compatible fields or explicitly borrowed/owned byte buffers.
#[repr(C)]
pub struct PluginVTableV1 {
    pub abi_version: u32,
    pub struct_size: u32,
    pub name: PluginNameFnV1,
    pub manifest: PluginManifestFnV1,
    pub on_load: PluginLoadFnV1,
    pub is_loaded: PluginLoadedFnV1,
    pub handle_endpoint: PluginEndpointFnV1,
    pub execute_database: PluginDatabaseFnV1,
    pub on_session_closed: PluginSessionClosedFnV1,
    pub on_unload: PluginLoadFnV1,
    pub free_buffer: PluginFreeBufferFnV1,
    pub destroy: PluginDestroyFnV1,
}

impl PluginVTableV1 {
    pub const fn is_compatible(&self) -> bool {
        self.abi_version == PLUGIN_ABI_VERSION_V1 &&
            self.struct_size >= (std::mem::size_of::<Self>() as u32)
    }
}

/// Opaque plugin instance returned from the dynamic library.
///
/// The host treats `context` as opaque and only invokes operations from the versioned function
/// table. The instance is destroyed by the plugin itself through [`PluginDestroyFnV1`].
#[repr(C)]
pub struct PluginInstanceV1 {
    pub abi_version: u32,
    pub struct_size: u32,
    pub context: *mut c_void,
    pub vtable: *const PluginVTableV1,
}

impl PluginInstanceV1 {
    pub const fn abi_version() -> u32 {
        PLUGIN_ABI_VERSION_V1
    }
}

/// Constructor function exported by a plugin library for ABI v1.
pub type PluginCreateFnV1 = unsafe extern "C" fn(
    host_services: *const PluginHostServicesV1
) -> *mut PluginInstanceV1;

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
        let size = std::mem::size_of::<PluginHostServicesV1>();
        let host = PluginHostServicesV1::new(dummy_log, None, None, AbiSlice::EMPTY);
        assert_eq!(host.abi_version, PLUGIN_ABI_VERSION_V1);
        assert_eq!(host.struct_size as usize, size);
        assert!(host.is_compatible());
    }

    unsafe extern "C" fn dummy_log(_level: u8, _message: AbiSlice) {}
}
