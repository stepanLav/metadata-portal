extern crate core;

mod chains_config;
mod cleaner;
mod collector;
mod common;
mod config;
mod deployment_checker;
mod export;
mod fetch;
mod opts;
mod qrs;
mod signer;
mod source;
mod updater;
mod verifier;

use std::process::exit;

use clap::Parser;
use env_logger::Env;
use log::error;

use crate::chains_config::update_chains_config;
use crate::cleaner::clean;
use crate::collector::{collect, collect_async};
use crate::config::AppConfig;
use crate::deployment_checker::check_deployment;
use crate::fetch::RpcFetcher;
use crate::opts::{Opts, SubCommand};
use crate::signer::sign;
use crate::updater::source::UpdateSource;
use crate::updater::{update_from_github, update_from_node, update_from_node_async};
use crate::verifier::verify;

/// Main entry point of the `metadata-cli`
fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format_target(false)
        .init();

    let opts: Opts = Opts::parse();
    let config = match AppConfig::load(&opts.config) {
        Ok(config) => config,
        Err(err) => {
            error!("{}", err);
            exit(1);
        }
    };

    // Check for ASYNC_MODE environment variable to enable parallel processing
    // Default to async mode for better performance
    let use_async = std::env::var("ASYNC_MODE")
        .unwrap_or_else(|_| "true".to_string())
        .parse::<bool>()
        .unwrap_or(true);

    let result = match opts.subcmd {
        SubCommand::Clean => clean(config),
        SubCommand::Collect => {
            if use_async {
                collect_async(config)
            } else {
                collect(config)
            }
        }
        SubCommand::Sign => sign(config),
        SubCommand::Verify => verify(config),
        SubCommand::Update(update_opts) => match update_opts.source {
            UpdateSource::Github => {
                update_from_github(config, update_opts.sign, update_opts.signing_key)
            }
            UpdateSource::Node => {
                if use_async {
                    update_from_node_async(
                        config,
                        update_opts.sign,
                        update_opts.signing_key,
                    )
                } else {
                    update_from_node(
                        config,
                        update_opts.sign,
                        update_opts.signing_key,
                        RpcFetcher,
                    )
                }
            }
        },
        SubCommand::CheckDeployment => check_deployment(config),
        SubCommand::UpdateChainConfig(chains_opts) => update_chains_config(chains_opts),
    };

    if let Err(err) = result {
        error!("{}", err);
        exit(1);
    }
}
