use std::{
    collections::HashSet,
    error::Error,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};
use structopt::StructOpt;

use evg_api_rs::EvgClient;
use lazy_static::lazy_static;
use mongo_task_gen::{
    find_suite_name, get_gen_task_var, get_project_config, is_fuzzer_task, is_task_generated,
    resmoke::{MultiversionConfig, ResmokeProxy, ResmokeSuiteConfig, TestDiscovery},
    split_tasks::{ResmokeGenParams, SplitConfig, TaskSplitter, TaskSplitting},
    task_history::{TaskHistoryService, TaskHistoryServiceImpl},
    task_types::fuzzer_tasks::{FuzzerGenTaskParams, GenFuzzerService, GenFuzzerServiceImpl},
    taskname::remove_gen_suffix_ref,
    write_config::WriteConfigActorHandle,
};
use regex::Regex;
use serde::Deserialize;
use shrub_rs::models::{
    project::EvgProject,
    task::{EvgTask, TaskRef},
    variant::{BuildVariant, DisplayTask},
};
use tracing::{event, Level};
use tracing_subscriber::fmt::format;

lazy_static! {
    static ref EXPANSION_RE: Regex =
        Regex::new(r"\$\{(?P<id>[a-zA-Z0-9_]+)(\|(?P<default>.*))?}").unwrap();
}

const CONFIG_DIR: &str = "generated_resmoke_config";

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
    /// Read the expansions from a yaml file.
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

    let suite = find_suite_name(task_def).to_string();
    let suite_config = ResmokeSuiteConfig::read_suite_config(&suite);
    FuzzerGenTaskParams {
        task_name: remove_gen_suffix_ref(&task_def.name).to_string(),
        variant: build_variant.name.to_string(),
        suite,
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
                .unwrap_or_default()
                .contains(&"multiversion".to_string()),
        ),
        use_large_distro: get_gen_task_var(task_def, "use_large_distro")
            .map(|d| d.parse().unwrap()),
        large_distro_name,
        config_location: config_location.to_string(),
        suite_config,
    }
}

async fn task_def_to_gen_params(
    task_def: &EvgTask,
    build_variant: &BuildVariant,
    config_location: &str,
) -> ResmokeGenParams {
    let resmoke_args = get_gen_task_var(task_def, "resmoke_args").unwrap_or("");
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

#[derive(Debug, Clone)]
struct GeneratedConfig {
    pub gen_task_def: Vec<EvgTask>,
    pub gen_task_specs: Vec<TaskRef>,
    pub display_tasks: Vec<DisplayTask>,
}

impl GeneratedConfig {
    pub fn new() -> Self {
        Self {
            gen_task_def: vec![],
            gen_task_specs: vec![],
            display_tasks: vec![],
        }
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

struct Dependencies {
    pub evg_client: Arc<EvgClient>,
    pub test_discovery: Arc<dyn TestDiscovery>,
    pub task_splitter: Arc<dyn TaskSplitting>,
    pub task_history_service: Arc<dyn TaskHistoryService>,
    pub gen_fuzzer_service: Arc<dyn GenFuzzerService>,
    pub write_config_actor: Arc<tokio::sync::Mutex<WriteConfigActorHandle>>,
}

impl Dependencies {
    pub fn new(
        evg_expansions: &EvgExpansions,
        evg_auth_file: &Path,
        last_versions: &[String],
    ) -> Self {
        let evg_client = Arc::new(EvgClient::from_file(evg_auth_file).unwrap());
        let gen_fuzzer_service = Arc::new(GenFuzzerServiceImpl::new(last_versions));
        let test_discovery = Arc::new(ResmokeProxy {});
        let task_splitter = Arc::new(TaskSplitter {
            test_discovery: test_discovery.clone(),
            split_config: SplitConfig {
                n_suites: evg_expansions.get_max_sub_suites(),
            },
        });
        let task_history_service = Arc::new(TaskHistoryServiceImpl::new(evg_client.clone()));
        let write_config_actor = Arc::new(tokio::sync::Mutex::new(WriteConfigActorHandle::new(
            CONFIG_DIR,
        )));

        Self {
            evg_client,
            gen_fuzzer_service,
            test_discovery,
            task_splitter,
            task_history_service,
            write_config_actor,
        }
    }
}

#[tokio::main]
async fn main() {
    let opt = Opt::from_args();
    let format = format::json();
    let subscriber = tracing_subscriber::fmt().event_format(format).finish();

    tracing::subscriber::set_global_default(subscriber).unwrap();

    let evg_project_location = opt.evg_project_location;
    let evg_project = get_project_config(&evg_project_location).unwrap();
    let expansion_file = opt.expansion_file;
    let evg_expansions = EvgExpansions::from_yaml_file(Path::new(&expansion_file)).unwrap();

    let task_map = evg_project.task_def_map();
    let bv_map = evg_project.build_variant_map();
    let build_variant = bv_map.get(&evg_expansions.build_variant).unwrap();
    let config_location = &evg_expansions.config_location();

    let mut found_tasks = HashSet::new();

    std::fs::create_dir_all(CONFIG_DIR).unwrap();
    let multiversion_config = MultiversionConfig::from_resmoke();
    let deps = Arc::new(Dependencies::new(
        &evg_expansions,
        &opt.evg_auth_file,
        &multiversion_config.last_versions,
    ));

    let mut handles = vec![];
    let generated_config = Arc::new(Mutex::new(GeneratedConfig::new()));

    for task in &build_variant.tasks {
        if let Some(task_def) = task_map.get(&task.name) {
            let task_def = *task_def;
            if is_task_generated(task_def) {
                let gc = generated_config.clone();
                found_tasks.insert(task_def.name.clone());
                if is_fuzzer_task(task_def) {
                    let deps = deps.clone();
                    let gen_fuzzer = deps.gen_fuzzer_service.clone();
                    let params =
                        task_def_to_fuzzer_params(task_def, build_variant, config_location);

                    handles.push(tokio::spawn(async move {
                        let generated_task = gen_fuzzer.generate_fuzzer_task(&params).unwrap();
                        let mut gen_config = gc.lock().unwrap();
                        gen_config
                            .gen_task_specs
                            .extend(generated_task.build_task_ref());
                        gen_config
                            .display_tasks
                            .push(generated_task.build_display_task());
                        gen_config.gen_task_def.extend(generated_task.sub_tasks);
                    }));
                } else {
                    let deps = deps.clone();
                    let bv = *build_variant;
                    let config_loc = config_location.clone();
                    let write_actor = deps.write_config_actor.clone();
                    let task_name = task_def.name.to_string();
                    let suite_name = find_suite_name(task_def).to_string();
                    let bv_name = bv.name.to_string();
                    let gen_params = task_def_to_gen_params(task_def, bv, &config_loc).await;

                    handles.push(tokio::spawn(async move {
                        let task_name = task_name.as_str();
                        let task_history_service = deps.task_history_service.clone();
                        let short_task_name = remove_gen_suffix_ref(task_name);
                        let task_history = task_history_service
                            .get_task_history(short_task_name, &bv_name, &suite_name)
                            .await;
                        event!(Level::INFO, task_name, "Splitting Task");
                        let start = Instant::now();
                        let ts = deps.task_splitter.clone();
                        let gen_suite = ts.split_task(&task_history, &bv_name);
                        event!(
                            Level::INFO,
                            task_name,
                            duration_ms = start.elapsed().as_millis() as u64,
                            "Split finished"
                        );
                        let start = Instant::now();
                        {
                            let mut writer = write_actor.lock().await;
                            writer.write_sub_suite(&gen_suite).await;
                        }
                        event!(
                            Level::INFO,
                            task_name,
                            duration_ms = start.elapsed().as_millis() as u64,
                            "Write config finished"
                        );
                        let start = Instant::now();
                        let mut gen_config = gc.lock().unwrap();
                        gen_config
                            .gen_task_def
                            .extend(gen_suite.execution_tasks(&gen_params));
                        gen_config.gen_task_specs.extend(gen_suite.task_refs());
                        gen_config.display_tasks.push(gen_suite.display_task());

                        event!(
                            Level::INFO,
                            task_name,
                            duration_ms = start.elapsed().as_millis() as u64,
                            "Gen config finished"
                        );
                    }));
                }
            }
        }
    }

    for handle in handles {
        handle.await.unwrap();
    }

    let mut config_file = Path::new(CONFIG_DIR).to_path_buf();
    config_file.push(format!("{}.json", &build_variant.name));

    let gen_config = generated_config.lock().unwrap();

    let gen_build_variant = BuildVariant {
        name: build_variant.name.clone(),
        tasks: gen_config.gen_task_specs.clone(),
        display_tasks: Some(gen_config.display_tasks.clone()),
        ..Default::default()
    };

    let gen_evg_project = EvgProject {
        buildvariants: vec![gen_build_variant],
        tasks: gen_config.gen_task_def.clone(),
        ..Default::default()
    };

    std::fs::write(
        config_file,
        serde_json::to_string(&gen_evg_project).unwrap(),
    )
    .unwrap();
    let write_config_actor = deps.write_config_actor.clone();
    let mut write_actor = write_config_actor.lock().await;
    write_actor.flush().await;
}
