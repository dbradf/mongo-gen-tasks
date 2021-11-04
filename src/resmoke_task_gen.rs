use shrub_rs::models::commands::{fn_call, fn_call_with_params, EvgCommand};
use shrub_rs::models::params::ParamValue;
use shrub_rs::models::task::{EvgTask, TaskDependency};
use std::collections::HashMap;

use crate::split_tasks::GeneratedSuite;

pub struct GenerateOptions {
    pub create_misc_suite: bool,
    pub is_patch: bool,
    pub generated_config_dir: String,
    pub use_default_timeouts: bool,
}

impl GenerateOptions {
    pub fn suite_location<'a>(&self, f: &'a str) -> &'a str {
        f
    }
}

pub struct ResmokeGenParams {
    pub use_large_distro: bool,
    pub large_distro_name: Option<String>,
    pub require_multiversion_setup: bool,
    pub repeat_suites: usize,
    pub resmoke_args: String,
    pub resmoke_jobs_max: Option<u64>,
    pub config_location: Option<String>,
}

pub struct ResmokeGenService {
    // options: GenerateOptions,
}

impl ResmokeGenService {
    pub fn generate_tasks(
        &self,
        generated_suite: &GeneratedSuite,
        params: &ResmokeGenParams,
    ) -> Vec<EvgTask> {
        let tasks: Vec<EvgTask> = generated_suite
            .sub_suites
            .iter()
            .map(|s| self.create_sub_task(&s.name, params))
            .collect();

        tasks
    }

    fn create_sub_task(&self, sub_suite_file: &str, params: &ResmokeGenParams) -> EvgTask {
        EvgTask {
            name: sub_suite_file.to_string(),
            commands: resmoke_commands(
                "run generated tests",
                run_test_vars(sub_suite_file, params),
                params.require_multiversion_setup,
            ),
            depends_on: Some(dependencies()),
            ..Default::default()
        }
    }
}

fn run_test_vars(suite_file: &str, params: &ResmokeGenParams) -> HashMap<String, ParamValue> {
    let mut run_test_vars = HashMap::new();
    let resmoke_args = resmoke_args(suite_file, params);

    run_test_vars.insert(
        String::from("require_multiversion_setup"),
        ParamValue::from(params.require_multiversion_setup),
    );
    run_test_vars.insert(
        String::from("resmoke_args"),
        ParamValue::from(resmoke_args.as_str()),
    );
    run_test_vars.insert(String::from("suite"), ParamValue::from(suite_file));

    if let Some(config_location) = &params.config_location {
        run_test_vars.insert(
            String::from("gen_task_config_location"),
            ParamValue::from(config_location.as_str()),
        );
    }

    if let Some(resmoke_jobs_max) = &params.resmoke_jobs_max {
        run_test_vars.insert(
            String::from("resmoke_jobs_max"),
            ParamValue::from(*resmoke_jobs_max),
        );
    }

    run_test_vars
}

fn resmoke_args(origin_suite: &str, params: &ResmokeGenParams) -> String {
    format!("--originSuite={} {}", origin_suite, params.resmoke_args)
}

fn resmoke_commands(
    run_test_fn_name: &str,
    run_test_vars: HashMap<String, ParamValue>,
    requires_multiversion_setup: bool,
) -> Vec<EvgCommand> {
    let mut commands = vec![];

    if requires_multiversion_setup {
        commands.push(fn_call("git get project no modules"));
        commands.push(fn_call("add git tag"));
    }

    commands.push(fn_call("do setup"));
    commands.push(fn_call("configure evergreen api credentials"));

    if requires_multiversion_setup {
        commands.push(fn_call("do multiversion setup"));
    }

    commands.push(fn_call_with_params(run_test_fn_name, run_test_vars));
    commands
}

fn dependencies() -> Vec<TaskDependency> {
    vec![TaskDependency {
        name: String::from("activate_archive_dist_test_debug"),
        variant: None,
    }]
}