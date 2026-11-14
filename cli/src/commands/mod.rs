pub mod app;
pub mod auth;
pub mod branch;
pub mod backend;
pub mod benchmark;
pub mod cache;
pub mod contract;
pub mod contract_build;
pub mod database;
pub mod database_capture;
pub mod database_convert;
pub mod database_dev;
pub mod database_fleet;
pub mod database_impact;
pub mod database_remote;
pub mod database_runtime;
pub mod database_sqlite;
pub mod dependencies;
pub mod deploy;
pub mod diff;
pub mod doctor;
pub mod domain;
pub mod editor;
pub mod editor_workflows;
pub mod extensions;
pub mod init;
pub mod inspector;
mod inspector_dashboard;
mod inspector_frontend;
mod inspector_jev;
mod inspector_runtime;
mod inspector_server;
pub mod install;
pub mod join;
mod legacy_mock;
pub mod login;
pub mod login_plain;
pub mod lsp;
pub mod mock;
mod mock_development;
pub mod onboard;
pub mod packages;
pub mod project; // Add this
pub mod pull;
pub mod release;
pub mod run;
pub mod scenario;
pub mod secrets;
mod secrets_cloud;
pub mod security;
pub mod serve;
pub mod server;
#[allow(dead_code)] // Standalone mock CLI types remain reserved for their execution phase.
pub(crate) mod server_command_contract;
mod server_development;
mod server_builder;
mod server_workflow;
pub mod test;
pub mod ui;
pub mod watch;

#[cfg(test)]
mod editor_schema_parity;

pub mod secret_development;

pub mod resources;
pub mod resources_access;
pub mod resources_placement;
pub mod resources_recovery;
#[cfg(test)]
pub(crate) mod cloud_mock;
pub mod step_up;
pub mod cloud_cli;
pub mod contract_releases;
pub mod sandbox_environments;
pub mod workspace;
pub mod signers;

pub mod data_metadata;

pub mod data;
pub mod intel;
pub mod intel_routes;
