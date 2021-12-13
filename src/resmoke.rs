use cmd_lib::run_fun;
use yaml_rust::{yaml::Hash, Yaml, YamlEmitter, YamlLoader};

pub trait TestDiscovery {
    fn discover_tests(&self, suite: &str) -> Vec<String>;
}

pub struct ResmokeProxy {}

impl TestDiscovery for ResmokeProxy {
    fn discover_tests(&self, suite: &str) -> Vec<String> {
        let cmd_output = run_fun!(
            python buildscripts/resmoke.py discover --suite $suite
        )
        .unwrap();
        cmd_output.split("\n").map(|s| s.to_string()).collect()
    }
}

pub fn update_config(
    config: &Yaml,
    test_list: &Vec<String>,
    all_tests: Option<&Vec<String>>,
) -> String {
    let mut new_map = Hash::new();
    match config {
        Yaml::Hash(map) => {
            for k in map.keys() {
                if k.as_str() == Some("selector") {
                    if let Yaml::Hash(selector) = map.get(k).unwrap() {
                        let mut new_selector = selector.clone();
                        if let Some(all_tests) = all_tests {
                            if let Some(excluded_files) =
                                selector.get(&Yaml::from_str("exclude_files"))
                            {
                                if let Yaml::Array(excluded_file_list) = excluded_files {
                                    let mut new_excluced_files = excluded_file_list.clone();
                                    new_excluced_files.extend(
                                        all_tests
                                            .iter()
                                            .map(|t| Yaml::from_str(t))
                                            .collect::<Vec<Yaml>>(),
                                    );
                                    new_selector.insert(
                                        Yaml::from_str("exclude_files"),
                                        Yaml::Array(new_excluced_files),
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
        _ => (),
    }

    let mut out_str = String::new();
    let mut emitter = YamlEmitter::new(&mut out_str);
    emitter.dump(&Yaml::Hash(new_map)).unwrap();

    out_str
}

pub fn generate_test_config(
    suite_name: &str,
    test_list: &Vec<String>,
    all_tests: Option<&Vec<String>>,
) -> String {
    let config = read_suite_config(suite_name);
    let mut new_map = Hash::new();
    match config {
        Yaml::Hash(map) => {
            for k in map.keys() {
                if k.as_str() == Some("selector") {
                    if let Yaml::Hash(selector) = map.get(k).unwrap() {
                        let mut new_selector = selector.clone();
                        if let Some(all_tests) = all_tests {
                            if let Some(excluded_files) =
                                selector.get(&Yaml::from_str("exclude_files"))
                            {
                                if let Yaml::Array(excluded_file_list) = excluded_files {
                                    let mut new_excluced_files = excluded_file_list.clone();
                                    new_excluced_files.extend(
                                        all_tests
                                            .iter()
                                            .map(|t| Yaml::from_str(t))
                                            .collect::<Vec<Yaml>>(),
                                    );
                                    new_selector.insert(
                                        Yaml::from_str("exclude_files"),
                                        Yaml::Array(new_excluced_files),
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
        _ => (),
    }

    let mut out_str = String::new();
    let mut emitter = YamlEmitter::new(&mut out_str);
    emitter.dump(&Yaml::Hash(new_map)).unwrap();

    out_str
}

pub fn read_suite_config(suite_name: &str) -> Yaml {
    let cmd_output = run_fun!(
        python buildscripts/resmoke.py suiteconfig --suite $suite_name
    )
    .unwrap();
    let suite_config = YamlLoader::load_from_str(&cmd_output).unwrap();
    suite_config[0].clone()
}
