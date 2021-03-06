use std::{path::Path, str::FromStr, time::Instant};

use anyhow::{bail, Result};
use cmd_lib::run_fun;
use serde::Deserialize;
use tracing::{event, Level};
use yaml_rust::{yaml::Hash, ScanError, Yaml, YamlEmitter, YamlLoader};

pub trait TestDiscovery: Send + Sync {
    fn discover_tests(&self, suite: &str) -> Vec<String>;
}

#[derive(Debug, Clone)]
pub struct ResmokeProxy {}

#[derive(Debug, Deserialize)]
struct TestDiscoveryOutput {
    pub suite_name: String,
    pub tests: Vec<String>,
}

impl TestDiscovery for ResmokeProxy {
    fn discover_tests(&self, suite: &str) -> Vec<String> {
        let start = Instant::now();
        let cmd_output = run_fun!(
            python buildscripts/resmoke.py test-discovery --suite $suite
        )
        .unwrap();
        event!(
            Level::INFO,
            suite,
            duration_ms = start.elapsed().as_millis() as u64,
            "Resmoke test discovery finished"
        );

        let output: TestDiscoveryOutput = serde_yaml::from_str(&cmd_output).unwrap();
        output
            .tests
            .into_iter()
            .filter(|f| Path::new(f).exists())
            .collect()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MultiversionConfig {
    pub last_versions: Vec<String>,
}

impl MultiversionConfig {
    pub fn from_resmoke() -> MultiversionConfig {
        let cmd_output = run_fun!(
            python buildscripts/resmoke.py multiversion-config
        )
        .unwrap();
        serde_yaml::from_str(&cmd_output).unwrap()
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum SuiteFixtureType {
    Shell,
    Repl,
    Shard,
    Other,
}

impl SuiteFixtureType {
    pub fn get_version_combinations(&self) -> Vec<String> {
        match self {
            Self::Shard => vec!["new_old_old_new".to_string()],
            Self::Repl => ["new_new_old", "new_old_new", "old_new_new"]
                .iter()
                .map(|v| v.to_string())
                .collect(),
            _ => vec!["".to_string()],
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResmokeSuiteConfig {
    config: Yaml,
}

impl FromStr for ResmokeSuiteConfig {
    type Err = ScanError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let suite_config = YamlLoader::load_from_str(s)?;
        Ok(Self {
            config: suite_config[0].to_owned(),
        })
    }
}

impl ResmokeSuiteConfig {
    pub fn read_suite_config(suite_name: &str) -> Self {
        let cmd_output = run_fun!(
            python buildscripts/resmoke.py suiteconfig --suite $suite_name
        )
        .unwrap();
        Self::from_str(&cmd_output).unwrap()
    }

    pub fn get_fixture_type(&self) -> Result<SuiteFixtureType> {
        let executor = self.get_executor()?;
        match executor {
            Yaml::Hash(executor) => {
                if let Some(fixture) = executor.get(&Yaml::from_str("fixture")) {
                    match fixture {
                        Yaml::Hash(fixture) => {
                            if let Some(fixture_class) = fixture.get(&Yaml::from_str("class")) {
                                match fixture_class {
                                    Yaml::String(fixture_class) => {
                                        if fixture_class == "ShardedClusterFixture" {
                                            Ok(SuiteFixtureType::Shard)
                                        } else if fixture_class == "ReplicaSetFixture" {
                                            Ok(SuiteFixtureType::Repl)
                                        } else {
                                            Ok(SuiteFixtureType::Other)
                                        }
                                    }
                                    _ => Ok(SuiteFixtureType::Other),
                                }
                            } else {
                                Ok(SuiteFixtureType::Other)
                            }
                        }
                        _ => Ok(SuiteFixtureType::Other),
                    }
                } else {
                    Ok(SuiteFixtureType::Shell)
                }
            }
            _ => bail!("Expected map as executor"),
        }
    }

    fn get_executor(&self) -> Result<&Yaml> {
        match &self.config {
            Yaml::Hash(map) => Ok(map.get(&Yaml::from_str("executor")).unwrap()),
            _ => bail!("Expected map at root of resmoke config"),
        }
    }

    pub fn update_config(&self, test_list: &[String], all_tests: Option<&Vec<String>>) -> String {
        let mut new_map = Hash::new();
        if let Yaml::Hash(map) = &self.config {
            for k in map.keys() {
                if k.as_str() == Some("selector") {
                    if let Yaml::Hash(selector) = map.get(k).unwrap() {
                        let mut new_selector = selector.clone();
                        if let Some(all_tests) = all_tests {
                            if let Some(excluded_files) =
                                selector.get(&Yaml::from_str("exclude_files"))
                            {
                                if let Yaml::Array(excluded_file_list) = excluded_files {
                                    let mut new_excluded_files = excluded_file_list.clone();
                                    new_excluded_files.extend(
                                        all_tests
                                            .iter()
                                            .map(|t| Yaml::from_str(t))
                                            .collect::<Vec<Yaml>>(),
                                    );
                                    new_selector.insert(
                                        Yaml::from_str("exclude_files"),
                                        Yaml::Array(new_excluded_files),
                                    );
                                }
                            } else {
                                new_selector.insert(
                                    Yaml::from_str("exclude_files"),
                                    Yaml::Array(
                                        all_tests
                                            .iter()
                                            .map(|t| Yaml::from_str(t))
                                            .collect::<Vec<Yaml>>(),
                                    ),
                                );
                            }
                        } else {
                            let exclude_key = Yaml::from_str("exclude_files");
                            if new_selector.contains_key(&exclude_key) {
                                new_selector.remove(&exclude_key);
                            }
                            new_selector.insert(
                                Yaml::from_str("roots"),
                                Yaml::Array(test_list.iter().map(|t| Yaml::from_str(t)).collect()),
                            );
                        }
                        new_map.insert(k.clone(), Yaml::Hash(new_selector));
                    }
                } else {
                    new_map.insert(k.clone(), map.get(k).unwrap().clone());
                }
            }
        }

        let mut out_str = String::new();
        let mut emitter = YamlEmitter::new(&mut out_str);
        emitter.dump(&Yaml::Hash(new_map)).unwrap();

        out_str
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // get_fixture_type tests.
    #[test]
    fn test_no_fixture_defined_should_return_shell() {
        let config_yaml = "
            test_kind: js_test

            selector:
              roots:
                - jstests/auth/*.js
              exclude_files:
                - jstests/auth/repl.js
        
            executor:
              config:
                shell_options:
                  global_vars:
                    TestData:
                      roleGraphInvalidationIsFatal: true
                  nodb: '' 
        ";

        let config = ResmokeSuiteConfig::from_str(config_yaml).unwrap();

        assert_eq!(config.get_fixture_type().unwrap(), SuiteFixtureType::Shell);
    }

    #[test]
    fn test_shared_cluster_fixture_should_return_sharded() {
        let config_yaml = "
            test_kind: js_test

            selector:
              roots:
                - jstests/auth/*.js
              exclude_files:
                - jstests/auth/repl.js
        
            executor:
              config:
                shell_options:
                  global_vars:
                    TestData:
                      roleGraphInvalidationIsFatal: true
                  nodb: '' 
              fixture:
                class: ShardedClusterFixture
                num_shards: 2
        ";

        let config = ResmokeSuiteConfig::from_str(config_yaml).unwrap();

        assert_eq!(config.get_fixture_type().unwrap(), SuiteFixtureType::Shard);
    }

    #[test]
    fn test_replica_set_fixture_should_return_repl() {
        let config_yaml = "
            test_kind: js_test

            selector:
              roots:
                - jstests/auth/*.js
              exclude_files:
                - jstests/auth/repl.js
        
            executor:
              config:
                shell_options:
                  global_vars:
                    TestData:
                      roleGraphInvalidationIsFatal: true
                  nodb: '' 
              fixture:
                class: ReplicaSetFixture
                num_nodes: 3
        ";

        let config = ResmokeSuiteConfig::from_str(config_yaml).unwrap();

        assert_eq!(config.get_fixture_type().unwrap(), SuiteFixtureType::Repl);
    }

    #[test]
    fn test_other_fixture_should_return_other() {
        let config_yaml = "
            test_kind: js_test

            selector:
              roots:
                - jstests/auth/*.js
              exclude_files:
                - jstests/auth/repl.js
        
            executor:
              config:
                shell_options:
                  global_vars:
                    TestData:
                      roleGraphInvalidationIsFatal: true
                  nodb: '' 
              fixture:
                num_nodes: 3
        ";

        let config = ResmokeSuiteConfig::from_str(config_yaml).unwrap();

        assert_eq!(config.get_fixture_type().unwrap(), SuiteFixtureType::Other);
    }
}
