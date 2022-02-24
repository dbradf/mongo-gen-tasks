use std::collections::HashMap;

use shrub_rs::models::{
    commands::{fn_call, fn_call_with_params},
    params::ParamValue,
    task::{EvgTask, TaskDependency, TaskRef},
    variant::DisplayTask,
};
use tracing::{event, Level};

use crate::{resmoke::ResmokeSuiteConfig, util::name_generated_task};

#[derive(Debug)]
pub struct FuzzerTask {
    pub task_name: String,
    pub sub_tasks: Vec<EvgTask>,
}

impl FuzzerTask {
    pub fn build_display_task(&self) -> DisplayTask {
        DisplayTask {
            name: self.task_name.clone(),
            execution_tasks: self.sub_tasks.iter().map(|s| s.name.to_string()).collect(),
        }
    }

    pub fn build_task_ref(&self) -> Vec<TaskRef> {
        self.sub_tasks
            .iter()
            .map(|s| s.get_reference(None, Some(false)))
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct FuzzerGenTaskParams {
    /// Name of task being generated.
    pub task_name: String,
    /// Name of build variant being generated on.
    pub variant: String,
    /// Resmoke suite for generated tests.
    pub suite: String,
    /// Number of javascript files fuzzer should generate.
    pub num_files: u64,
    /// Number of sub-tasks fuzzer should generate.
    pub num_tasks: u64,
    /// Arguments to pass to resmoke invocation.
    pub resmoke_args: String,
    /// NPM command to perform fuzzer execution.
    pub npm_command: String,
    /// Arguments to pass to fuzzer invocation.
    pub jstestfuzz_vars: Option<String>,
    /// Should generated tests continue running after hitting error.
    pub continue_on_failure: bool,
    /// Maximum number of jobs resmoke should execute in parallel.
    pub resmoke_jobs_max: u64,
    /// Should tests be executed out of order.
    pub should_shuffle: bool,
    /// Timeout before test execution is considered hung.
    pub timeout_secs: u64,
    /// Requires downloading multiverion binaries.
    pub require_multiversion_setup: Option<bool>,
    /// Should tests be generated on a large distro.
    pub use_large_distro: Option<bool>,
    /// Name of large distro to generate.
    pub large_distro_name: Option<String>,
    /// Location of generated task configuration.
    pub config_location: String,
    pub suite_config: ResmokeSuiteConfig,
}

impl FuzzerGenTaskParams {
    fn build_jstestfuzz_vars(&self) -> HashMap<String, ParamValue> {
        let mut vars = HashMap::new();
        vars.insert(
            "npm_command".to_string(),
            ParamValue::from(self.npm_command.as_str()),
        );
        vars.insert(
            "jstestfuzz_vars".to_string(),
            ParamValue::String(format!(
                "--numGeneratedFiles {} {}",
                self.num_files,
                self.jstestfuzz_vars.clone().unwrap_or_default()
            )),
        );
        vars
    }

    fn build_run_tests_vars(
        &self,
        suite_name: Option<&str>,
        bin_version: Option<&str>,
    ) -> HashMap<String, ParamValue> {
        let mut vars = HashMap::new();
        vars.insert(
            "continue_on_failure".to_string(),
            ParamValue::from(self.continue_on_failure),
        );
        vars.insert(
            "resmoke_args".to_string(),
            ParamValue::from(self.resmoke_args.as_str()),
        );
        vars.insert(
            "resmoke_jobs_max".to_string(),
            ParamValue::from(self.resmoke_jobs_max),
        );
        vars.insert(
            "should_shuffle".to_string(),
            ParamValue::from(self.should_shuffle),
        );
        vars.insert(
            "require_multiversion_setup".to_string(),
            ParamValue::from(self.require_multiversion_setup.unwrap_or(false)),
        );
        vars.insert(
            "timeout_secs".to_string(),
            ParamValue::from(self.timeout_secs),
        );
        vars.insert(
            "task".to_string(),
            ParamValue::from(self.task_name.as_str()),
        );
        vars.insert(
            "gen_task_config_location".to_string(),
            ParamValue::from(self.config_location.as_str()),
        );
        if let Some(suite) = suite_name {
            vars.insert("suite".to_string(), ParamValue::from(suite));
        } else {
            vars.insert("suite".to_string(), ParamValue::from(self.suite.as_str()));
        }

        if let Some(bin_version) = bin_version {
            vars.insert(
                "multiversion_exclude_tags_version".to_string(),
                ParamValue::from(bin_version),
            );
        }

        vars
    }

    pub fn get_version_combination(&self) -> Vec<String> {
        self.suite_config
            .get_fixture_type()
            .unwrap()
            .get_version_combinations()
    }
}

pub trait GenFuzzerService: Sync + Send {
    fn generate_fuzzer_task(&self, params: &FuzzerGenTaskParams) -> FuzzerTask;
}

#[derive(Debug, Clone)]
pub struct GenFuzzerServiceImpl {
    last_versions: Vec<String>,
}

impl GenFuzzerServiceImpl {
    pub fn new(last_versions: &[String]) -> Self {
        Self {
            last_versions: last_versions.to_owned(),
        }
    }

    fn build_name(base_name: &str, old_version: &str, mixed_bin_version: &str) -> String {
        [base_name, old_version, mixed_bin_version]
            .iter()
            .filter_map(|p| {
                if !p.is_empty() {
                    Some(p.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<String>>()
            .join("_")
    }
}

impl GenFuzzerService for GenFuzzerServiceImpl {
    fn generate_fuzzer_task(&self, params: &FuzzerGenTaskParams) -> FuzzerTask {
        let task_name = &params.task_name;
        let mut sub_tasks: Vec<EvgTask> = vec![];
        if params.require_multiversion_setup.unwrap_or(false) {
            let version_combinations = &params.get_version_combination();
            event!(
                Level::INFO,
                task_name = task_name.as_str(),
                "Generating multiversion fuzzer"
            );
            for version in &self.last_versions {
                for mixed_bin_version in version_combinations {
                    let base_task_name =
                        Self::build_name(&params.task_name, version, mixed_bin_version);
                    let base_suite_name =
                        Self::build_name(&params.suite, version, mixed_bin_version);

                    sub_tasks.extend(
                        (0..params.num_tasks)
                            .map(|i| {
                                build_fuzzer_sub_task(
                                    &base_task_name,
                                    i,
                                    params,
                                    Some(&base_suite_name),
                                    Some(mixed_bin_version),
                                )
                            })
                            .collect::<Vec<EvgTask>>(),
                    );
                }
            }
        } else {
            sub_tasks = (0..params.num_tasks)
                .map(|i| build_fuzzer_sub_task(&params.task_name, i, params, None, None))
                .collect();
        }

        FuzzerTask {
            task_name: params.task_name.to_string(),
            sub_tasks,
        }
    }
}

fn build_fuzzer_sub_task(
    task_name: &str,
    task_index: u64,
    params: &FuzzerGenTaskParams,
    suite_name: Option<&str>,
    bin_version: Option<&str>,
) -> EvgTask {
    let sub_task_name = name_generated_task(
        task_name,
        Some(task_index),
        Some(params.num_tasks),
        Some(&params.variant),
    );
    let mut commands = vec![];
    if params.require_multiversion_setup.unwrap_or(false) {
        commands.extend(vec![
            fn_call("git get project no modules"),
            fn_call("add git tag"),
        ]);
    }
    commands.extend(vec![
        fn_call("do setup"),
        fn_call("configure evergreen api credentials"),
    ]);

    if params.require_multiversion_setup.unwrap_or(false) {
        commands.push(fn_call("do multiversion setup"));
    }

    commands.extend(vec![
        fn_call("setup jstestfuzz"),
        fn_call_with_params("run jstestfuzz", params.build_jstestfuzz_vars()),
        fn_call_with_params(
            "run generated tests",
            params.build_run_tests_vars(suite_name, bin_version),
        ),
    ]);

    EvgTask {
        name: sub_task_name,
        commands,
        depends_on: Some(vec![TaskDependency {
            name: "archive_dist_test_debug".to_string(),
            variant: None,
        }]),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;

    // build_name
    #[rstest]
    #[case(
        "agg_fuzzer",
        "last_lts",
        "new_old_new",
        "agg_fuzzer_last_lts_new_old_new"
    )]
    #[case("agg_fuzzer", "last_lts", "", "agg_fuzzer_last_lts")]
    fn test_build_name(
        #[case] base_name: &str,
        #[case] version: &str,
        #[case] bin_version: &str,
        #[case] expected: &str,
    ) {
        let name = GenFuzzerServiceImpl::build_name(base_name, version, bin_version);

        assert_eq!(name, expected);
    }
}
