use crate::resmoke::{ResmokeProxy, TestDiscovery};
use crate::task_history::{get_test_name, TaskRuntimeHistory};
use shrub_rs::models::commands::{fn_call, fn_call_with_params, EvgCommand};
use shrub_rs::models::params::ParamValue;
use shrub_rs::models::task::{EvgTask, TaskDependency, TaskRef};
use shrub_rs::models::variant::DisplayTask;
use std::cmp::min;
use std::collections::HashMap;
use std::path::Path;

/// Parameters describing how a specific resmoke suite should be generated.
#[derive(Clone, Debug)]
pub struct ResmokeGenParams {
    pub use_large_distro: bool,
    pub large_distro_name: Option<String>,
    pub require_multiversion_setup: bool,
    // pub require_multiversion_setup_combo: bool,
    pub repeat_suites: usize,
    pub resmoke_args: String,
    pub resmoke_jobs_max: Option<u64>,
    pub config_location: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SubSuite {
    pub name: String,
    pub test_list: Vec<String>,
}

impl SubSuite {
    pub fn task_ref(&self) -> TaskRef {
        TaskRef {
            name: self.name.to_string(),
            distros: None,
            activate: Some(false),
        }
    }

    pub fn task(&self, gen_params: &ResmokeGenParams) -> EvgTask {
        EvgTask {
            name: self.name.clone(),
            commands: resmoke_commands(
                "run generated tests",
                run_test_vars(&self.name, gen_params),
                gen_params.require_multiversion_setup,
            ),
            depends_on: Some(dependencies()),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone)]
pub struct GeneratedSuite {
    pub task_name: String,
    pub suite_name: String,
    pub sub_suites: Vec<SubSuite>,
}

impl GeneratedSuite {
    pub fn task_refs(&self) -> Vec<TaskRef> {
        self.sub_suites.iter().map(|s| s.task_ref()).collect()
    }

    pub fn display_task(&self) -> DisplayTask {
        DisplayTask {
            name: self.task_name.clone(),
            execution_tasks: self.sub_suites.iter().map(|s| s.name.clone()).collect(),
        }
    }

    pub fn execution_tasks(&self, gen_params: &ResmokeGenParams) -> Vec<EvgTask> {
        self.sub_suites.iter().map(|s| s.task(gen_params)).collect()
    }
}

#[derive(Debug, Clone)]
pub struct SplitConfig {
    pub n_suites: usize,
}

#[derive(Clone)]
pub struct TaskSplitter {
    pub test_discovery: ResmokeProxy,
    pub split_config: SplitConfig,
}

impl TaskSplitter {
    pub fn split_task(&self, task_stats: &TaskRuntimeHistory, bv_name: &str) -> GeneratedSuite {
        let suite_name = &task_stats.suite_name;
        let test_list: Vec<String> = self
            .test_discovery
            .discover_tests(suite_name)
            .into_iter()
            .filter(|s| Path::new(s).exists())
            .collect();

        let total_runtime = task_stats
            .test_map
            .iter()
            .fold(0.0, |init, (_, item)| init + item.average_runtime);

        let max_tasks = min(self.split_config.n_suites, test_list.len());

        let runtime_per_subtask = total_runtime / max_tasks as f64;
        let mut sub_suites = vec![];
        let mut running_tests = vec![];
        let mut running_runtime = 0.0;
        let mut i = 0;
        for test in test_list {
            let test_name = get_test_name(&test);
            if let Some(test_stats) = task_stats.test_map.get(&test_name) {
                if (running_runtime + test_stats.average_runtime > runtime_per_subtask)
                    && !running_tests.is_empty()
                    && sub_suites.len() < max_tasks - 1
                {
                    sub_suites.push(SubSuite {
                        name: format!("{}_{}_{}", &task_stats.task_name, i, bv_name),
                        test_list: running_tests.clone(),
                    });
                    running_tests = vec![];
                    running_runtime = 0.0;
                    i += 1;
                }
                running_runtime += test_stats.average_runtime;
            }
            running_tests.push(test.clone());
        }
        if !running_tests.is_empty() {
            sub_suites.push(SubSuite {
                name: format!("{}_{}_{}", &task_stats.task_name, i, bv_name),
                test_list: running_tests.clone(),
            });
        }

        GeneratedSuite {
            task_name: task_stats.task_name.clone(),
            sub_suites,
            suite_name: suite_name.to_string(),
        }
    }
}

fn resmoke_args(origin_suite: &str, params: &ResmokeGenParams) -> String {
    format!("--originSuite={} {}", origin_suite, params.resmoke_args)
}

fn dependencies() -> Vec<TaskDependency> {
    vec![TaskDependency {
        name: String::from("archive_dist_test"),
        variant: None,
    }]
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
    run_test_vars.insert(
        String::from("suite"),
        ParamValue::from(format!("generated_resmoke_config/{}.yml", suite_file).as_str()),
    );

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
