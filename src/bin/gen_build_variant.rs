use rayon::prelude::*;
use structopt::StructOpt;
use std::{
    collections::HashSet,
    error::Error,
    path::{Path, PathBuf},
};

use evg_api_rs::EvgClient;
use lazy_static::lazy_static;
use mongo_task_gen::{
    find_suite_name, get_gen_task_var, get_project_config, is_fuzzer_task, is_task_generated,
    resmoke::{generate_test_config, ResmokeProxy},
    resmoke_task_gen::{ResmokeGenParams, ResmokeGenService},
    split_tasks::{SplitConfig, TaskSplitter},
    task_history::{get_task_history, TaskRuntimeHistory},
    task_types::fuzzer_tasks::{generate_fuzzer_task, FuzzerGenTaskParams},
    taskname::remove_gen_suffix_ref,
};
use regex::Regex;
use serde::Deserialize;
use shrub_rs::models::{
    project::EvgProject,
    task::EvgTask,
    variant::{BuildVariant, DisplayTask},
};

lazy_static! {
    static ref EXPANSION_RE: Regex =
        Regex::new(r"\$\{(?P<id>[a-zA-Z0-9_]+)(\|(?P<default>.*))?}").unwrap();
}

/// Data extracted from Evergreen expansions.
#[derive(Debug, Deserialize, Clone)]
struct EvgExpansions {
    /// ID of build being run under.
    pub build_id: String,
    /// Build variant be generated.
    pub build_variant: String,
    /// Whether a patch build is being generated.
    pub is_patch: Option<String>,
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
    pub target_resmoke_time: Option<String>,
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
        if let Some(is_patch) = &self.is_patch {
            if is_patch == "true" {
                return self.max_sub_suite.unwrap_or(5);
            }
        }
        self.mainline_max_sub_suites.unwrap_or(1)
    }

    pub fn config_location(&self) -> String {
        let generated_task_name = remove_gen_suffix_ref(&self.task_name);
        format!(
            "{}/{}/generate_tasks/{}_gen-{}.tgz",
            self.build_variant, self.revision, generated_task_name, self.build_id
        )
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

fn task_def_to_fuzzer_params(
    task_def: &EvgTask,
    build_variant: &BuildVariant,
    config_location: &str,
) -> FuzzerGenTaskParams {
    let large_distro_name = build_variant
        .expansions
        .clone()
        .map(|e| e.get("large_distro_name").map(|d| d.to_string()))
        .flatten();
    let num_files = translate_run_var(
        get_gen_task_var(task_def, "num_files").unwrap(),
        build_variant,
    )
    .unwrap();

    FuzzerGenTaskParams {
        task_name: remove_gen_suffix_ref(&task_def.name).to_string(),
        variant: build_variant.name.to_string(),
        suite: find_suite_name(task_def).to_string(),
        num_files: num_files.parse().unwrap(),
        num_tasks: get_gen_task_var(task_def, "num_tasks")
            .unwrap()
            .parse()
            .unwrap(),
        resmoke_args: get_gen_task_var(task_def, "resmoke_args")
            .unwrap()
            .to_string(),
        npm_command: get_gen_task_var(task_def, "npm_command")
            .unwrap_or("jstestfuzz")
            .to_string(),
        jstestfuzz_vars: get_gen_task_var(task_def, "jstestfuzz_vars").map(|j| j.to_string()),
        continue_on_failure: get_gen_task_var(task_def, "continue_on_failure")
            .unwrap()
            .parse()
            .unwrap(),
        resmoke_jobs_max: get_gen_task_var(task_def, "resmoke_jobs_max")
            .unwrap()
            .parse()
            .unwrap(),
        should_shuffle: get_gen_task_var(task_def, "should_shuffle")
            .unwrap()
            .parse()
            .unwrap(),
        timeout_secs: get_gen_task_var(task_def, "timeout_secs")
            .unwrap()
            .parse()
            .unwrap(),
        require_multiversion_setup: Some(
            task_def
                .tags
                .clone()
                .unwrap_or(vec![])
                .contains(&"multiversion".to_string()),
        ),
        use_large_distro: get_gen_task_var(task_def, "use_large_distro")
            .map(|d| d.parse().unwrap()),
        large_distro_name: large_distro_name.clone(),
        config_location: config_location.to_string(),
    }
}

async fn task_def_to_split_params(
    evg_client: &EvgClient,
    task_def: &EvgTask,
    build_variant: &str,
) -> TaskRuntimeHistory {
    let task_name = remove_gen_suffix_ref(&task_def.name);
    get_task_history(
        evg_client,
        task_name,
        build_variant,
        find_suite_name(task_def),
    )
    .await
}

async fn task_def_to_gen_params(
    task_def: &EvgTask,
    build_variant: &BuildVariant,
    config_location: &str,
) -> ResmokeGenParams {
    let resmoke_args = get_gen_task_var(&task_def, "resmoke_args").unwrap();
    ResmokeGenParams {
        use_large_distro: get_gen_task_var(task_def, "use_large_distro")
            .map(|d| d == "true")
            .unwrap_or(false),
        large_distro_name: build_variant
            .expansions
            .as_ref()
            .map(|e| e.get("large_distro_name").map(|d| d.to_string()))
            .flatten(),
        require_multiversion_setup: false,
        repeat_suites: 1,
        resmoke_args: resmoke_args.to_string(),
        config_location: Some(config_location.to_string()),
        resmoke_jobs_max: None,
    }
}

#[derive(Debug, StructOpt)]
struct Opt {
    #[structopt(long, parse(from_os_str))]
    evg_project_location: PathBuf,

    #[structopt(long, parse(from_os_str))]
    expansion_file: PathBuf,

    #[structopt(long, parse(from_os_str))]
    evg_auth_file: PathBuf,
}

#[tokio::main]
async fn main() {
    let opt = Opt::from_args();
    let evg_project_location = opt.evg_project_location;
    let evg_project = get_project_config(&evg_project_location).unwrap();
    let expansion_file = opt.expansion_file;
    let evg_expansions = EvgExpansions::from_yaml_file(Path::new(&expansion_file)).unwrap();

    let evg_client = EvgClient::from_file(&opt.evg_auth_file).unwrap();

    let task_map = evg_project.task_def_map();
    let bv_map = evg_project.build_variant_map();
    let build_variant = bv_map.get(&evg_expansions.build_variant).unwrap();
    let config_location = &evg_expansions.config_location();
    let resmoke_gen_service = ResmokeGenService {};

    let mut found_tasks = HashSet::new();
    let mut gen_task_def = vec![];
    let mut gen_task_specs = vec![];
    let mut display_tasks = vec![];

    let config_dir = "generated_resmoke_config";
    std::fs::create_dir_all(config_dir).unwrap();

    for task in &build_variant.tasks {
        if let Some(task_def) = task_map.get(&task.name) {
            if is_task_generated(task_def) {
                found_tasks.insert(task_def.name.clone());
                if is_fuzzer_task(task_def) {
                    let params =
                        task_def_to_fuzzer_params(task_def, build_variant, &config_location);
                    let generated_task = generate_fuzzer_task(&params);
                    gen_task_def.extend(generated_task.sub_tasks.clone());
                    gen_task_specs.extend(generated_task.build_task_ref());
                    display_tasks.push(generated_task.build_display_task());
                } else {
                    let test_discovery = ResmokeProxy {};
                    let task_history = task_def_to_split_params(
                        &evg_client,
                        task_def,
                        &evg_expansions.build_variant,
                    )
                    .await;
                    let task_splitter = TaskSplitter {
                        test_discovery: Box::new(test_discovery),
                        split_config: SplitConfig {
                            n_suites: evg_expansions.get_max_sub_suites(),
                        },
                    };
                    let gen_suite = task_splitter.split_task(&task_history);
                    let gen_params =
                        task_def_to_gen_params(task_def, &build_variant, &config_location).await;
                    let all_tests: Vec<String> = gen_suite
                        .sub_suites
                        .iter()
                        .map(|s| s.test_list.clone())
                        .flatten()
                        .collect();

                    gen_suite.sub_suites.par_iter().for_each(|s| {
                        let config =
                            generate_test_config(&gen_suite.suite_name, &s.test_list, None);
                        let mut path = PathBuf::from(config_dir);
                        path.push(format!("{}.yml", s.name));

                        std::fs::write(path, config).unwrap();
                    });
                    let misc_config =
                        generate_test_config(&gen_suite.suite_name, &vec![], Some(&all_tests));
                    let mut path = PathBuf::from(config_dir);
                    path.push(format!("{}_misc.yml", gen_suite.task_name));
                    std::fs::write(path, misc_config).unwrap();

                    resmoke_gen_service
                        .generate_tasks(&gen_suite, &gen_params)
                        .into_iter()
                        .for_each(|t| {
                            gen_task_def.push(t.clone());
                            gen_task_specs.push(t.get_reference(None, Some(false)));
                        });
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

    // println!("{}", serde_json::to_string(&gen_evg_project).unwrap());
    println!("{}", serde_yaml::to_string(&gen_evg_project).unwrap());
}
