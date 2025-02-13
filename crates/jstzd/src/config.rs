#![allow(dead_code)]
use crate::task::jstzd::JstzdConfig;
use anyhow::{Context, Result};
use octez::{
    r#async::{
        baker::{BakerBinaryPath, OctezBakerConfig, OctezBakerConfigBuilder},
        client::{OctezClientConfig, OctezClientConfigBuilder},
        node_config::{OctezNodeConfig, OctezNodeConfigBuilder},
        protocol::{BootstrapAccount, Protocol, ProtocolParameterBuilder},
    },
    unused_port,
};
use serde::Deserialize;
use tokio::io::AsyncReadExt;

const ACTIVATOR_PUBLIC_KEY: &str =
    "edpkuSLWfVU1Vq7Jg9FucPyKmma6otcMHac9zG4oU1KMHSTBpJuGQ2";

#[derive(Deserialize, Default)]
struct Config {
    server_port: Option<u16>,
    #[serde(default)]
    octez_node: OctezNodeConfigBuilder,
    #[serde(default)]
    octez_baker: OctezBakerConfigBuilder,
    octez_client: Option<OctezClientConfigBuilder>,
    #[serde(default)]
    protocol: ProtocolParameterBuilder,
}

async fn parse_config(path: &str) -> Result<Config> {
    let mut s = String::new();
    tokio::fs::File::open(path)
        .await
        .context("failed to open config file")?
        .read_to_string(&mut s)
        .await
        .context("failed to read config file")?;
    Ok(serde_json::from_str::<Config>(&s)?)
}

async fn build_config(
    config_path: &Option<String>,
) -> anyhow::Result<(u16, JstzdConfig)> {
    let mut config = match config_path {
        Some(p) => parse_config(p).await?,
        None => default_config(),
    };
    let octez_node_config = config.octez_node.build()?;
    let octez_client_config = match config.octez_client {
        Some(v) => v,
        None => OctezClientConfigBuilder::new(octez_node_config.rpc_endpoint.clone()),
    }
    .build()?;
    let baker_config = populate_baker_config(
        config.octez_baker,
        &octez_node_config,
        &octez_client_config,
    )?;

    let protocol_params = config.protocol.build()?;
    let server_port = config.server_port.unwrap_or(unused_port());
    Ok((
        server_port,
        JstzdConfig::new(
            octez_node_config,
            baker_config,
            octez_client_config,
            protocol_params,
        ),
    ))
}

fn default_config() -> Config {
    let mut config = Config::default();
    config
        .protocol
        .set_bootstrap_accounts([BootstrapAccount::new(
            // add activator to bootstrap accounts in default config so that
            // at least baker has an account to run with
            ACTIVATOR_PUBLIC_KEY,
            40_000_000_000,
        )
        .unwrap()]);
    config
}

fn populate_baker_config(
    mut config_builder: OctezBakerConfigBuilder,
    octez_node_config: &OctezNodeConfig,
    octez_client_config: &OctezClientConfig,
) -> anyhow::Result<OctezBakerConfig> {
    if config_builder.binary_path().is_none() {
        config_builder =
            config_builder.set_binary_path(BakerBinaryPath::Env(Protocol::Alpha));
    }
    if config_builder.octez_client_base_dir().is_none() {
        config_builder = config_builder
            .set_octez_client_base_dir(&octez_client_config.base_dir().to_string());
    }
    if config_builder.octez_node_endpoint().is_none() {
        config_builder =
            config_builder.set_octez_node_endpoint(&octez_node_config.rpc_endpoint);
    }
    config_builder.build()
}

#[cfg(test)]
mod tests {
    use std::{io::Read, io::Write, path::PathBuf, str::FromStr};

    use http::Uri;
    use octez::r#async::{
        baker::{BakerBinaryPath, OctezBakerConfigBuilder},
        client::OctezClientConfigBuilder,
        endpoint::Endpoint,
        node_config::{
            OctezNodeConfigBuilder, OctezNodeHistoryMode, OctezNodeRunOptionsBuilder,
        },
        protocol::{
            BootstrapAccount, BootstrapContract, BootstrapSmartRollup, Protocol,
            ProtocolConstants, ProtocolParameterBuilder, SmartRollupPvmKind,
        },
    };
    use tempfile::{tempdir, NamedTempFile};

    use super::Config;

    #[tokio::test]
    async fn parse_config() {
        let mut tmp_file = NamedTempFile::new().unwrap();
        let content = serde_json::to_string(
            &serde_json::json!({"octez_client": {"octez_node_endpoint": "localhost:8888"}}),
        )
        .unwrap();
        tmp_file.write_all(content.as_bytes()).unwrap();

        let config = super::parse_config(&tmp_file.path().to_string_lossy())
            .await
            .unwrap();
        assert_eq!(
            config.octez_client,
            Some(OctezClientConfigBuilder::new(Endpoint::localhost(8888)))
        );
    }

    #[test]
    fn deserialize_config_default() {
        let config = serde_json::from_value::<Config>(serde_json::json!({})).unwrap();
        assert_eq!(config.octez_baker, OctezBakerConfigBuilder::default());
        assert!(config.octez_client.is_none());
        assert_eq!(config.octez_node, OctezNodeConfigBuilder::default());
        assert_eq!(config.protocol, ProtocolParameterBuilder::default());
        assert!(config.server_port.is_none());
    }

    #[test]
    fn deserialize_config_octez_node() {
        let config = serde_json::from_value::<Config>(serde_json::json!({
            "octez_node": {
                "binary_path": "bin",
                "data_dir": "data_dir",
                "network": "test",
                "rpc_endpoint": "rpc.test",
                "p2p_address": "p2p.test",
                "log_file": "log_file",
                "run_options": {
                    "synchronisation_threshold": 1,
                    "network": "test",
                    "history_mode": "archive"
                }
            }
        }))
        .unwrap();
        let mut expected = OctezNodeConfigBuilder::new();
        expected
            .set_binary_path("bin")
            .set_data_dir("data_dir")
            .set_network("test")
            .set_rpc_endpoint(&Endpoint::try_from(Uri::from_static("rpc.test")).unwrap())
            .set_p2p_address(&Endpoint::try_from(Uri::from_static("p2p.test")).unwrap())
            .set_log_file("log_file")
            .set_run_options(
                &OctezNodeRunOptionsBuilder::new()
                    .set_history_mode(OctezNodeHistoryMode::Archive)
                    .set_network("test")
                    .set_synchronisation_threshold(1)
                    .build(),
            );
        assert_eq!(config.octez_node, expected);
    }

    #[test]
    fn deserialize_config_octez_client() {
        let config = serde_json::from_value::<Config>(serde_json::json!({
            "octez_client": {
                "binary_path": "bin",
                "base_dir": "base_dir",
                "disable_unsafe_disclaimer": false,
                "octez_node_endpoint": "rpc.test",
            }
        }))
        .unwrap();
        let expected = OctezClientConfigBuilder::new(
            Endpoint::try_from(Uri::from_static("rpc.test")).unwrap(),
        )
        .set_binary_path(PathBuf::from_str("bin").unwrap())
        .set_base_dir(PathBuf::from_str("base_dir").unwrap())
        .set_disable_unsafe_disclaimer(false);
        assert_eq!(config.octez_client, Some(expected));
    }

    #[test]
    fn deserialize_config_baker() {
        let config = serde_json::from_value::<Config>(serde_json::json!({
            "octez_baker": {
                "binary_path": "bin",
                "octez_client_base_dir": "base_dir",
                "octez_node_endpoint": "rpc.test",
            }
        }))
        .unwrap();
        let expected = OctezBakerConfigBuilder::new()
            .set_binary_path(BakerBinaryPath::Custom(PathBuf::from_str("bin").unwrap()))
            .set_octez_client_base_dir("base_dir")
            .set_octez_node_endpoint(
                &Endpoint::try_from(Uri::from_static("rpc.test")).unwrap(),
            );
        assert_eq!(config.octez_baker, expected);
    }

    #[test]
    fn deserialize_config_protocol() {
        let config = serde_json::from_value::<Config>(serde_json::json!({
            "protocol": {
                "protocol": "parisC",
                "constants": "sandbox",
                "bootstrap_accounts": [["edpktkhoky4f5kqm2EVwYrMBq5rY9sLYdpFgXixQDWifuBHjhuVuNN", "1"]],
                "bootstrap_contracts": [{"amount":"1", "script": "dummy-script-no-hash"}],
                "bootstrap_smart_rollups": [{
                    "address": "sr1PuFMgaRUN12rKQ3J2ae5psNtwCxPNmGNK",
                    "pvm_kind": "riscv",
                    "kernel": "dummy-kernel",
                    "parameters_ty": "dummy-params"
                }]
            }
        }))
        .unwrap();
        let mut expected = ProtocolParameterBuilder::new();
        expected
            .set_protocol(Protocol::ParisC)
            .set_constants(ProtocolConstants::Sandbox)
            .set_bootstrap_accounts([BootstrapAccount::new(
                "edpktkhoky4f5kqm2EVwYrMBq5rY9sLYdpFgXixQDWifuBHjhuVuNN",
                1,
            )
            .unwrap()])
            .set_bootstrap_contracts([BootstrapContract::new(
                serde_json::json!("dummy-script-no-hash"),
                1,
                None,
            )
            .unwrap()])
            .set_bootstrap_smart_rollups([BootstrapSmartRollup::new(
                "sr1PuFMgaRUN12rKQ3J2ae5psNtwCxPNmGNK",
                SmartRollupPvmKind::Riscv,
                "dummy-kernel",
                serde_json::json!("dummy-params"),
            )
            .unwrap()]);
        assert_eq!(config.protocol, expected);
    }

    #[test]
    fn deserialize_config_port() {
        let config =
            serde_json::from_value::<Config>(serde_json::json!({"server_port":5678}))
                .unwrap();
        assert_eq!(config.server_port, Some(5678));
    }

    #[test]
    fn populate_baker_config() {
        let tmp_dir = tempdir().unwrap();
        let node_config = OctezNodeConfigBuilder::new()
            .set_rpc_endpoint(&Endpoint::localhost(5678))
            .build()
            .unwrap();
        let client_config = OctezClientConfigBuilder::new(Endpoint::localhost(5678))
            .set_base_dir(tmp_dir.path().to_path_buf())
            .build()
            .unwrap();
        let baker_builder = OctezBakerConfigBuilder::new();
        let baker_config =
            super::populate_baker_config(baker_builder, &node_config, &client_config)
                .unwrap();
        assert_eq!(
            baker_config,
            OctezBakerConfigBuilder::new()
                .set_binary_path(BakerBinaryPath::Env(Protocol::Alpha))
                .set_octez_client_base_dir(tmp_dir.path().to_str().unwrap())
                .set_octez_node_endpoint(&Endpoint::localhost(5678))
                .build()
                .unwrap()
        );
    }

    #[test]
    fn default_config() {
        let config = super::default_config();
        let accounts = config.protocol.bootstrap_accounts();
        assert_eq!(accounts.len(), 1);
        assert_eq!(
            **accounts.first().unwrap(),
            BootstrapAccount::new(super::ACTIVATOR_PUBLIC_KEY, 40_000_000_000).unwrap()
        );
    }

    #[tokio::test]
    async fn build_config() {
        let mut tmp_file = NamedTempFile::new().unwrap();
        let content = serde_json::to_string(&serde_json::json!({
            "octez_node": {
                "rpc_endpoint": "localhost:8888",
            },
            "octez_client": {
                "octez_node_endpoint": "localhost:9999",
            },
            "protocol": {
                "bootstrap_accounts": [["edpktkhoky4f5kqm2EVwYrMBq5rY9sLYdpFgXixQDWifuBHjhuVuNN", "6000000000"]]
            }
        }))
        .unwrap();
        tmp_file.write_all(content.as_bytes()).unwrap();
        let (_, config) =
            super::build_config(&Some(tmp_file.path().to_str().unwrap().to_owned()))
                .await
                .unwrap();
        assert_eq!(
            config.octez_client_config().octez_node_endpoint(),
            &Endpoint::localhost(9999)
        );
    }

    #[tokio::test]
    async fn build_config_with_default_config() {
        let (_, config) = super::build_config(&None).await.unwrap();
        let mut buf = String::new();
        config
            .protocol_params()
            .parameter_file()
            .read_to_string(&mut buf)
            .unwrap();
        let params = serde_json::from_str::<serde_json::Value>(&buf).unwrap();

        // one bootstrap account should have been inserted: the activator account
        let accounts = params
            .as_object()
            .unwrap()
            .get("bootstrap_accounts")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(
            serde_json::from_value::<BootstrapAccount>(accounts.first().unwrap().clone())
                .unwrap(),
            BootstrapAccount::new(super::ACTIVATOR_PUBLIC_KEY, 40_000_000_000).unwrap()
        );
    }

    #[tokio::test]
    async fn build_config_without_octez_client() {
        let mut tmp_file = NamedTempFile::new().unwrap();
        let content = serde_json::to_string(&serde_json::json!({
            "octez_node": {
                "rpc_endpoint": "localhost:8888",
            },
            "protocol": {
                "bootstrap_accounts": [["edpktkhoky4f5kqm2EVwYrMBq5rY9sLYdpFgXixQDWifuBHjhuVuNN", "6000000000"]]
            }
        }))
        .unwrap();
        tmp_file.write_all(content.as_bytes()).unwrap();
        let (_, config) =
            super::build_config(&Some(tmp_file.path().to_str().unwrap().to_owned()))
                .await
                .unwrap();
        assert_eq!(
            config.octez_client_config().octez_node_endpoint(),
            &Endpoint::localhost(8888)
        );
    }
}
