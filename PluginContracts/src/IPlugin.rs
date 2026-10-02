use DatabaseContracts::{ DatabaseError, DatabaseRequest, DatabaseResponse };
use crate::ApiContract::{
    PluginEndpointError,
    PluginEndpointRequest,
    PluginEndpointResponse,
    PluginLifecycleError,
    PluginManifest,
    PluginSessionContext,
    PluginHostServices,
};

/// Declares the in-process plugin implementation and exports the stable ABI v1 constructor.
///
/// The exported ABI does not expose `Box<dyn IPlugin>` or any other Rust ABI type. The plugin
/// implementation remains a normal Rust trait internally and is wrapped behind an opaque handle
/// and versioned C-compatible function table.
#[macro_export]
macro_rules! declare_plugin {
    ($constructor:path) => {
        #[doc(hidden)]
        mod __ezf_plugin_abi_v1 {
            use super::*;
            struct PluginContext {
                plugin: ::std::boxed::Box<dyn $crate::IPlugin::IPlugin>,
            }

            unsafe fn context_mut(
                context: *mut ::std::ffi::c_void,
            ) -> Option<&'static mut PluginContext> {
                if context.is_null() {
                    return None;
                }
                Some(unsafe { &mut *(context as *mut PluginContext) })
            }

            unsafe fn checked_instance(
                instance: *mut $crate::PluginAbi::PluginInstanceV1,
            ) -> Option<&'static mut $crate::PluginAbi::PluginInstanceV1> {
                if instance.is_null() {
                    return None;
                }
                Some(unsafe { &mut *instance })
            }

            fn write_json<T: ::serde::Serialize>(
                value: &T,
                response_out: *mut $crate::PluginAbi::AbiBuffer,
            ) -> i32 {
                if response_out.is_null() {
                    return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                }

                let bytes = match ::serde_json::to_vec(value) {
                    Ok(bytes) => bytes,
                    Err(_) => return $crate::PluginAbi::PLUGIN_STATUS_SERIALIZATION,
                };

                unsafe {
                    *response_out = $crate::PluginAbi::AbiBuffer::from_vec(bytes);
                }
                $crate::PluginAbi::PLUGIN_STATUS_OK
            }

            fn read_json<T: ::serde::de::DeserializeOwned>(
                request: $crate::PluginAbi::AbiSlice,
            ) -> Result<T, i32> {
                let bytes = unsafe { request.to_vec() }
                    .ok_or($crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT)?;
                ::serde_json::from_slice(&bytes)
                    .map_err(|_| $crate::PluginAbi::PLUGIN_STATUS_SERIALIZATION)
            }

            unsafe extern "C" fn plugin_name(
                context: *mut ::std::ffi::c_void,
                response_out: *mut $crate::PluginAbi::AbiBuffer,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_mut(context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    write_json(&context.plugin.name(), response_out)
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            unsafe extern "C" fn plugin_manifest(
                context: *mut ::std::ffi::c_void,
                response_out: *mut $crate::PluginAbi::AbiBuffer,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_mut(context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    write_json(&context.plugin.manifest(), response_out)
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            unsafe extern "C" fn plugin_on_load(
                context: *mut ::std::ffi::c_void,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_mut(context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    if context.plugin.on_load() {
                        $crate::PluginAbi::PLUGIN_STATUS_OK
                    } else {
                        $crate::PluginAbi::PLUGIN_STATUS_PLUGIN_ERROR
                    }
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            unsafe extern "C" fn plugin_is_loaded(
                context: *mut ::std::ffi::c_void,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_mut(context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    if context.plugin.isLoaded() {
                        $crate::PluginAbi::PLUGIN_STATUS_OK
                    } else {
                        $crate::PluginAbi::PLUGIN_STATUS_NOT_INITIALIZED
                    }
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            unsafe extern "C" fn plugin_handle_endpoint(
                context: *mut ::std::ffi::c_void,
                request: $crate::PluginAbi::AbiSlice,
                response_out: *mut $crate::PluginAbi::AbiBuffer,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_mut(context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    let request: $crate::ApiContract::PluginEndpointRequest =
                        match read_json(request) {
                            Ok(request) => request,
                            Err(status) => return status,
                        };
                    let response = context.plugin.handle_endpoint(request);
                    write_json(&response, response_out)
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            unsafe extern "C" fn plugin_execute_database(
                context: *mut ::std::ffi::c_void,
                request: $crate::PluginAbi::AbiSlice,
                response_out: *mut $crate::PluginAbi::AbiBuffer,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_mut(context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    let request: ::DatabaseContracts::DatabaseRequest =
                        match read_json(request) {
                            Ok(request) => request,
                            Err(status) => return status,
                        };
                    let response = context.plugin.execute_database(request);
                    write_json(&response, response_out)
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            unsafe extern "C" fn plugin_on_session_closed(
                context: *mut ::std::ffi::c_void,
                request: $crate::PluginAbi::AbiSlice,
                response_out: *mut $crate::PluginAbi::AbiBuffer,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_mut(context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    let request: $crate::ApiContract::PluginSessionContext =
                        match read_json(request) {
                            Ok(request) => request,
                            Err(status) => return status,
                        };
                    let response = context.plugin.on_session_closed(&request);
                    write_json(&response, response_out)
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            unsafe extern "C" fn plugin_on_unload(
                context: *mut ::std::ffi::c_void,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_mut(context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    if context.plugin.on_unload() {
                        $crate::PluginAbi::PLUGIN_STATUS_OK
                    } else {
                        $crate::PluginAbi::PLUGIN_STATUS_PLUGIN_ERROR
                    }
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            unsafe extern "C" fn plugin_free_buffer(
                buffer: $crate::PluginAbi::AbiBuffer,
            ) {
                unsafe { buffer.free() };
            }

            unsafe extern "C" fn plugin_destroy(
                instance: *mut $crate::PluginAbi::PluginInstanceV1,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(instance_ref) = (unsafe { checked_instance(instance) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };

                    if instance_ref.context.is_null() {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    }

                    let context_ptr = instance_ref.context;
                    let can_destroy = unsafe {
                        (&mut *(context_ptr as *mut PluginContext)).plugin.on_destroy()
                    };
                    if !can_destroy {
                        return $crate::PluginAbi::PLUGIN_STATUS_PLUGIN_ERROR;
                    }

                    instance_ref.context = ::std::ptr::null_mut();
                    unsafe {
                        drop(::std::boxed::Box::from_raw(context_ptr as *mut PluginContext));
                        drop(::std::boxed::Box::from_raw(instance));
                    }
                    $crate::PluginAbi::PLUGIN_STATUS_OK
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            static PLUGIN_VTABLE: $crate::PluginAbi::PluginVTableV1 =
                $crate::PluginAbi::PluginVTableV1 {
                    abi_version: $crate::PluginAbi::PLUGIN_ABI_VERSION_V1,
                    struct_size: ::std::mem::size_of::<$crate::PluginAbi::PluginVTableV1>() as u32,
                    name: plugin_name,
                    manifest: plugin_manifest,
                    on_load: plugin_on_load,
                    is_loaded: plugin_is_loaded,
                    handle_endpoint: plugin_handle_endpoint,
                    execute_database: plugin_execute_database,
                    on_session_closed: plugin_on_session_closed,
                    on_unload: plugin_on_unload,
                    free_buffer: plugin_free_buffer,
                    destroy: plugin_destroy,
                };

            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn InstantiateV1(
                host_services: *const $crate::PluginAbi::PluginHostServicesV1,
            ) -> *mut $crate::PluginAbi::PluginInstanceV1 {
                let result = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    if host_services.is_null() {
                        return ::std::ptr::null_mut();
                    }

                    let host_services = unsafe { *host_services };
                    if host_services.abi_version != $crate::PluginAbi::PLUGIN_ABI_VERSION_V1
                        || host_services.struct_size
                            < ::std::mem::size_of::<$crate::PluginAbi::PluginHostServicesV1>() as u32
                    {
                        return ::std::ptr::null_mut();
                    }

                    let Some(host_services) =
                        $crate::ApiContract::PluginHostServices::from_abi(host_services)
                    else {
                        return ::std::ptr::null_mut();
                    };

                    if !$crate::ApiContract::initialize_plugin_logging(host_services.clone()) {
                        return ::std::ptr::null_mut();
                    }

                    let mut plugin = $constructor();
                    if !plugin.on_load_with_host(&host_services) {
                        let _ = plugin.on_unload();
                        return ::std::ptr::null_mut();
                    }

                    let context = ::std::boxed::Box::new(PluginContext { plugin });
                    let context_ptr = ::std::boxed::Box::into_raw(context) as *mut ::std::ffi::c_void;
                    let instance = ::std::boxed::Box::new($crate::PluginAbi::PluginInstanceV1 {
                        abi_version: $crate::PluginAbi::PLUGIN_ABI_VERSION_V1,
                        struct_size: ::std::mem::size_of::<$crate::PluginAbi::PluginInstanceV1>() as u32,
                        context: context_ptr,
                        vtable: &PLUGIN_VTABLE,
                    });
                    ::std::boxed::Box::into_raw(instance)
                }));

                match result {
                    Ok(instance) => instance,
                    Err(_) => ::std::ptr::null_mut(),
                }
            }
        }
    };
}

pub trait IPlugin: Send + Sync + 'static {
    /// Executes a domain-neutral database request when this plugin exposes database capability.
    fn execute_database(
        &self,
        _request: DatabaseRequest
    ) -> Result<DatabaseResponse, DatabaseError> {
        Err(
            DatabaseError::unsupported(
                format!("plugin '{}' does not provide database execution", self.name())
            )
        )
    }

    fn isLoaded(&self) -> bool {
        false
    }

    fn manifest(&self) -> PluginManifest {
        PluginManifest::without_api(self.name())
    }

    fn handle_endpoint(
        &self,
        request: PluginEndpointRequest
    ) -> Result<PluginEndpointResponse, PluginEndpointError> {
        Err(PluginEndpointError::not_supported(&request.endpoint_id))
    }

    fn name(&self) -> String;

    fn to_box(&self) -> Box<dyn IPlugin>;

    fn on_load(&mut self) -> bool;

    /// Initializes the plugin with the host capabilities belonging to this plugin instance.
    fn on_load_with_host(&mut self, _host_services: &PluginHostServices) -> bool {
        self.on_load()
    }

    fn on_unload(&mut self) -> bool;

    /// Final plugin-instance destruction hook. Returning `false` aborts destruction so the host
    /// can keep the runtime and dynamic library loaded and retry safely.
    fn on_destroy(&mut self) -> bool {
        true
    }

    fn on_session_closed(
        &self,
        _context: &PluginSessionContext
    ) -> Result<(), PluginLifecycleError> {
        Ok(())
    }
}
