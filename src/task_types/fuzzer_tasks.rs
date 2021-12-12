use std::collections::HashMap;

use shrub_rs::models::{
    commands::{fn_call, fn_call_with_params},
    params::ParamValue,
    task::{EvgTask, TaskDependency, TaskRef},
    variant::DisplayTask,
};

use crate::util::name_generated_task;

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
                self.jstestfuzz_vars.clone().unwrap_or("".to_string())
            )),
        );
        vars
    }

    fn build_run_tests_vars(&self) -> HashMap<String, ParamValue> {
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
        vars.insert("suite".to_string(), ParamValue::from(self.suite.as_str()));

        vars
    }
}

pub fn generate_fuzzer_task(params: &FuzzerGenTaskParams) -> FuzzerTask {
    let sub_tasks: Vec<EvgTask> = (0..params.num_tasks)
        .map(|i| build_fuzzer_sub_task(i, params))
        .collect();

    FuzzerTask {
        task_name: params.task_name.to_string(),
        sub_tasks,
    }
}

fn build_fuzzer_sub_task(task_index: u64, params: &FuzzerGenTaskParams) -> EvgTask {
    let sub_task_name = name_generated_task(
        &params.task_name,
        Some(task_index),
        Some(params.num_tasks),
        Some(&params.variant),
    );
    let commands = vec![
        fn_call("do setup"),
        fn_call("configure evergreen api credentials"),
        fn_call("setup jstestfuzz"),
        fn_call_with_params("run jstestfuzz", params.build_jstestfuzz_vars()),
        fn_call_with_params("run generated tests", params.build_run_tests_vars()),
    ];

    EvgTask {
        name: sub_task_name.to_string(),
        commands,
        depends_on: Some(vec![TaskDependency {
            name: "archive_dist_test_debug".to_string(),
            variant: None,
        }]),
        ..Default::default()
    }
}
