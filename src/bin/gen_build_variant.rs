use std::{path::Path, error::Error, collections::HashSet};

use lazy_static::lazy_static;
use evg_api_rs::EvgClient;
use mongo_task_gen::{get_project_config, taskname::remove_gen_suffix_ref, is_task_generated, is_fuzzer_task, task_types::fuzzer_tasks::{FuzzerGenTaskParams, generate_fuzzer_task}, get_gen_task_var, find_suite_name};
use regex::Regex;
use serde::Deserialize;
use shrub_rs::models::{task::EvgTask, variant::{BuildVariant, DisplayTask}, project::EvgProject};


lazy_static! {
    static ref EXPANSION_RE: Regex = Regex::new(r"\$\{(?P<id>[a-zA-Z0-9_]+)(\|(?P<default>.*))?}").unwrap();
}

/// Data extracted from Evergreen expansions.
#[derive(Debug, Deserialize, Clone)]
struct EvgExpansions {
    /// ID of build being run under.
    pub build_id: String,
    /// Build variant be generated.
    pub build_variant: String,
    /// Whether a patch build is being generated.
    pub is_patch: Option<bool>,
    /// Evergreen project being generated on.
    pub project: String,
    /// Max number of tests to add to each suite.
    pub max_tests_per_suite: Option<usize>,
    /// Maximum number of sub suites to generate in patch builds.
    pub max_sub_suite: Option<usize>,
    /// Maximum number of suites to generate on mainline builds
    pub mainline_max_sub_suites: Option<usize>,
    /// Repeat parameters to pass to resmoke.
    pub resmoke_repeat_suites: Option<usize>,
    /// Git revision being run against.
    pub revision: String,
    /// Name of task doing the generation.
    pub task_name: String,
    /// Target runtime for generated tasks.
    pub target_resmoke_time: Option<usize>,
    /// ID of task doing the generation.
    pub task_id: String,
}

impl EvgExpansions {
    pub fn from_yaml_file(path: &Path) -> Result<Self, Box<dyn Error>> {
        let contents = std::fs::read_to_string(path)?;
        Ok(serde_yaml::from_str(&contents)?)
    }

    /// Determine the max sub suites to split into.
    pub fn get_max_sub_suites(&self) -> usize {
        if let Some(is_patch) = self.is_patch {
            if is_patch {
                return self.max_sub_suite.unwrap_or(5);
            }
        }
        self.mainline_max_sub_suites.unwrap_or(1)
    }

    pub fn config_location(&self) -> String {
        let generated_task_name = remove_gen_suffix_ref(&self.task_name);
        format!("{}/{}/generate_tasks/{}_gen-{}.tgz", self.build_variant, self.revision, generated_task_name, self.build_id)
    }
}


fn translate_run_var(run_var: &str, build_variant: &BuildVariant) -> Option<String> {
    let expansion = EXPANSION_RE.captures(run_var);
    if let Some(captures) = expansion {
        let id = captures.name("id").unwrap();
        if let Some(value) = build_variant.expansions.clone().unwrap().get(id.as_str()) {
            Some(value.to_string())
        } else {
            captures.name("default").map(|d| d.as_str().to_string())
        }
    } else {
        Some(run_var.to_string())
    }
}

fn task_def_to_fuzzer_params(task_def: &EvgTask, build_variant: &BuildVariant, config_location: &str) -> FuzzerGenTaskParams {
    let large_distro_name = build_variant.expansions.clone().map(|e| e.get("large_distro_name").map(|d| d.to_string())).flatten();
    let num_files = translate_run_var(get_gen_task_var(task_def, "num_files").unwrap(), build_variant).unwrap();

    FuzzerGenTaskParams {
        task_name: remove_gen_suffix_ref(&task_def.name).to_string(),
        variant: build_variant.name.to_string(),
        suite: find_suite_name(task_def).to_string(),
        num_files: num_files.parse().unwrap(),
        num_tasks: get_gen_task_var(task_def, "num_tasks").unwrap().parse().unwrap(),
        resmoke_args: get_gen_task_var(task_def, "resmoke_args").unwrap().to_string(),
        npm_command: get_gen_task_var(task_def, "npm_command").unwrap_or("jstestfuzz").to_string(),
        jstestfuzz_vars: get_gen_task_var(task_def, "jstestfuzz_vars").map(|j| j.to_string()),
        continue_on_failure: get_gen_task_var(task_def, "continue_on_failure").unwrap().parse().unwrap(),
        resmoke_jobs_max: get_gen_task_var(task_def, "resmoke_jobs_max").unwrap().parse().unwrap(),
        should_shuffle: get_gen_task_var(task_def, "should_shuffle").unwrap().parse().unwrap(),
        timeout_secs: get_gen_task_var(task_def, "timeout_secs").unwrap().parse().unwrap(),
        require_multiversion_setup: Some(task_def.tags.clone().unwrap_or(vec![]).contains(&"multiversion".to_string())),
        use_large_distro: get_gen_task_var(task_def, "use_large_distro").map(|d| d.parse().unwrap()),
        large_distro_name: large_distro_name.clone(),
        config_location: config_location.to_string(),
    }
}

#[tokio::main]
async fn main() {
    let evg_project_location = std::env::args().nth(1).expect("Expected project config");
    let evg_project = get_project_config(&evg_project_location).unwrap();
    let expansion_file = std::env::args().nth(2).expect("Expected expansions file");
    let evg_expansions = EvgExpansions::from_yaml_file(Path::new(&expansion_file)).unwrap();

    // let evg_client = EvgClient::new().unwrap();

    let task_map = evg_project.task_def_map();
    let bv_map = evg_project.build_variant_map();
    let build_variant = bv_map.get(&evg_expansions.build_variant).unwrap();

    let mut found_tasks = HashSet::new();
    let mut gen_task_def = vec![];
    let mut gen_task_specs = vec![];
    let mut display_tasks = vec![];

    for task in &build_variant.tasks {
        if let Some(task_def) = task_map.get(&task.name) {
            if is_task_generated(task_def) {
                found_tasks.insert(task_def.name.clone());
                if is_fuzzer_task(task_def) {
                    let params = task_def_to_fuzzer_params(task_def, build_variant, &evg_expansions.config_location());
                    let generated_task = generate_fuzzer_task(&params);
                    gen_task_def.extend(generated_task.sub_tasks.clone());
                    gen_task_specs.extend(generated_task.build_task_ref());
                    display_tasks.push(generated_task.build_display_task());

                    // println!("{:?}", params);
                }
            }
        }
    }

    display_tasks.push(DisplayTask { 
        name: "generator_tasks".to_string(), 
        execution_tasks: found_tasks.into_iter().collect(),
    });

    let gen_build_variant = BuildVariant {
        name: build_variant.name.clone(),
        tasks: gen_task_specs,
        display_tasks: Some(display_tasks),
        ..Default::default()
    };

    let gen_evg_project = EvgProject {
        buildvariants: vec![gen_build_variant],
        tasks: gen_task_def,
        ..Default::default()
    };

    println!("{}", serde_json::to_string(&gen_evg_project).unwrap());
}
