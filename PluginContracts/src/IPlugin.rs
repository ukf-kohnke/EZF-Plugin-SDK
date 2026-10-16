use DatabaseContracts::{ DatabaseError, DatabaseRequest, DatabaseResponse };
use crate::ApiContract::{
    PluginCancellation,
    PluginEndpointError,
    PluginEndpointRequest,
    PluginEndpointResponse,
    PluginLifecycleError,
    PluginManifest,
    PluginSessionContext,
    PluginHostServices,
};

/// Declares the in-process plugin implementation and exports the stable ABI V2 constructor.
///
/// ABI V2 passes an explicit cooperative cancellation signal to every potentially long-running
/// plugin operation. The dynamic-library boundary still exposes only opaque handles, a versioned
/// C-compatible function table and serialized byte buffers.
#[macro_export]
macro_rules! declare_plugin {
    ($constructor:path) => {
        #[doc(hidden)]
        mod __ezf_plugin_abi_v2 {
            use super::*;
            struct PluginContext {
                // Request callbacks take a shared lock and may run concurrently. Lifecycle callbacks
                // take the exclusive lock because IPlugin's lifecycle methods require `&mut self`.
                // The host must drain admitted calls before invoking destroy and freeing this context.
                plugin: ::std::sync::RwLock<::std::boxed::Box<dyn $crate::IPlugin::IPlugin>>,
            }

            impl PluginContext {
                fn read(&self) -> ::std::sync::RwLockReadGuard<'_, ::std::boxed::Box<dyn $crate::IPlugin::IPlugin>> {
                    self.plugin.read().unwrap_or_else(|poisoned| poisoned.into_inner())
                }

                fn write(&self) -> ::std::sync::RwLockWriteGuard<'_, ::std::boxed::Box<dyn $crate::IPlugin::IPlugin>> {
                    self.plugin.write().unwrap_or_else(|poisoned| poisoned.into_inner())
                }
            }

            /// Borrows the opaque context only for the current ABI callback. The host's runtime
            /// admission gate guarantees the allocation remains alive until every admitted callback
            /// has returned; unload waits for that gate to become idle before calling destroy.
            unsafe fn context_ref<'a>(
                context: &'a *mut ::std::ffi::c_void,
            ) -> Option<&'a PluginContext> {
                if (*context).is_null() {
                    return None;
                }
                Some(unsafe { &*((*context) as *const PluginContext) })
            }

            fn convert_cancellation(
                cancellation: $crate::PluginAbi::PluginCancellationV2,
            ) -> $crate::ApiContract::PluginCancellation {
                $crate::ApiContract::PluginCancellation::from_abi(cancellation)
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

            fn write_json_and_body<T: ::serde::Serialize>(
                metadata: &T,
                body: ::std::vec::Vec<u8>,
                metadata_out: *mut $crate::PluginAbi::AbiBuffer,
                body_out: *mut $crate::PluginAbi::AbiBuffer,
            ) -> i32 {
                if metadata_out.is_null() || body_out.is_null() {
                    return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                }

                let metadata = match ::serde_json::to_vec(metadata) {
                    Ok(bytes) => bytes,
                    Err(_) => return $crate::PluginAbi::PLUGIN_STATUS_SERIALIZATION,
                };

                unsafe {
                    *metadata_out = $crate::PluginAbi::AbiBuffer::from_vec(metadata);
                    *body_out = $crate::PluginAbi::AbiBuffer::from_vec(body);
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
                cancellation: $crate::PluginAbi::PluginCancellationV2,
                response_out: *mut $crate::PluginAbi::AbiBuffer,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_ref(&context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    let cancellation = convert_cancellation(cancellation);
                    $crate::ApiContract::with_plugin_cancellation(cancellation, ||
                        write_json(&context.read().name(), response_out)
                    )
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            unsafe extern "C" fn plugin_manifest(
                context: *mut ::std::ffi::c_void,
                cancellation: $crate::PluginAbi::PluginCancellationV2,
                response_out: *mut $crate::PluginAbi::AbiBuffer,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_ref(&context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    let cancellation = convert_cancellation(cancellation);
                    $crate::ApiContract::with_plugin_cancellation(cancellation, ||
                        write_json(&context.read().manifest(), response_out)
                    )
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            unsafe extern "C" fn plugin_on_load(
                context: *mut ::std::ffi::c_void,
                cancellation: $crate::PluginAbi::PluginCancellationV2,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_ref(&context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    let cancellation = convert_cancellation(cancellation);
                    $crate::ApiContract::with_plugin_cancellation(cancellation, || {
                        if context.write().on_load() {
                            $crate::PluginAbi::PLUGIN_STATUS_OK
                        } else {
                            $crate::PluginAbi::PLUGIN_STATUS_PLUGIN_ERROR
                        }
                    })
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            unsafe extern "C" fn plugin_is_loaded(
                context: *mut ::std::ffi::c_void,
                cancellation: $crate::PluginAbi::PluginCancellationV2,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_ref(&context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    let cancellation = convert_cancellation(cancellation);
                    $crate::ApiContract::with_plugin_cancellation(cancellation, || {
                        if context.read().isLoaded() {
                            $crate::PluginAbi::PLUGIN_STATUS_OK
                        } else {
                            $crate::PluginAbi::PLUGIN_STATUS_NOT_INITIALIZED
                        }
                    })
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            unsafe extern "C" fn plugin_handle_endpoint(
                context: *mut ::std::ffi::c_void,
                request: $crate::PluginAbi::AbiSlice,
                cancellation: $crate::PluginAbi::PluginCancellationV2,
                response_out: *mut $crate::PluginAbi::AbiBuffer,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_ref(&context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    let request: $crate::ApiContract::PluginEndpointRequest =
                        match read_json(request) {
                            Ok(request) => request,
                            Err(status) => return status,
                        };
                    let cancellation = convert_cancellation(cancellation);
                    $crate::ApiContract::with_plugin_cancellation(cancellation, || {
                        let response = context.read().handle_endpoint(request, &cancellation);
                        write_json(&response, response_out)
                    })
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            unsafe extern "C" fn plugin_handle_endpoint_raw(
                context: *mut ::std::ffi::c_void,
                request: $crate::PluginAbi::AbiSlice,
                cancellation: $crate::PluginAbi::PluginCancellationV2,
                metadata_out: *mut $crate::PluginAbi::AbiBuffer,
                body_out: *mut $crate::PluginAbi::AbiBuffer,
            ) -> i32 {
                if metadata_out.is_null() || body_out.is_null() {
                    return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                }
                unsafe {
                    *metadata_out = $crate::PluginAbi::AbiBuffer::EMPTY;
                    *body_out = $crate::PluginAbi::AbiBuffer::EMPTY;
                }

                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_ref(&context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    let request: $crate::ApiContract::PluginEndpointRequest =
                        match read_json(request) {
                            Ok(request) => request,
                            Err(status) => return status,
                        };
                    let cancellation = convert_cancellation(cancellation);
                    $crate::ApiContract::with_plugin_cancellation(cancellation, || {
                        match context.read().handle_endpoint(request, &cancellation) {
                            Ok(mut response) => {
                                // Move the body out instead of serializing it into an escaped
                                // JSON string and allocating it again on the host side.
                                let body = ::std::mem::take(&mut response.body);
                                let metadata: ::std::result::Result<
                                    $crate::ApiContract::PluginEndpointResponse,
                                    $crate::ApiContract::PluginEndpointError,
                                > = Ok(response);
                                write_json_and_body(&metadata, body, metadata_out, body_out)
                            }
                            Err(error) => {
                                let metadata: ::std::result::Result<
                                    $crate::ApiContract::PluginEndpointResponse,
                                    $crate::ApiContract::PluginEndpointError,
                                > = Err(error);
                                write_json_and_body(
                                    &metadata,
                                    ::std::vec::Vec::new(),
                                    metadata_out,
                                    body_out,
                                )
                            }
                        }
                    })
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            unsafe extern "C" fn plugin_execute_database(
                context: *mut ::std::ffi::c_void,
                request: $crate::PluginAbi::AbiSlice,
                cancellation: $crate::PluginAbi::PluginCancellationV2,
                response_out: *mut $crate::PluginAbi::AbiBuffer,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_ref(&context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    let request: ::DatabaseContracts::DatabaseRequest =
                        match read_json(request) {
                            Ok(request) => request,
                            Err(status) => return status,
                        };
                    let cancellation = convert_cancellation(cancellation);
                    $crate::ApiContract::with_plugin_cancellation(cancellation, || {
                        let response = context.read().execute_database(request, &cancellation);
                        write_json(&response, response_out)
                    })
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            unsafe extern "C" fn plugin_on_session_closed(
                context: *mut ::std::ffi::c_void,
                request: $crate::PluginAbi::AbiSlice,
                cancellation: $crate::PluginAbi::PluginCancellationV2,
                response_out: *mut $crate::PluginAbi::AbiBuffer,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_ref(&context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    let request: $crate::ApiContract::PluginSessionContext =
                        match read_json(request) {
                            Ok(request) => request,
                            Err(status) => return status,
                        };
                    let cancellation = convert_cancellation(cancellation);
                    $crate::ApiContract::with_plugin_cancellation(cancellation, || {
                        let response = context.read().on_session_closed(&request, &cancellation);
                        write_json(&response, response_out)
                    })
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            unsafe extern "C" fn plugin_on_unload(
                context: *mut ::std::ffi::c_void,
                cancellation: $crate::PluginAbi::PluginCancellationV2,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    let Some(context) = (unsafe { context_ref(&context) }) else {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    };
                    let cancellation = convert_cancellation(cancellation);
                    $crate::ApiContract::with_plugin_cancellation(cancellation, || {
                        if context.write().on_unload(&cancellation) {
                            $crate::PluginAbi::PLUGIN_STATUS_OK
                        } else {
                            $crate::PluginAbi::PLUGIN_STATUS_PLUGIN_ERROR
                        }
                    })
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
                instance: *mut $crate::PluginAbi::PluginInstanceV2,
                cancellation: $crate::PluginAbi::PluginCancellationV2,
            ) -> i32 {
                match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    if instance.is_null() {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    }

                    let context_ptr = unsafe { (*instance).context };
                    if context_ptr.is_null() {
                        return $crate::PluginAbi::PLUGIN_STATUS_INVALID_ARGUMENT;
                    }

                    let cancellation = convert_cancellation(cancellation);
                    let can_destroy = $crate::ApiContract::with_plugin_cancellation(cancellation, || {
                        let Some(context) = (unsafe { context_ref(&context_ptr) }) else {
                            return false;
                        };
                        context.write().on_destroy(&cancellation)
                    });
                    if !can_destroy {
                        return $crate::PluginAbi::PLUGIN_STATUS_PLUGIN_ERROR;
                    }

                    // No plugin lock guard or context reference may survive the allocation teardown.
                    unsafe {
                        (*instance).context = ::std::ptr::null_mut();
                        drop(::std::boxed::Box::from_raw(context_ptr as *mut PluginContext));
                        drop(::std::boxed::Box::from_raw(instance));
                    }
                    $crate::PluginAbi::PLUGIN_STATUS_OK
                })) {
                    Ok(status) => status,
                    Err(_) => $crate::PluginAbi::PLUGIN_STATUS_PANIC,
                }
            }

            static PLUGIN_VTABLE: $crate::PluginAbi::PluginVTableV2Ext =
                $crate::PluginAbi::PluginVTableV2Ext {
                    base: $crate::PluginAbi::PluginVTableV2 {
                        abi_version: $crate::PluginAbi::PLUGIN_ABI_VERSION_V2,
                        struct_size: ::std::mem::size_of::<$crate::PluginAbi::PluginVTableV2Ext>() as u32,
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
                    },
                    handle_endpoint_raw: Some(plugin_handle_endpoint_raw),
                };

            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn InstantiateV2(
                host_services: *const $crate::PluginAbi::PluginHostServicesV2,
            ) -> *mut $crate::PluginAbi::PluginInstanceV2 {
                let result = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                    if host_services.is_null() {
                        return ::std::ptr::null_mut();
                    }

                    let host_services = unsafe { *host_services };
                    if host_services.abi_version != $crate::PluginAbi::PLUGIN_ABI_VERSION_V2
                        || host_services.struct_size
                            < ::std::mem::size_of::<$crate::PluginAbi::PluginHostServicesV2>() as u32
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
                        let cancellation = $crate::ApiContract::PluginCancellation::from_abi(
                            $crate::PluginAbi::PluginCancellationV2 {
                                context: ::std::ptr::null(),
                                is_cancelled: never_cancelled,
                            }
                        );
                        let _ = plugin.on_unload(&cancellation);
                        return ::std::ptr::null_mut();
                    }

                    let context = ::std::boxed::Box::new(PluginContext { plugin: ::std::sync::RwLock::new(plugin) });
                    let context_ptr = ::std::boxed::Box::into_raw(context) as *mut ::std::ffi::c_void;
                    let instance = ::std::boxed::Box::new($crate::PluginAbi::PluginInstanceV2 {
                        abi_version: $crate::PluginAbi::PLUGIN_ABI_VERSION_V2,
                        struct_size: ::std::mem::size_of::<$crate::PluginAbi::PluginInstanceV2>() as u32,
                        context: context_ptr,
                        vtable: &PLUGIN_VTABLE.base,
                    });
                    ::std::boxed::Box::into_raw(instance)
                }));

                match result {
                    Ok(instance) => instance,
                    Err(_) => ::std::ptr::null_mut(),
                }
            }

            unsafe extern "C" fn never_cancelled(_context: *const ::std::ffi::c_void) -> bool {
                false
            }
        }
    };
}

pub trait IPlugin: Send + Sync + 'static {
    /// Executes a domain-neutral database request when this plugin exposes database capability.
    fn execute_database(
        &self,
        _request: DatabaseRequest,
        _cancellation: &PluginCancellation
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
        request: PluginEndpointRequest,
        _cancellation: &PluginCancellation
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

    /// Cooperative unload callback. The host has already quiesced normal calls before entering it.
    fn on_unload(&mut self, _cancellation: &PluginCancellation) -> bool;

    /// Final plugin-instance destruction hook. Returning `false` aborts destruction so the host
    /// can keep the runtime and dynamic library loaded and retry safely.
    fn on_destroy(&mut self, _cancellation: &PluginCancellation) -> bool {
        true
    }

    fn on_session_closed(
        &self,
        _context: &PluginSessionContext,
        _cancellation: &PluginCancellation
    ) -> Result<(), PluginLifecycleError> {
        Ok(())
    }
}
