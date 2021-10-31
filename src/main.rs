use rayon::prelude::*;
use chrono::{Date, Duration, Utc};
use evg_api_rs::models::stats::EvgTestStatsRequest;
use evg_api_rs::EvgClient;
use futures::future::join_all;
use maplit::hashmap;
use mongo_task_gen::resmoke::{ResmokeProxy, TestDiscovery};
use mongo_task_gen::split_tasks::{SplitConfig, TaskSplitter};
use mongo_task_gen::task_history::get_task_history;
use mongo_task_gen::taskname::{name_sub_suite, remove_gen_suffix, remove_gen_suffix_ref};
use shrub_rs::models::builtin::{
    BuiltInCommand, EvgCommandSpec, SubprocessExecParams, SubprocessExecutionConfig,
};
use shrub_rs::models::commands::EvgCommand::Function;
use shrub_rs::models::project::FunctionDefinition;
use shrub_rs::models::{
    commands::{fn_call_with_params, EvgCommand},
    params::ParamValue,
    project::EvgProject,
    task::{EvgTask, TaskDependency},
};
use std::{collections::HashMap, error::Error, process::Command};
use shrub_rs::models::commands::FunctionCall;

fn get_project_config(location: &str) -> Result<EvgProject, Box<dyn Error>> {
    let evg_config_yaml = Command::new("evergreen")
        .args(&["evaluate", location])
        .output()?;
    EvgProject::from_yaml_str(std::str::from_utf8(&evg_config_yaml.stdout)?)
}

#[derive(Debug)]
struct EvgExpansions {
    build_variant: String,
    task_name: String,
}

fn is_task_generated(task: &EvgTask) -> bool {
    task.commands.iter().any(|c| {
        if let Function(func) = c {
            if func.func == "generate resmoke tasks" {
                return true;
            }
        }
        false
    })
}

fn get_generate_resmoke_func(task: &EvgTask) -> Option<&FunctionCall> {
    let command = task
        .commands
        .iter()
        .filter(|c| {
            if let Function(func) = c {
                if func.func == "generate resmoke tasks" {
                    return true;
                }
            }
            false
        })
        .nth(0).unwrap();
    if let Function(func) = command {
        return Some(func);
    }
    None
}

fn get_gen_task_var<'a>(task: &'a EvgTask, var: &str) -> Option<&'a str> {
    let generate_func = get_generate_resmoke_func(task);
    if let Some(func) = generate_func {
        if let Some(vars) = &func.vars {
            if let Some(value) = vars.get(var) {
                match value {
                    ParamValue::String(value) => return Some(value),
                    _ => (),
                }
            }
        }
    }
    None
}

fn find_suite_name(task: &EvgTask) -> &str {
    let suite = get_gen_task_var(task, "suite");
    if let Some(suite) = suite {
        suite
    } else {
        remove_gen_suffix_ref(&task.name)
    }
}

fn is_fuzzer_task(task: &EvgTask) -> bool {
    let is_jstestfuzz = get_gen_task_var(task, "is_jstestfuzz");
    if let Some(is_jstestfuzz) = is_jstestfuzz {
        is_jstestfuzz == "true"
    } else {
        false
    }
}

#[tokio::main]
async fn main() {
    let evg_project_location = std::env::args().nth(1).expect("Expected project config");
    let evg_project = get_project_config(&evg_project_location).unwrap();
    let evg_client = EvgClient::new().unwrap();
    let build_variant = "enterprise-rhel-80-64-bit-dynamic-required";

    let mut history_futures = vec![];
    for task in &evg_project.tasks {
        if is_task_generated(task) && !is_fuzzer_task(task) {
            let task_name = remove_gen_suffix_ref(&task.name);
            let suite_name = find_suite_name(task);
            history_futures.push(get_task_history(
                &evg_client,
                &task_name,
                build_variant,
                &suite_name,
            ))
        }
    }

    let task_histories = join_all(history_futures).await;

    let test_discovery = ResmokeProxy {};
    let task_splitter = TaskSplitter {
        test_discovery: Box::new(test_discovery),
        split_config: SplitConfig { n_suites: 5 },
    };

    task_histories.par_iter().for_each(|task_history| {
        let test_discovery = ResmokeProxy {};
        let task_splitter = TaskSplitter {
            test_discovery: Box::new(test_discovery),
            split_config: SplitConfig { n_suites: 5 },
        };
        let gen_suite = task_splitter.split_task(task_history);
        println!("{:?}", gen_suite);

    });
    // for task_history in task_histories {
    //     let gen_suite = task_splitter.split_task(task_history);
    //     println!("{:?}", gen_suite);
    // }

    // let test_list = test_discovery.discover_tests("core");
    // for test in test_list {
    //     println!("- {}", test);
    // }

    // let mut task_map = HashMap::with_capacity(evg_project.tasks.len());
    // for task in &evg_project.tasks {
    //     task_map.insert(&task.name, task);
    // }
    //
    // let mut gen_tasks = 0;
    // let mut gen_resmoke = 0;
    // let mut gen_fuzzer = 0;
    // let mut other_gen = vec![];
    // let mut gen_task_list = vec![];
    // for build_variant in &evg_project.buildvariants {
    //     println!("{}", build_variant.name);
    //     for task in &build_variant.tasks {
    //         if task.name.ends_with("_gen") {
    //             if let Some(task_def) = task_map.get(&task.name) {
    //                 for cmd in &task_def.commands {
    //                     if let EvgCommand::Function(func) = cmd {
    //                         if func.func == "generate resmoke tasks" {
    //                             gen_resmoke += 1;
    //                         } else if func.func == "generate fuzzer tasks" {
    //                             if let Some(vars) = &func.vars {
    //                                 let params = FuzzerTaskParams::from_task_def_vars(
    //                                     &task.name,
    //                                     "location",
    //                                     &build_variant.name,
    //                                     vars,
    //                                 );
    //                                 gen_task_list.push(FuzzerTask::generate(&params));
    //                             }
    //                             gen_fuzzer += 1;
    //                         } else {
    //                             other_gen.push(func.func.to_string());
    //                         }
    //                     }
    //                 }
    //             }
    //             gen_tasks += 1;
    //             println!(" - {}", task.name);
    //         }
    //     }
    // }
    //
    // println!("# of tasks to generate: {}", gen_tasks);
    // println!("# resmoke tasks: {}", gen_resmoke);
    // println!("# of fuzzer tasks: {}", gen_fuzzer);
    // println!("Other gens: {}", other_gen.join(", "));
    // gen_task_list.iter().for_each(|t| {
    //     println!("===== {} =====", t.name);
    //     t.sub_tasks.iter().for_each(|s| {
    //         println!("{}", serde_yaml::to_string(s).unwrap());
    //     });
    // });
}

//
// struct FuzzerTaskParams {
//     pub task_name: String,
//     pub gen_task_config_location: String,
//     pub build_variant: String,
//
//     pub suite: ParamValue,
//     pub num_files: ParamValue,
//     pub num_tasks: ParamValue,
//     pub npm_command: ParamValue,
//     pub jstestfuzz_vars: Option<ParamValue>,
//     pub resmoke_args: ParamValue,
//     pub resmoke_jobs_max: Option<ParamValue>,
//     pub should_shuffle: Option<ParamValue>,
//     pub continue_on_failure: Option<ParamValue>,
//     pub task_path_suffix: Option<ParamValue>,
//     pub timeout_secs: Option<ParamValue>,
// }
//
// impl FuzzerTaskParams {
//     pub fn from_task_def_vars(
//         task_name: &str,
//         config_location: &str,
//         build_variant: &str,
//         vars: &HashMap<String, ParamValue>,
//     ) -> Self {
//         Self {
//             task_name: task_name.to_string(),
//             gen_task_config_location: config_location.to_string(),
//             build_variant: build_variant.to_string(),
//
//             suite: vars.get("suite").unwrap().clone(),
//             num_files: vars.get("num_files").unwrap().clone(),
//             num_tasks: vars.get("num_tasks").unwrap().clone(),
//             npm_command: vars
//                 .get("npm_command")
//                 .unwrap_or(&ParamValue::from("jstestfuzz"))
//                 .clone(),
//             jstestfuzz_vars: vars.get("jstestfuzz_vars").map(|v| v.clone()),
//             resmoke_args: vars.get("resmoke_args").unwrap().clone(),
//             resmoke_jobs_max: vars.get("resmoke_jobs_max").map(|v| v.clone()),
//             should_shuffle: vars.get("should_shuffle").map(|v| v.clone()),
//             continue_on_failure: vars.get("continue_on_failure").map(|v| v.clone()),
//             task_path_suffix: vars.get("task_pah_suffix").map(|v| v.clone()),
//             timeout_secs: vars.get("timeout_secs").map(|v| v.clone()),
//         }
//     }
//
//     fn build_run_test_vars(&self) -> HashMap<String, ParamValue> {
//         let resmoke_args = format!("--suites={} {}", self.suite, self.resmoke_args);
//
//         let mut map = hashmap! {
//             String::from("task") => ParamValue::from(self.task_name.as_str()),
//             String::from("resmoke_args") => ParamValue::String(resmoke_args),
//             String::from("gen_task_config_location") => ParamValue::from(self.gen_task_config_location.as_str()),
//         };
//
//         if let Some(task_path_suffix) = &self.task_path_suffix {
//             map.insert(String::from("task_path_suffix"), task_path_suffix.clone());
//         }
//         if let Some(continue_on_failure) = &self.continue_on_failure {
//             map.insert(
//                 String::from("continue_on_failure"),
//                 continue_on_failure.clone(),
//             );
//         }
//         if let Some(resmoke_jobs_max) = &self.resmoke_jobs_max {
//             map.insert(String::from("resmoke_jobs_max"), resmoke_jobs_max.clone());
//         }
//         if let Some(should_shuffle) = &self.should_shuffle {
//             map.insert(String::from("should_shuffle"), should_shuffle.clone());
//         }
//         if let Some(timeout_secs) = &self.timeout_secs {
//             map.insert(String::from("timeout_secs"), timeout_secs.clone());
//         }
//
//         map
//     }
//
//     fn build_jstestfuzz_params(&self) -> HashMap<String, ParamValue> {
//         let jstestfuzz_vars = if let Some(jstestfuzz) = &self.jstestfuzz_vars {
//             format!("--numGeneratedFiles {} {}", self.num_files, jstestfuzz)
//         } else {
//             format!("--numGeneratedFiles {}", self.num_files)
//         };
//         hashmap! {
//             String::from("jstestfuzz_vars") => ParamValue::from(jstestfuzz_vars.as_str()),
//             String::from("npm_command") => self.npm_command.clone(),
//         }
//     }
// }
//
// struct FuzzerTask {
//     pub name: String,
//     pub sub_tasks: Vec<EvgTask>,
// }
//
// fn extract_num(param: &ParamValue) -> u64 {
//     match param {
//         ParamValue::Number(n) => *n,
//         _ => 1,
//     }
// }
//
// impl FuzzerTask {
//     pub fn generate(params: &FuzzerTaskParams) -> Self {
//         let num_tasks = extract_num(&params.num_tasks);
//         let sub_tasks: Vec<EvgTask> = (0..num_tasks)
//             .into_iter()
//             .map(|i| FuzzerTask::generate_sub_task(params, i))
//             .collect();
//
//         Self {
//             name: String::from(&params.task_name),
//             sub_tasks,
//         }
//     }
//
//     pub fn generate_sub_task(params: &FuzzerTaskParams, task_index: u64) -> EvgTask {
//         let num_tasks = extract_num(&params.num_tasks);
//         let sub_task_name = name_sub_suite(
//             &params.task_name,
//             task_index,
//             num_tasks,
//             Some(&params.build_variant),
//         );
//
//         let mut commands = vec![EvgCommand::from("do setup")];
//         if params.task_path_suffix.is_some() {
//             commands.append(&mut vec![
//                 EvgCommand::from("configure evergreen api credentials"),
//                 EvgCommand::from("do multiversion setup"),
//             ]);
//         }
//         commands.append(&mut vec![
//             EvgCommand::from("setup jstestfuzz"),
//             fn_call_with_params("run jstestfuzz", params.build_jstestfuzz_params()),
//             fn_call_with_params("run generated tests", params.build_run_test_vars()),
//         ]);
//
//         EvgTask {
//             name: sub_task_name,
//             commands,
//             depends_on: Some(vec![TaskDependency {
//                 name: String::from("archive_dist_test_debug"),
//                 variant: None,
//             }]),
//             ..Default::default()
//         }
//     }
// }
