/// Defines the API contract between the backend and plugins.
///
/// This module contains structures and validation logic for plugin manifests, endpoints, and requests/responses.
/// It enforces versioning, naming conventions, and security constraints for plugin endpoints.
///
/// # Plugin Contract Version
/// - Current version: [`PLUGIN_CONTRACT_VERSION`] (7)
/// - Maximum request body size: [`MAX_PLUGIN_REQUEST_BODY_BYTES`] (64 MiB)
///
/// # Key Types
/// - [`PluginManifest`]: Describes a plugin's metadata and endpoints
/// - [`PluginEndpointDescriptor`]: Defines an individual API endpoint
/// - [`PluginEndpointRequest`]/[`PluginEndpointResponse`]: Request/response structures
/// - [`PluginSessionContext`]: Sanitized lifecycle context for discarded sessions
/// - [`PluginCatalog`]: Aggregates multiple plugins with validation and deduplication
///
/// # Validation
/// The catalog validates:
/// - No duplicate plugin names (case-insensitive)
/// - No conflicting routes across plugins
/// /// - Valid identifiers, paths, and media types
/// - No reserved route collisions
/// - Proper request body configuration
///
/// # Example
/// ```ignore
/// let mut plugin = PluginManifest::new("MyPlugin", "1.0.0", PluginKind::Domain);
/// plugin.endpoints.push(
///     PluginEndpointDescriptor::get("list", "/items")
///         .documented("Items", "List all items", Some("Returns a JSON array"))
/// );
/// let catalog = PluginCatalog::from_manifests([plugin])?;
/// ```
use DatabaseContracts::{ DatabaseError, DatabaseRequest, DatabaseResponse };
use PluginAbi::{
    AbiBuffer,
    AbiSlice,
    PluginCancellationV2,
    PluginDatabaseExecuteFnV2,
    PluginFreeBufferFnV2,
    PluginHostServicesV2,
    PLUGIN_ABI_VERSION_V2,
};
use serde::{ Deserialize, Serialize };
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{ BTreeMap, BTreeSet },
    fmt,
    path::PathBuf,
    sync::{ atomic::{ AtomicBool, Ordering }, OnceLock },
};

pub const PLUGIN_CONTRACT_VERSION: u32 = 7;
pub const JSON_BODY_LIMIT: u64 = 2 * 1024 * 1024;
pub const MAX_PLUGIN_REQUEST_BODY_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_PLUGIN_OPENAPI_EXAMPLE_BYTES: usize = 128 * 1024;

/// Internal alias for the stable ABI callback types. These are no longer used to expose Rust
/// contract objects across the dynamic-library boundary; payloads are serialized bytes.
pub type PluginDatabaseExecuteFn = PluginDatabaseExecuteFnV2;
pub type PluginFreeBufferFn = PluginFreeBufferFnV2;

/// Cooperative cancellation state exposed to Rust plugin implementations.
///
/// The value is a lightweight view over the ABI-owned cancellation callback and is only valid
/// during the plugin operation that received it.
#[derive(Clone, Copy)]
pub struct PluginCancellation {
    abi: PluginCancellationV2,
}

impl PluginCancellation {
    pub fn from_abi(abi: PluginCancellationV2) -> Self {
        Self { abi }
    }

    pub fn uncancelled() -> Self {
        Self::from_abi(PluginCancellationV2 {
            context: std::ptr::null(),
            is_cancelled: never_cancelled,
        })
    }

    pub fn as_abi(&self) -> PluginCancellationV2 {
        self.abi
    }

    pub fn is_cancelled(&self) -> bool {
        unsafe { (self.abi.is_cancelled)(self.abi.context) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginLogLevel {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

pub type PluginLogFn = PluginAbi::PluginLogFnV2;

#[derive(Clone, Copy)]
pub struct PluginLogger {
    pub emit: PluginLogFn,
}

impl PluginLogger {
    pub fn log(&self, level: PluginLogLevel, message: &str) {
        unsafe {
            (self.emit)(level as u8, AbiSlice::from_bytes(message.as_bytes()));
        }
    }
}

#[derive(Clone, Copy)]
pub struct PluginDatabaseCapability {
    pub execute: PluginDatabaseExecuteFn,
    pub free: PluginFreeBufferFn,
}

/// Host services used internally by the Rust `IPlugin` implementation. The dynamic-library ABI
/// uses `PluginAbi::PluginHostServicesV2` instead; this type is reconstructed inside the plugin
/// and therefore never crosses the ABI boundary.
#[derive(Clone)]
pub struct PluginHostServices {
    pub version: u32,
    pub logger: PluginLogger,
    pub database: Option<PluginDatabaseCapability>,
    plugin_directory: Option<PathBuf>,
}

impl PluginHostServices {
    pub fn from_abi(host: PluginHostServicesV2) -> Option<Self> {
        if host.abi_version != PLUGIN_ABI_VERSION_V2 || !host.is_compatible() {
            return None;
        }

        let plugin_directory = if host.plugin_directory.len == 0 {
            None
        } else {
            let bytes = (unsafe { host.plugin_directory.to_vec() })?;
            let value = String::from_utf8(bytes).ok()?;
            Some(PathBuf::from(value))
        };

        let database = match (host.database, host.database_free) {
            (Some(execute), Some(free)) => Some(PluginDatabaseCapability { execute, free }),
            (None, None) => None,
            _ => {
                return None;
            }
        };

        Some(Self {
            version: host.abi_version,
            logger: PluginLogger { emit: host.logger.emit },
            database,
            plugin_directory,
        })
    }

    pub fn plugin_directory(&self) -> Option<PathBuf> {
        self.plugin_directory.clone()
    }
}

struct HostForwardingLogger {
    logger: PluginLogger,
}

impl log::Log for HostForwardingLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Trace
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let level = match record.level() {
            log::Level::Error => PluginLogLevel::Error,
            log::Level::Warn => PluginLogLevel::Warn,
            log::Level::Info => PluginLogLevel::Info,
            log::Level::Debug => PluginLogLevel::Debug,
            log::Level::Trace => PluginLogLevel::Trace,
        };

        let message = format!("[{}] {}", record.target(), record.args());
        self.logger.log(level, &message);
    }

    fn flush(&self) {}
}

static HOST_FORWARDING_LOGGER: OnceLock<HostForwardingLogger> = OnceLock::new();
static HOST_FORWARDING_LOGGER_INSTALLED: AtomicBool = AtomicBool::new(false);

pub fn initialize_plugin_logging(host: PluginHostServices) -> bool {
    if host.version != PLUGIN_ABI_VERSION_V2 {
        return false;
    }

    if HOST_FORWARDING_LOGGER_INSTALLED.load(Ordering::Acquire) {
        return true;
    }

    let logger = HOST_FORWARDING_LOGGER.get_or_init(|| HostForwardingLogger {
        logger: host.logger,
    });

    if log::set_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Trace);
        HOST_FORWARDING_LOGGER_INSTALLED.store(true, Ordering::Release);
        return true;
    }

    HOST_FORWARDING_LOGGER_INSTALLED.load(Ordering::Acquire)
}

thread_local! {
    static ACTIVE_DATABASE_EXECUTOR: RefCell<Option<PluginDatabaseCapability>> = const {
        RefCell::new(None)
    };

    static ACTIVE_PLUGIN_CANCELLATION: RefCell<Option<PluginCancellation>> = const {
        RefCell::new(None)
    };
}

struct PluginCancellationScope {
    previous: Option<PluginCancellation>,
}

impl Drop for PluginCancellationScope {
    fn drop(&mut self) {
        ACTIVE_PLUGIN_CANCELLATION.with(|slot| {
            let _ = slot.replace(self.previous.take());
        });
    }
}

pub fn with_plugin_cancellation<T>(
    cancellation: PluginCancellation,
    operation: impl FnOnce() -> T
) -> T {
    let previous = ACTIVE_PLUGIN_CANCELLATION.with(|slot| slot.replace(Some(cancellation)));
    let _scope = PluginCancellationScope { previous };
    operation()
}

pub fn current_plugin_cancellation() -> Option<PluginCancellation> {
    ACTIVE_PLUGIN_CANCELLATION.with(|slot| slot.borrow().as_ref().copied())
}

struct DatabaseExecutorScope {
    previous: Option<PluginDatabaseCapability>,
}

impl Drop for DatabaseExecutorScope {
    fn drop(&mut self) {
        ACTIVE_DATABASE_EXECUTOR.with(|slot| {
            let _ = slot.replace(self.previous.take());
        });
    }
}

pub fn with_database_executor<T>(
    executor: Option<PluginDatabaseCapability>,
    operation: impl FnOnce() -> T
) -> T {
    let previous = ACTIVE_DATABASE_EXECUTOR.with(|slot| slot.replace(executor));
    let _scope = DatabaseExecutorScope { previous };
    operation()
}

pub fn database_executor() -> Option<PluginDatabaseCapability> {
    ACTIVE_DATABASE_EXECUTOR.with(|slot| slot.borrow().as_ref().copied())
}

unsafe extern "C" fn never_cancelled(_context: *const std::ffi::c_void) -> bool {
    false
}

pub fn execute_database(request: DatabaseRequest) -> Result<DatabaseResponse, DatabaseError> {
    let Some(capability) = database_executor() else {
        return Err(DatabaseError::unsupported("plugin host does not expose database execution"));
    };

    let request_bytes = serde_json
        ::to_vec(&request)
        .map_err(|error|
            DatabaseError::new(
                DatabaseContracts::DatabaseErrorKind::InvalidRequest,
                error.to_string()
            )
        )?;
    let mut response = AbiBuffer::EMPTY;
    let cancellation = current_plugin_cancellation().unwrap_or_else(
        PluginCancellation::uncancelled
    );
    if cancellation.is_cancelled() {
        return Err(
            DatabaseError::new(
                DatabaseContracts::DatabaseErrorKind::Query,
                "database request cancelled"
            )
        );
    }

    let status = unsafe {
        (capability.execute)(
            AbiSlice::from_bytes(&request_bytes),
            cancellation.as_abi(),
            &mut response
        )
    };

    if status != PluginAbi::PLUGIN_STATUS_OK {
        if !response.ptr.is_null() {
            unsafe {
                (capability.free)(response);
            }
        }
        return Err(
            DatabaseError::new(DatabaseContracts::DatabaseErrorKind::Query, if
                status == PluginAbi::PLUGIN_STATUS_CANCELLED
            {
                "host database capability cancelled".to_owned()
            } else {
                format!("host database capability failed with ABI status {status}")
            })
        );
    }

    let bytes = unsafe { response.to_vec() };
    unsafe {
        (capability.free)(response);
    }
    let bytes = bytes.ok_or_else(||
        DatabaseError::new(
            DatabaseContracts::DatabaseErrorKind::Decode,
            "host database capability returned an invalid response buffer"
        )
    )?;

    serde_json
        ::from_slice::<Result<DatabaseResponse, DatabaseError>>(&bytes)
        .map_err(|error|
            DatabaseError::new(
                DatabaseContracts::DatabaseErrorKind::Decode,
                format!("failed to decode host database capability response: {error}")
            )
        )?
}

pub fn execute_database_query(
    request: DatabaseRequest,
    operation: &str
) -> Result<DatabaseContracts::ResultSet, String> {
    match execute_database(request).map_err(|error| error.to_string())? {
        DatabaseResponse::Query(result_set) => Ok(result_set),
        DatabaseResponse::Execute(_) => {
            Err(format!("database host returned an execution result for {operation}"))
        }
    }
}

pub fn execute_database_statement(request: DatabaseRequest, operation: &str) -> Result<(), String> {
    execute_database_statement_result(request, operation).map(|_| ())
}

pub fn execute_database_statement_result(
    request: DatabaseRequest,
    operation: &str
) -> Result<DatabaseContracts::ExecutionResult, String> {
    match execute_database(request).map_err(|error| error.to_string())? {
        DatabaseResponse::Execute(result) => Ok(result),
        DatabaseResponse::Query(_) => {
            Err(format!("database host returned a query result for {operation}"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginKind {
    Domain,
    DataSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PluginHttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl PluginHttpMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }

    pub fn as_openapi_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Post => "post",
            Self::Put => "put",
            Self::Patch => "patch",
            Self::Delete => "delete",
        }
    }
}

impl fmt::Display for PluginHttpMethod {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginQueryParameterDescriptor {
    pub name: String,
    pub required: bool,
    pub description: Option<String>,
    pub example: Option<String>,
    pub multiline: bool,
}

impl PluginQueryParameterDescriptor {
    pub fn required(name: impl Into<String>) -> Self {
        Self::new(name, true)
    }

    pub fn optional(name: impl Into<String>) -> Self {
        Self::new(name, false)
    }

    fn new(name: impl Into<String>, required: bool) -> Self {
        Self {
            name: name.into(),
            required,
            description: None,
            example: None,
            multiline: false,
        }
    }

    pub fn documented(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn example(mut self, example: impl Into<String>) -> Self {
        self.example = Some(example.into());
        self
    }

    pub fn multiline(mut self) -> Self {
        self.multiline = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginRequestBodyDescriptor {
    pub description: Option<String>,
    pub example: Option<Value>,
}

impl PluginRequestBodyDescriptor {
    pub fn documented(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn example(mut self, example: Value) -> Self {
        self.example = Some(example);
        self
    }
}

pub fn get(
    tag: &str,
    id: &str,
    path: &str,
    summary: &str,
    descr: Option<&str>,
    query_required: bool
) -> PluginEndpointDescriptor {
    let mut endpoint = PluginEndpointDescriptor::get(id, path).documented(tag, summary, descr);

    if let Some(query_description) = descr {
        let parameter = if query_required {
            PluginQueryParameterDescriptor::required("query")
        } else {
            PluginQueryParameterDescriptor::optional("query")
        };

        endpoint = endpoint.query_parameter(parameter.documented(query_description).multiline());
    }

    endpoint
}

pub fn post(
    tag: &str,
    id: &str,
    path: &str,
    summary: &str,
    descr: Option<&str>
) -> PluginEndpointDescriptor {
    PluginEndpointDescriptor::post(id, path)
        .json_body(JSON_BODY_LIMIT)
        .documented(tag, summary, descr)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEndpointDescriptor {
    pub id: String,
    pub method: PluginHttpMethod,
    pub path: String,
    pub consumes: Option<String>,
    pub produces: String,
    pub max_body_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_body: Option<PluginRequestBodyDescriptor>,
    #[serde(default)]
    pub query_parameters: Vec<PluginQueryParameterDescriptor>,
    pub tag: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
}

impl PluginEndpointDescriptor {
    pub fn get(id: impl Into<String>, path: impl Into<String>) -> Self {
        Self::new(id, PluginHttpMethod::Get, path)
    }

    pub fn post(id: impl Into<String>, path: impl Into<String>) -> Self {
        Self::new(id, PluginHttpMethod::Post, path)
    }

    pub fn redirect(id: impl Into<String>, path: impl Into<String>) -> Self {
        let mut endpoint = Self::get(id, path);
        endpoint.produces = "text/plain; charset=utf-8".to_owned();
        endpoint
    }

    fn new(id: impl Into<String>, method: PluginHttpMethod, path: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            method,
            path: path.into(),
            consumes: None,
            produces: "application/json".to_owned(),
            max_body_bytes: None,
            request_body: None,
            query_parameters: Vec::new(),
            tag: None,
            summary: None,
            description: None,
        }
    }

    pub fn json_body(mut self, max_body_bytes: u64) -> Self {
        self.consumes = Some("application/json".to_owned());
        self.max_body_bytes = Some(max_body_bytes);
        self
    }

    pub fn request_body_description(mut self, description: impl Into<String>) -> Self {
        self.request_body.get_or_insert_with(PluginRequestBodyDescriptor::default).description =
            Some(description.into());
        self
    }

    pub fn request_body_example(mut self, example: Value) -> Self {
        self.request_body.get_or_insert_with(PluginRequestBodyDescriptor::default).example =
            Some(example);
        self
    }

    pub fn query_parameter(mut self, parameter: PluginQueryParameterDescriptor) -> Self {
        self.query_parameters.push(parameter);
        self
    }

    pub fn documented(
        mut self,
        tag: impl Into<String>,
        summary: impl Into<String>,
        descr: Option<impl Into<String>>
    ) -> Self {
        self.tag = Some(tag.into());
        self.summary = Some(summary.into());
        self.description = descr.map(|d| d.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub contract_version: u32,
    pub name: String,
    pub version: String,
    pub kind: PluginKind,
    pub endpoints: Vec<PluginEndpointDescriptor>,
}

impl PluginManifest {
    pub fn new(name: impl Into<String>, version: impl Into<String>, kind: PluginKind) -> Self {
        Self {
            contract_version: PLUGIN_CONTRACT_VERSION,
            name: name.into(),
            version: version.into(),
            kind,
            endpoints: Vec::new(),
        }
    }

    pub fn without_api(name: impl Into<String>) -> Self {
        Self::new(name, "unknown", PluginKind::Domain)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginPrincipal {
    pub user_id: String,
    pub username: String,
    pub given_name: String,
    pub family_name: String,
    pub groups: Vec<String>,
    pub session_id: String,
}

/// Sanitized session context sent to loaded plugins when a backend session is
/// discarded.
///
/// The value is an opaque digest created by `UserApi`; it is the same value
/// exposed to dynamic endpoints through [`PluginPrincipal::session_id`]. The
/// raw session cookie, JWTs and identity-provider tokens never cross the plugin
/// boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSessionContext {
    pub session_id: String,
}

impl PluginSessionContext {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
        }
    }
}

/// Error returned by a plugin lifecycle callback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginLifecycleError {
    pub code: String,
    pub message: String,
}

impl PluginLifecycleError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for PluginLifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PluginLifecycleError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEndpointRequest {
    pub endpoint_id: String,
    pub method: PluginHttpMethod,
    pub raw_path: String,
    pub base_path: String,
    pub raw_query: Option<String>,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    pub principal: PluginPrincipal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEndpointResponse {
    pub status: u16,
    pub content_type: String,
    pub headers: Vec<PluginHeader>,
    pub body: Vec<u8>,
}

impl PluginEndpointResponse {
    pub fn json<T: Serialize>(status: u16, value: &T) -> Result<Self, PluginEndpointError> {
        let body = serde_json
            ::to_vec(value)
            .map_err(|error| {
                PluginEndpointError::internal(
                    "plugin_response_serialization_failed",
                    "The plugin response could not be serialized",
                    error.to_string()
                )
            })?;

        Ok(Self {
            status,
            content_type: "application/json".to_owned(),
            headers: Vec::new(),
            body,
        })
    }

    pub fn redirect(location: impl Into<String>) -> Self {
        Self {
            status: 303,
            content_type: "text/plain; charset=utf-8".to_owned(),
            headers: vec![PluginHeader {
                name: "Location".to_owned(),
                value: location.into(),
            }],
            body: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEndpointError {
    pub status: u16,
    pub code: String,
    pub user_message: String,
    pub detailed_message: Option<String>,
}

impl PluginEndpointError {
    pub fn bad_request(
        code: impl Into<String>,
        user_message: impl Into<String>,
        detailed_message: impl Into<String>
    ) -> Self {
        Self::new(400, code, user_message, Some(detailed_message.into()))
    }

    pub fn not_found(
        code: impl Into<String>,
        user_message: impl Into<String>,
        detailed_message: impl Into<String>
    ) -> Self {
        Self::new(404, code, user_message, Some(detailed_message.into()))
    }

    pub fn internal(
        code: impl Into<String>,
        user_message: impl Into<String>,
        detailed_message: impl Into<String>
    ) -> Self {
        Self::new(500, code, user_message, Some(detailed_message.into()))
    }

    pub fn not_supported(endpoint_id: &str) -> Self {
        Self::new(
            404,
            "plugin_endpoint_not_supported",
            "The plugin endpoint is not supported",
            Some(format!("Unknown endpoint id '{endpoint_id}'"))
        )
    }

    pub fn new(
        status: u16,
        code: impl Into<String>,
        user_message: impl Into<String>,
        detailed_message: Option<String>
    ) -> Self {
        Self {
            status,
            code: code.into(),
            user_message: user_message.into(),
            detailed_message,
        }
    }
}

impl fmt::Display for PluginEndpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.detailed_message.as_deref() {
            Some(details) => write!(formatter, "{}: {}", self.user_message, details),
            None => formatter.write_str(&self.user_message),
        }
    }
}

impl std::error::Error for PluginEndpointError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEndpointBinding {
    pub plugin_name: String,
    pub endpoint: PluginEndpointDescriptor,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCatalog {
    pub plugins: BTreeMap<String, PluginManifest>,
    pub endpoints: Vec<PluginEndpointBinding>,
}

impl PluginCatalog {
    pub fn from_manifests<I>(manifests: I) -> Result<Self, PluginCatalogError>
        where I: IntoIterator<Item = PluginManifest>
    {
        let mut catalog = Self::default();
        let mut route_owners: BTreeMap<(PluginHttpMethod, String), String> = BTreeMap::new();

        for manifest in manifests {
            validate_manifest(&manifest)?;
            let normalized_plugin_name = normalize_identifier(&manifest.name);

            if
                let Some(existing) = catalog.plugins
                    .keys()
                    .find(|name| { normalize_identifier(name) == normalized_plugin_name })
            {
                return Err(PluginCatalogError::DuplicatePlugin {
                    plugin: manifest.name,
                    existing: existing.clone(),
                });
            }

            let mut endpoint_ids = BTreeSet::new();
            for endpoint in &manifest.endpoints {
                validate_endpoint(&manifest.name, endpoint)?;
                let endpoint_id = normalize_identifier(&endpoint.id);
                if !endpoint_ids.insert(endpoint_id) {
                    return Err(PluginCatalogError::DuplicateEndpointId {
                        plugin: manifest.name.clone(),
                        endpoint_id: endpoint.id.clone(),
                    });
                }

                let normalized_path = normalize_plugin_path(&endpoint.path)?;
                if is_reserved_route(&normalized_path) {
                    return Err(PluginCatalogError::ReservedRoute {
                        plugin: manifest.name.clone(),
                        method: endpoint.method,
                        path: endpoint.path.clone(),
                    });
                }

                let route_key = (endpoint.method, normalized_path);
                if let Some(existing_plugin) = route_owners.get(&route_key) {
                    return Err(PluginCatalogError::DuplicateRoute {
                        method: endpoint.method,
                        path: endpoint.path.clone(),
                        existing_plugin: existing_plugin.clone(),
                        plugin: manifest.name.clone(),
                    });
                }

                route_owners.insert(route_key, manifest.name.clone());
                catalog.endpoints.push(PluginEndpointBinding {
                    plugin_name: manifest.name.clone(),
                    endpoint: endpoint.clone(),
                });
            }

            catalog.plugins.insert(manifest.name.clone(), manifest);
        }

        catalog.endpoints.sort_by(|left, right| {
            left.endpoint.path
                .cmp(&right.endpoint.path)
                .then(left.endpoint.method.cmp(&right.endpoint.method))
                .then(left.plugin_name.cmp(&right.plugin_name))
        });

        Ok(catalog)
    }

    /// Resolves the plugin owning a REST endpoint by its externally visible HTTP contract.
    ///
    /// The plugin name is deliberately not part of the lookup key. This is the important
    /// boundary that keeps callers independent from concrete plugin names: the catalog owns
    /// the mapping from `(method, path)` to the plugin that implements it.
    pub fn plugin_for_endpoint(&self, method: PluginHttpMethod, path: &str) -> Option<&str> {
        let normalized_path = normalize_plugin_path(path).ok()?;
        self.endpoints
            .iter()
            .find_map(|binding| {
                (
                    binding.endpoint.method == method &&
                    normalize_plugin_path(&binding.endpoint.path).ok().as_deref() ==
                        Some(normalized_path.as_str())
                ).then_some(binding.plugin_name.as_str())
            })
    }

    /// Returns the catalog binding for an HTTP endpoint.
    pub fn endpoint_binding(
        &self,
        method: PluginHttpMethod,
        path: &str
    ) -> Option<&PluginEndpointBinding> {
        let normalized_path = normalize_plugin_path(path).ok()?;
        self.endpoints
            .iter()
            .find(|binding| {
                binding.endpoint.method == method &&
                    normalize_plugin_path(&binding.endpoint.path).ok().as_deref() ==
                        Some(normalized_path.as_str())
            })
    }

    pub fn endpoint_count(&self) -> usize {
        self.endpoints.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginCatalogError {
    UnsupportedContractVersion {
        plugin: String,
        found: u32,
        expected: u32,
    },
    InvalidPluginName,
    InvalidPluginVersion {
        plugin: String,
    },
    DuplicatePlugin {
        plugin: String,
        existing: String,
    },
    InvalidEndpointPath {
        path: String,
        reason: String,
    },
    InvalidEndpointId {
        plugin: String,
        endpoint_id: String,
    },
    InvalidEndpointMediaType {
        plugin: String,
        endpoint_id: String,
        field: &'static str,
        value: String,
    },
    InvalidEndpointBodyContract {
        plugin: String,
        endpoint_id: String,
        reason: String,
    },
    InvalidEndpointQueryParameter {
        plugin: String,
        endpoint_id: String,
        parameter: String,
        reason: String,
    },
    DuplicateEndpointQueryParameter {
        plugin: String,
        endpoint_id: String,
        parameter: String,
    },
    DuplicateEndpointId {
        plugin: String,
        endpoint_id: String,
    },
    DuplicateRoute {
        method: PluginHttpMethod,
        path: String,
        existing_plugin: String,
        plugin: String,
    },
    ReservedRoute {
        plugin: String,
        method: PluginHttpMethod,
        path: String,
    },
}

impl fmt::Display for PluginCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedContractVersion { plugin, found, expected } =>
                write!(
                    formatter,
                    "plugin '{plugin}' uses contract version {found}, expected {expected}"
                ),
            Self::InvalidPluginName => formatter.write_str("plugin name must not be empty"),
            Self::InvalidPluginVersion { plugin } => {
                write!(formatter, "plugin '{plugin}' has no version")
            }
            Self::DuplicatePlugin { plugin, existing } =>
                write!(
                    formatter,
                    "plugin name '{plugin}' conflicts with already loaded plugin '{existing}'"
                ),
            Self::InvalidEndpointPath { path, reason } => {
                write!(formatter, "invalid plugin endpoint path '{path}': {reason}")
            }
            Self::InvalidEndpointId { plugin, endpoint_id } =>
                write!(formatter, "plugin '{plugin}' exports invalid endpoint id '{endpoint_id}'"),
            Self::InvalidEndpointMediaType { plugin, endpoint_id, field, value } =>
                write!(
                    formatter,
                    "plugin '{plugin}' endpoint '{endpoint_id}' has invalid {field} media type '{value}'"
                ),
            Self::InvalidEndpointBodyContract { plugin, endpoint_id, reason } =>
                write!(
                    formatter,
                    "plugin '{plugin}' endpoint '{endpoint_id}' has an invalid body contract: {reason}"
                ),
            Self::InvalidEndpointQueryParameter { plugin, endpoint_id, parameter, reason } =>
                write!(
                    formatter,
                    "plugin '{plugin}' endpoint '{endpoint_id}' has invalid query parameter '{parameter}': {reason}"
                ),
            Self::DuplicateEndpointQueryParameter { plugin, endpoint_id, parameter } =>
                write!(
                    formatter,
                    "plugin '{plugin}' endpoint '{endpoint_id}' declares query parameter '{parameter}' more than once"
                ),
            Self::DuplicateEndpointId { plugin, endpoint_id } =>
                write!(
                    formatter,
                    "plugin '{plugin}' exports endpoint id '{endpoint_id}' more than once"
                ),
            Self::DuplicateRoute { method, path, existing_plugin, plugin } =>
                write!(
                    formatter,
                    "route {method} {path} is exported by both '{existing_plugin}' and '{plugin}'"
                ),
            Self::ReservedRoute { plugin, method, path } =>
                write!(
                    formatter,
                    "plugin '{plugin}' attempts to export reserved route {method} {path}"
                ),
        }
    }
}

impl std::error::Error for PluginCatalogError {}

fn validate_manifest(manifest: &PluginManifest) -> Result<(), PluginCatalogError> {
    if manifest.contract_version != PLUGIN_CONTRACT_VERSION {
        return Err(PluginCatalogError::UnsupportedContractVersion {
            plugin: manifest.name.clone(),
            found: manifest.contract_version,
            expected: PLUGIN_CONTRACT_VERSION,
        });
    }

    if !is_valid_identifier(&manifest.name) {
        return Err(PluginCatalogError::InvalidPluginName);
    }

    if manifest.version.trim().is_empty() {
        return Err(PluginCatalogError::InvalidPluginVersion {
            plugin: manifest.name.clone(),
        });
    }

    Ok(())
}

fn validate_endpoint(
    plugin_name: &str,
    endpoint: &PluginEndpointDescriptor
) -> Result<(), PluginCatalogError> {
    if !is_valid_identifier(&endpoint.id) {
        return Err(PluginCatalogError::InvalidEndpointId {
            plugin: plugin_name.to_owned(),
            endpoint_id: endpoint.id.clone(),
        });
    }

    validate_media_type(plugin_name, endpoint, "produces", &endpoint.produces)?;
    if let Some(consumes) = endpoint.consumes.as_deref() {
        validate_media_type(plugin_name, endpoint, "consumes", consumes)?;
    }

    match (endpoint.consumes.as_ref(), endpoint.max_body_bytes) {
        (None, None) => {}
        (Some(_), Some(limit)) if (1..=MAX_PLUGIN_REQUEST_BODY_BYTES).contains(&limit) => {}
        (Some(_), Some(limit)) => {
            return Err(PluginCatalogError::InvalidEndpointBodyContract {
                plugin: plugin_name.to_owned(),
                endpoint_id: endpoint.id.clone(),
                reason: format!(
                    "body limit {limit} must be between 1 and {MAX_PLUGIN_REQUEST_BODY_BYTES} bytes"
                ),
            });
        }
        (Some(_), None) => {
            return Err(PluginCatalogError::InvalidEndpointBodyContract {
                plugin: plugin_name.to_owned(),
                endpoint_id: endpoint.id.clone(),
                reason: "a consumed media type requires a finite body limit".to_owned(),
            });
        }
        (None, Some(_)) => {
            return Err(PluginCatalogError::InvalidEndpointBodyContract {
                plugin: plugin_name.to_owned(),
                endpoint_id: endpoint.id.clone(),
                reason: "a body limit requires a consumed media type".to_owned(),
            });
        }
    }

    if
        matches!(endpoint.method, PluginHttpMethod::Get | PluginHttpMethod::Delete) &&
        endpoint.max_body_bytes.is_some()
    {
        return Err(PluginCatalogError::InvalidEndpointBodyContract {
            plugin: plugin_name.to_owned(),
            endpoint_id: endpoint.id.clone(),
            reason: format!("{} endpoints must not declare a request body", endpoint.method),
        });
    }

    if let Some(request_body) = endpoint.request_body.as_ref() {
        if endpoint.consumes.is_none() || endpoint.max_body_bytes.is_none() {
            return Err(PluginCatalogError::InvalidEndpointBodyContract {
                plugin: plugin_name.to_owned(),
                endpoint_id: endpoint.id.clone(),
                reason: "request-body documentation requires a consumed media type and a finite body limit".to_owned(),
            });
        }

        if
            request_body.description
                .as_ref()
                .is_some_and(|description| description.trim().is_empty())
        {
            return Err(PluginCatalogError::InvalidEndpointBodyContract {
                plugin: plugin_name.to_owned(),
                endpoint_id: endpoint.id.clone(),
                reason: "request-body description must not be empty".to_owned(),
            });
        }

        if let Some(example) = request_body.example.as_ref() {
            if example.is_null() {
                return Err(PluginCatalogError::InvalidEndpointBodyContract {
                    plugin: plugin_name.to_owned(),
                    endpoint_id: endpoint.id.clone(),
                    reason: "request-body example must not be null".to_owned(),
                });
            }

            let serialized_size = serde_json
                ::to_vec(example)
                .map(|value| value.len())
                .unwrap_or(usize::MAX);
            if serialized_size > MAX_PLUGIN_OPENAPI_EXAMPLE_BYTES {
                return Err(PluginCatalogError::InvalidEndpointBodyContract {
                    plugin: plugin_name.to_owned(),
                    endpoint_id: endpoint.id.clone(),
                    reason: format!(
                        "request-body example is {serialized_size} bytes; the maximum is {MAX_PLUGIN_OPENAPI_EXAMPLE_BYTES} bytes"
                    ),
                });
            }
        }
    }

    let mut query_parameter_names = BTreeSet::new();
    for parameter in &endpoint.query_parameters {
        validate_query_parameter(plugin_name, endpoint, parameter)?;

        let normalized_name = normalize_identifier(&parameter.name);
        if !query_parameter_names.insert(normalized_name) {
            return Err(PluginCatalogError::DuplicateEndpointQueryParameter {
                plugin: plugin_name.to_owned(),
                endpoint_id: endpoint.id.clone(),
                parameter: parameter.name.clone(),
            });
        }
    }

    Ok(())
}

fn validate_query_parameter(
    plugin_name: &str,
    endpoint: &PluginEndpointDescriptor,
    parameter: &PluginQueryParameterDescriptor
) -> Result<(), PluginCatalogError> {
    let name = parameter.name.trim();
    let valid_name =
        !name.is_empty() &&
        name == parameter.name &&
        name.bytes().all(|byte| {
            let isOk = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.');
            isOk
        });

    if !valid_name {
        return Err(PluginCatalogError::InvalidEndpointQueryParameter {
            plugin: plugin_name.to_owned(),
            endpoint_id: endpoint.id.clone(),
            parameter: parameter.name.clone(),
            reason: "name must contain only ASCII letters, digits, '-', '_' or '.'".to_owned(),
        });
    }

    Ok(())
}

fn validate_media_type(
    plugin_name: &str,
    endpoint: &PluginEndpointDescriptor,
    field: &'static str,
    value: &str
) -> Result<(), PluginCatalogError> {
    let media_type = value.split(';').next().unwrap_or_default().trim();
    let valid = media_type
        .split_once('/')
        .is_some_and(|(kind, subtype)| is_media_token(kind) && is_media_token(subtype));
    let safe = !value.contains('\r') && !value.contains('\n') && !value.trim().is_empty();

    if valid && safe {
        Ok(())
    } else {
        Err(PluginCatalogError::InvalidEndpointMediaType {
            plugin: plugin_name.to_owned(),
            endpoint_id: endpoint.id.clone(),
            field,
            value: value.to_owned(),
        })
    }
}

fn is_media_token(value: &str) -> bool {
    !value.is_empty() &&
        value
            .bytes()
            .all(|byte| {
                byte.is_ascii_alphanumeric() ||
                    matches!(byte, b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-')
            })
}

fn is_valid_identifier(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() &&
        trimmed
            .bytes()
            .all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
}

fn normalize_identifier(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub fn normalize_plugin_path(path: &str) -> Result<String, PluginCatalogError> {
    let trimmed = path.trim();
    if trimmed.is_empty() || !trimmed.starts_with('/') {
        return Err(PluginCatalogError::InvalidEndpointPath {
            path: path.to_owned(),
            reason: "path must start with '/'".to_owned(),
        });
    }

    if trimmed == "/" {
        return Err(PluginCatalogError::InvalidEndpointPath {
            path: path.to_owned(),
            reason: "the API root is reserved".to_owned(),
        });
    }

    if
        trimmed.contains('?') ||
        trimmed.contains('#') ||
        trimmed.contains('<') ||
        trimmed.contains('>') ||
        trimmed.contains("//") ||
        trimmed.contains('\\') ||
        trimmed.chars().any(char::is_whitespace) ||
        trimmed.chars().any(char::is_control)
    {
        return Err(PluginCatalogError::InvalidEndpointPath {
            path: path.to_owned(),
            reason: "query strings, fragments, dynamic segments, backslashes and whitespace are not allowed".to_owned(),
        });
    }

    if
        trimmed.split('/').any(|segment| {
            let is_traversal = segment == "." || segment == "..";
            is_traversal
        })
    {
        return Err(PluginCatalogError::InvalidEndpointPath {
            path: path.to_owned(),
            reason: "path traversal segments are not allowed".to_owned(),
        });
    }

    if !trimmed.bytes().all(is_static_path_byte) {
        return Err(PluginCatalogError::InvalidEndpointPath {
            path: path.to_owned(),
            reason: "path contains characters outside the supported static URI set".to_owned(),
        });
    }

    Ok(trimmed.trim_end_matches('/').to_ascii_lowercase())
}

fn is_static_path_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() ||
        matches!(
            byte,
            b'/' |
                b'-' |
                b'.' |
                b'_' |
                b'~' |
                b'!' |
                b'$' |
                b'&' |
                b'\'' |
                b'(' |
                b')' |
                b'*' |
                b'+' |
                b',' |
                b';' |
                b'=' |
                b':' |
                b'@'
        )
}

fn is_reserved_route(normalized_path: &str) -> bool {
    const RESERVED: &[&str] = &[
        "/login",
        "/redirect",
        "/logout",
        "/getlogout",
        "/getloginurl",
        "/getlogouturl",
        "/isauthenticated",
        "/version",
        "/persistlogfromui",
        "/openapi.json",
        "/docs",
    ];

    RESERVED.contains(&normalized_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(name: &str) -> PluginManifest {
        PluginManifest::new(name, "3.0.16", PluginKind::Domain)
    }

    #[test]
    fn rejects_removed_function_manifest_field() {
        let json =
            r#"{
            "contract_version": 7,
            "name": "LegacyPlugin",
            "version": "1.0.0",
            "kind": "Domain",
            "functions": [],
            "endpoints": []
        }"#;

        assert!(serde_json::from_str::<PluginManifest>(json).is_err());
    }

    #[test]
    fn rejects_duplicate_routes_after_path_normalization() {
        let mut first = manifest("First");
        first.endpoints.push(PluginEndpointDescriptor::get("one", "/example"));
        let mut second = manifest("Second");
        second.endpoints.push(PluginEndpointDescriptor::get("two", "/EXAMPLE/"));

        let error = PluginCatalog::from_manifests([first, second]).unwrap_err();
        assert!(matches!(error, PluginCatalogError::DuplicateRoute { .. }));
    }

    #[test]
    fn rejects_plugin_override_of_static_master_data_route() {
        let mut plugin = manifest("BadPlugin");
        plugin.endpoints.push(PluginEndpointDescriptor::get("patient", "/getPatientList"));

        let error = PluginCatalog::from_manifests([plugin]).unwrap_err();
        assert!(matches!(error, PluginCatalogError::ReservedRoute { .. }));
    }

    #[test]
    fn rejects_unbounded_plugin_request_body() {
        let mut plugin = manifest("BadPlugin");
        let mut endpoint = PluginEndpointDescriptor::post("upload", "/upload");
        endpoint.consumes = Some("application/json".to_owned());
        plugin.endpoints.push(endpoint);

        let error = PluginCatalog::from_manifests([plugin]).unwrap_err();
        assert!(matches!(error, PluginCatalogError::InvalidEndpointBodyContract { .. }));
    }

    #[test]
    fn rejects_response_header_injection_via_media_type() {
        let mut plugin = manifest("BadPlugin");
        let mut endpoint = PluginEndpointDescriptor::get("download", "/download");
        endpoint.produces = "application/json\r\nX-Test: injected".to_owned();
        plugin.endpoints.push(endpoint);

        let error = PluginCatalog::from_manifests([plugin]).unwrap_err();
        assert!(matches!(error, PluginCatalogError::InvalidEndpointMediaType { .. }));
    }

    #[test]
    fn rejects_duplicate_query_parameter_names_case_insensitively() {
        let mut plugin = manifest("BadPlugin");
        let endpoint = PluginEndpointDescriptor::get("search", "/search")
            .query_parameter(PluginQueryParameterDescriptor::required("query"))
            .query_parameter(PluginQueryParameterDescriptor::optional("QUERY"));
        plugin.endpoints.push(endpoint);

        let error = PluginCatalog::from_manifests([plugin]).unwrap_err();
        assert!(matches!(error, PluginCatalogError::DuplicateEndpointQueryParameter { .. }));
    }

    #[test]
    fn preserves_valid_query_parameter_contract() {
        let mut plugin = manifest("SearchPlugin");
        plugin.endpoints.push(
            PluginEndpointDescriptor::get("search", "/search").query_parameter(
                PluginQueryParameterDescriptor::required("query")
                    .documented("Serialized search expression")
                    .example("table=patients")
                    .multiline()
            )
        );

        let catalog = PluginCatalog::from_manifests([plugin]).unwrap();
        let parameter = &catalog.endpoints[0].endpoint.query_parameters[0];

        assert_eq!(parameter.name, "query");
        assert!(parameter.required);
        assert!(parameter.multiline);
        assert_eq!(parameter.example.as_deref(), Some("table=patients"));
    }

    #[test]
    fn preserves_valid_request_body_documentation() {
        let mut plugin = manifest("UploadPlugin");
        plugin.endpoints.push(
            PluginEndpointDescriptor::post("upload", "/upload")
                .json_body(JSON_BODY_LIMIT)
                .request_body_description("Structured upload request")
                .request_body_example(
                    serde_json::json!({
                    "items": [
                        {
                            "id": "example-id",
                            "enabled": true
                        }
                    ]
                })
                )
        );

        let catalog = PluginCatalog::from_manifests([plugin]).unwrap();
        let body = catalog.endpoints[0].endpoint.request_body.as_ref().unwrap();

        assert_eq!(body.description.as_deref(), Some("Structured upload request"));
        assert_eq!(body.example.as_ref().unwrap()["items"][0]["id"], "example-id");
    }

    #[test]
    fn rejects_request_body_example_without_body_contract() {
        let mut plugin = manifest("BadPlugin");
        plugin.endpoints.push(
            PluginEndpointDescriptor::post("upload", "/upload").request_body_example(
                serde_json::json!({ "value": true })
            )
        );

        let error = PluginCatalog::from_manifests([plugin]).unwrap_err();
        assert!(matches!(error, PluginCatalogError::InvalidEndpointBodyContract { .. }));
    }

    #[test]
    fn resolves_endpoint_owner_from_http_contract_without_name_derivation() {
        let mut first = manifest("BusinessAlpha");
        first.endpoints.push(
            PluginEndpointDescriptor::get("alpha-operation", "/getBusinessRecord")
        );

        let mut second = manifest("CompletelyDifferentName");
        second.endpoints.push(PluginEndpointDescriptor::get("beta-operation", "/getOtherRecord"));

        let catalog = PluginCatalog::from_manifests([first, second]).unwrap();

        assert_eq!(
            catalog.plugin_for_endpoint(PluginHttpMethod::Get, "/getBusinessRecord"),
            Some("BusinessAlpha")
        );
        assert_eq!(
            catalog.plugin_for_endpoint(PluginHttpMethod::Get, "/getOtherRecord"),
            Some("CompletelyDifferentName")
        );
        assert_eq!(catalog.plugin_for_endpoint(PluginHttpMethod::Post, "/getBusinessRecord"), None);
    }
}

#[cfg(test)]
mod plugin_host_abi_tests {
    use super::*;

    unsafe extern "C" fn dummy_log(_level: u8, _message: PluginAbi::AbiSlice) {}

    #[test]
    fn abi_host_services_are_reconstructed_without_rust_owned_ffi_types() {
        let directory = b"/plugins/MolgenDB".to_vec();
        let host = PluginHostServicesV2::new(
            dummy_log,
            None,
            None,
            PluginAbi::AbiSlice::from_bytes(&directory)
        );

        let services = PluginHostServices::from_abi(host).expect("host services must decode");
        assert_eq!(services.version, PLUGIN_ABI_VERSION_V2);
        assert_eq!(services.plugin_directory(), Some(PathBuf::from("/plugins/MolgenDB")));
        assert!(services.database.is_none());
    }
}
