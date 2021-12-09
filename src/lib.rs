use std::{error::Error, process::Command};

use shrub_rs::models::commands::EvgCommand::Function;
use shrub_rs::models::{project::EvgProject, task::EvgTask, commands::FunctionCall, params::ParamValue};
use taskname::remove_gen_suffix_ref;

pub mod resmoke;
pub mod resmoke_task_gen;
pub mod split_tasks;
pub mod task_history;
pub mod task_types;
pub mod taskname;
pub mod util;

pub struct SubSuite {
    pub index: usize,
    pub suite_name: String,
    pub test_list: Vec<String>,
}

pub struct GeneratedSuite {
    pub sub_suites: Vec<SubSuite>,
    pub build_variant: String,
    pub task_name: String,
    pub suite_name: String,
    pub filename: String,
    pub include_build_variant_in_name: bool,
}

pub fn get_project_config(location: &str) -> Result<EvgProject, Box<dyn Error>> {
    let evg_config_yaml = Command::new("evergreen")
        .args(&["evaluate", location])
        .output()?;
    EvgProject::from_yaml_str(std::str::from_utf8(&evg_config_yaml.stdout)?)
}

pub fn is_task_generated(task: &EvgTask) -> bool {
    task.commands.iter().any(|c| {
        if let Function(func) = c {
            if func.func == "generate resmoke tasks" {
                return true;
            }
        }
        false
    })
}

pub fn get_generate_resmoke_func(task: &EvgTask) -> Option<&FunctionCall> {
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
        .nth(0)
        .unwrap();
    if let Function(func) = command {
        return Some(func);
    }
    None
}

pub fn get_gen_task_var<'a>(task: &'a EvgTask, var: &str) -> Option<&'a str> {
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

pub fn find_suite_name(task: &EvgTask) -> &str {
    let suite = get_gen_task_var(task, "suite");
    if let Some(suite) = suite {
        suite
    } else {
        remove_gen_suffix_ref(&task.name)
    }
}

pub fn is_fuzzer_task(task: &EvgTask) -> bool {
    let is_jstestfuzz = get_gen_task_var(task, "is_jstestfuzz");
    if let Some(is_jstestfuzz) = is_jstestfuzz {
        is_jstestfuzz == "true"
    } else {
        false
    }
}