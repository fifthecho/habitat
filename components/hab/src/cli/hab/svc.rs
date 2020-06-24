use super::util::{CacheKeyPath,
                  ConfigOptCacheKeyPath,
                  ConfigOptPkgIdent,
                  ConfigOptRemoteSup,
                  PkgIdent,
                  RemoteSup};
use crate::error::Result;
use configopt::{configopt_fields,
                ConfigOpt};
use habitat_core::{os::process::ShutdownTimeout,
                   package::PackageIdent,
                   service::{ServiceBind,
                             ServiceGroup},
                   ChannelIdent};
use habitat_sup_protocol::types::UpdateCondition;
use std::{convert::TryFrom,
          path::{Path,
                 PathBuf}};
use structopt::StructOpt;
use url::Url;
use walkdir::WalkDir;

const DEFAULT_SVC_CONFIG_PATH: &str = "/hab/sup/default/config/svc";

/// Commands relating to Habitat services
#[derive(ConfigOpt, StructOpt)]
#[structopt(no_version)]
#[allow(clippy::large_enum_variant)]
pub enum Svc {
    #[structopt(name = "bulkload")]
    BulkLoad(BulkLoad),
    Key(Key),
    #[structopt(no_version)]
    Load(Load),
    /// Start a loaded, but stopped, Habitat service.
    Start {
        #[structopt(flatten)]
        pkg_ident:  PkgIdent,
        #[structopt(flatten)]
        remote_sup: RemoteSup,
    },
    /// Query the status of Habitat services
    Status {
        /// A package identifier (ex: core/redis, core/busybox-static/1.42.2)
        #[structopt(name = "PKG_IDENT")]
        pkg_ident:  Option<PackageIdent>,
        #[structopt(flatten)]
        remote_sup: RemoteSup,
    },
    /// Stop a running Habitat service.
    Stop {
        #[structopt(flatten)]
        pkg_ident:        PkgIdent,
        #[structopt(flatten)]
        remote_sup:       RemoteSup,
        /// The delay in seconds after sending the shutdown signal to wait before killing the
        /// service process
        ///
        /// The default value is set in the packages plan file.
        #[structopt(name = "SHUTDOWN_TIMEOUT", long = "shutdown-timeout")]
        shutdown_timeout: Option<ShutdownTimeout>,
    },
    /// Unload a service loaded by the Habitat Supervisor. If the service is running it will
    /// additionally be stopped.
    Unload {
        #[structopt(flatten)]
        pkg_ident:        PkgIdent,
        #[structopt(flatten)]
        remote_sup:       RemoteSup,
        /// The delay in seconds after sending the shutdown signal to wait before killing the
        /// service process
        ///
        /// The default value is set in the packages plan file.
        #[structopt(name = "SHUTDOWN_TIMEOUT", long = "shutdown-timeout")]
        shutdown_timeout: Option<ShutdownTimeout>,
    },
}

#[derive(ConfigOpt, StructOpt)]
#[structopt(name = "bulkload", no_version, rename_all = "screamingsnake")]
/// Load services using the service config files from the specified paths
///
/// The service config files are in the format generated by `hab svc load --generate-config`.
/// The specified paths will be searched recursively for all files with a `.toml` extension.
/// Service config files will be patched with the default values from `/hab/sup/default/
/// config/svc.toml`.
pub struct BulkLoad {
    /// Paths to files or directories of service config files
    #[structopt(long = "svc-config-paths",
                default_value = "/hab/sup/default/config/svc")]
    pub svc_config_paths: Vec<PathBuf>,
}

#[derive(ConfigOpt, StructOpt)]
#[structopt(no_version)]
/// Commands relating to Habitat service keys
pub enum Key {
    /// Generates a Habitat service key
    Generate {
        /// Target service group service.group[@organization] (ex: redis.default or
        /// foo.default@bazcorp)
        #[structopt(name = "SERVICE_GROUP")]
        service_group:  ServiceGroup,
        /// The service organization
        #[structopt(name = "ORG")]
        org:            Option<String>,
        #[structopt(flatten)]
        cache_key_path: CacheKeyPath,
    },
}

lazy_static::lazy_static! {
    static ref CHANNEL_IDENT_DEFAULT: String = String::from(ChannelIdent::default().as_str());
    static ref GROUP_DEFAULT: String = String::from("default");
}

impl GROUP_DEFAULT {
    fn get() -> String { GROUP_DEFAULT.clone() }
}

fn health_check_interval_default() -> u64 { 30 }

#[derive(ConfigOpt, StructOpt, Deserialize, Debug)]
#[configopt(attrs(serde), derive(Clone, Debug))]
#[serde(deny_unknown_fields)]
#[structopt(no_version, rename_all = "screamingsnake")]
pub struct SharedLoad {
    /// Receive updates from the specified release channel
    #[structopt(long = "channel", default_value = &*CHANNEL_IDENT_DEFAULT)]
    #[serde(default)]
    pub channel:               ChannelIdent,
    /// Specify an alternate Builder endpoint. If not specified, the value will be taken from
    /// the HAB_BLDR_URL environment variable if defined. (default: https://bldr.habitat.sh)
    // TODO (DM): This should probably use `env` and `default_value`
    // TODO (DM): serde nested flattens do no work https://github.com/serde-rs/serde/issues/1547
    #[structopt(short = "u", long = "url")]
    pub bldr_url:              Option<Url>,
    /// The service group with shared config and topology
    #[structopt(long = "group", default_value = &*GROUP_DEFAULT)]
    #[serde(default = "GROUP_DEFAULT::get")]
    pub group:                 String,
    /// Service topology
    #[structopt(long = "topology",
            short = "t",
            possible_values = &["standalone", "leader"])]
    pub topology:              Option<habitat_sup_protocol::types::Topology>,
    /// The update strategy
    #[structopt(long = "strategy",
                short = "s",
                default_value = "none",
                possible_values = &["none", "at-once", "rolling"])]
    #[serde(default)]
    pub strategy:              habitat_sup_protocol::types::UpdateStrategy,
    /// The condition dictating when this service should update
    ///
    /// latest: Runs the latest package that can be found in the configured channel and local
    /// packages.
    ///
    /// track-channel: Always run what is at the head of a given channel. This enables service
    /// rollback where demoting a package from a channel will cause the package to rollback to
    /// an older version of the package. A ramification of enabling this condition is packages
    /// newer than the package at the head of the channel will be automatically uninstalled
    /// during a service rollback.
    #[structopt(long = "update-condition",
                default_value = UpdateCondition::Latest.as_str(),
                possible_values = UpdateCondition::VARIANTS)]
    #[serde(default)]
    pub update_condition:      UpdateCondition,
    /// One or more service groups to bind to a configuration
    #[structopt(long = "bind")]
    #[serde(default)]
    pub bind:                  Vec<ServiceBind>,
    /// Governs how the presence or absence of binds affects service startup
    ///
    /// strict: blocks startup until all binds are present.
    #[structopt(long = "binding-mode",
                default_value = "strict",
                possible_values = &["strict", "relaxed"])]
    #[serde(default)]
    pub binding_mode:          habitat_sup_protocol::types::BindingMode,
    /// The interval in seconds on which to run health checks
    // We would prefer to use `HealthCheckInterval`. However, `HealthCheckInterval` uses a map based
    // serialization format. We want to allow the user to simply specify a `u64` to be consistent
    // with the CLI, but we cannot change the serialization because the spec file depends on the map
    // based format.
    #[structopt(long = "health-check-interval", short = "i", default_value = "30")]
    #[serde(default = "health_check_interval_default")]
    pub health_check_interval: u64,
    /// The delay in seconds after sending the shutdown signal to wait before killing the service
    /// process
    ///
    /// The default value can be set in the packages plan file.
    #[structopt(long = "shutdown-timeout")]
    pub shutdown_timeout:      Option<ShutdownTimeout>,
    #[cfg(target_os = "windows")]
    /// Password of the service user
    #[structopt(long = "password")]
    pub password:              Option<String>,
    // TODO (DM): This flag can eventually be removed.
    // See https://github.com/habitat-sh/habitat/issues/7339
    /// DEPRECATED
    #[structopt(long = "application", short = "a", takes_value = false, hidden = true)]
    #[serde(skip)]
    pub application:           Vec<String>,
    // TODO (DM): This flag can eventually be removed.
    // See https://github.com/habitat-sh/habitat/issues/7339
    /// DEPRECATED
    #[structopt(long = "environment", short = "e", takes_value = false, hidden = true)]
    #[serde(skip)]
    pub environment:           Vec<String>,
    /// Use the package config from this path rather than the package itself
    #[structopt(long = "config-from")]
    pub config_from:           Option<PathBuf>,
}

#[configopt_fields]
#[derive(ConfigOpt, StructOpt, Deserialize, Debug)]
#[configopt(attrs(serde),
            derive(Clone, Debug),
            default_config_file("/hab/sup/default/config/svc.toml"))]
#[serde(deny_unknown_fields)]
#[structopt(name = "load", no_version, rename_all = "screamingsnake")]
/// Load a service to be started and supervised by Habitat from a package identifier. If an
/// installed package doesn't satisfy the given package identifier, a suitable package will be
/// installed from Builder.
pub struct Load {
    #[structopt(flatten)]
    pub pkg_ident:   PkgIdent,
    /// Load or reload an already loaded service. If the service was previously loaded and
    /// running this operation will also restart the service
    #[structopt(short = "f", long = "force")]
    #[serde(default)]
    pub force:       bool,
    #[structopt(flatten)]
    #[serde(flatten)]
    pub remote_sup:  RemoteSup,
    #[structopt(flatten)]
    #[serde(flatten)]
    pub shared_load: SharedLoad,
}

pub fn svc_loads_from_paths<T: AsRef<Path>>(paths: &[T]) -> Result<Vec<Load>> {
    // If the only path is the default location and the directory does not exist do not report an
    // error. This allows users to run the Supervisor without creating the directory.
    if paths.len() == 1 {
        let path = paths[0].as_ref();
        if path == Path::new(DEFAULT_SVC_CONFIG_PATH) && !path.exists() {
            return Ok(Vec::new());
        }
    }
    let mut svc_loads = Vec::new();
    let default_svc_load = ConfigOptLoad::from_default_config_files()?;
    for path in paths {
        for entry in WalkDir::new(path) {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type().is_file() {
                if let Some(extension) = path.extension() {
                    if extension == "toml" {
                        // Patch the service config with values from the default config file. We
                        // must use two `take` calls instead of a single patch call to ensure
                        // deserialization default values are correctly overwritten.
                        let mut configopt_svc_load = configopt::from_toml_file(path)?;
                        let mut default_svc_load = default_svc_load.clone();
                        default_svc_load.take(&mut configopt_svc_load);
                        let mut svc_load = configopt::from_toml_file(path)?;
                        default_svc_load.clone().take_for(&mut svc_load);
                        svc_loads.push(svc_load);
                    }
                }
            }
        }
    }
    Ok(svc_loads)
}

pub fn shared_load_cli_to_ctl(ident: PackageIdent,
                              shared_load: SharedLoad,
                              force: bool)
                              -> Result<habitat_sup_protocol::ctl::SvcLoad> {
    use habitat_common::{ui,
                         ui::UIWriter};
    #[cfg(target_os = "windows")]
    use habitat_core::crypto::dpapi;
    use habitat_sup_protocol::{ctl::{ServiceBindList,
                                     SvcLoad},
                               types::{HealthCheckInterval,
                                       ServiceBind}};

    // TODO (DM): This check can eventually be removed.
    // See https://github.com/habitat-sh/habitat/issues/7339
    if !shared_load.application.is_empty() || !shared_load.environment.is_empty() {
        ui::ui().warn("--application and --environment flags are deprecated and ignored.")
                .ok();
    }

    let binds = if shared_load.bind.is_empty() {
        None
    } else {
        Some(ServiceBindList { binds: shared_load.bind
                                                 .into_iter()
                                                 .map(ServiceBind::from)
                                                 .collect(), })
    };

    let config_from = if let Some(config_from) = shared_load.config_from {
        warn!("");
        warn!("WARNING: Setting '--config-from' should only be used in development, not \
               production!");
        warn!("");
        Some(config_from.to_string_lossy().to_string())
    } else {
        None
    };

    #[cfg(target_os = "windows")]
    let svc_encrypted_password = if let Some(password) = shared_load.password {
        Some(dpapi::encrypt(password)?)
    } else {
        None
    };
    #[cfg(not(target_os = "windows"))]
    let svc_encrypted_password = None;

    Ok(SvcLoad { ident: Some(ident.into()),
                 application_environment: None,
                 binds,
                 specified_binds: None,
                 binding_mode: Some(shared_load.binding_mode as i32),
                 bldr_url: Some(habitat_core::url::bldr_url(shared_load.bldr_url)),
                 bldr_channel: Some(shared_load.channel.to_string()),
                 config_from,
                 force: Some(force),
                 group: Some(shared_load.group),
                 svc_encrypted_password,
                 topology: shared_load.topology.map(i32::from),
                 update_strategy: Some(shared_load.strategy as i32),
                 health_check_interval:
                     Some(HealthCheckInterval { seconds: shared_load.health_check_interval, }),
                 shutdown_timeout: shared_load.shutdown_timeout.map(u32::from),
                 update_condition: Some(shared_load.update_condition as i32) })
}

impl TryFrom<Load> for habitat_sup_protocol::ctl::SvcLoad {
    type Error = crate::error::Error;

    fn try_from(svc_load: Load) -> Result<Self> {
        shared_load_cli_to_ctl(svc_load.pkg_ident.pkg_ident(),
                               svc_load.shared_load,
                               svc_load.force)
    }
}
