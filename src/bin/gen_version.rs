use std::collections::HashMap;
use std::path::Path;

use evg_api_rs::EvgClient;
use futures::future::join_all;
use mongo_task_gen::resmoke::ResmokeProxy;
use mongo_task_gen::resmoke_task_gen::{ResmokeGenParams, ResmokeGenService};
use mongo_task_gen::split_tasks::{SplitConfig, TaskSplitter};
use mongo_task_gen::task_history::get_task_history;
use mongo_task_gen::taskname::remove_gen_suffix_ref;
use mongo_task_gen::{
    find_suite_name, get_gen_task_var, get_project_config, is_fuzzer_task, is_task_generated,
};
use shrub_rs::models::project::EvgProject;
use shrub_rs::models::variant::BuildVariant;

#[tokio::main]
async fn main() {
    let evg_project_location = std::env::args().nth(1).expect("Expected project config");
    let evg_project = get_project_config(&Path::new(&evg_project_location)).unwrap();
    let evg_client = EvgClient::new().unwrap();
    let build_variant = "enterprise-rhel-80-64-bit-dynamic-required";

    let resmoke_gen_service = ResmokeGenService {};

    let mut task_def_map = HashMap::new();

    let mut history_futures = vec![];
    for task in &evg_project.tasks {
        if is_task_generated(task) && !is_fuzzer_task(task) {
            let task_name = remove_gen_suffix_ref(&task.name);
            let suite_name = find_suite_name(task);
            task_def_map.insert(task_name, task);
            history_futures.push(get_task_history(
                &evg_client,
                &task_name,
                build_variant,
                &suite_name,
            ))
        }
    }
    let mut shrub_project = EvgProject {
        ..Default::default()
    };

    let mut task_map = HashMap::with_capacity(evg_project.tasks.len());
    let task_histories = join_all(history_futures).await;
    for task_history in task_histories {
        let test_discovery = ResmokeProxy {};
        let task_splitter = TaskSplitter {
            test_discovery: test_discovery,
            split_config: SplitConfig { n_suites: 5 },
        };
        let gen_suite = task_splitter.split_task(&task_history);
        task_map.insert(gen_suite.task_name.clone(), gen_suite.clone());

        let task_def = task_def_map.get(&gen_suite.task_name.as_str()).unwrap();
        let resmoke_args = get_gen_task_var(&task_def, "resmoke_args").unwrap();
        let params = ResmokeGenParams {
            use_large_distro: false,
            large_distro_name: None,
            require_multiversion_setup: false,
            repeat_suites: 1,
            resmoke_args: resmoke_args.to_string(),
            config_location: Some("path/to/config".to_string()),
            resmoke_jobs_max: None,
        };
        resmoke_gen_service
            .generate_tasks(&gen_suite, &params)
            .into_iter()
            .for_each(|t| {
                shrub_project.tasks.push(t);
            });
    }

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

    for build_variant in &evg_project.buildvariants {
        let mut bv = BuildVariant {
            name: build_variant.name.clone(),
            display_tasks: Some(vec![]),
            ..Default::default()
        };
        let generated_tasks: Vec<&String> = build_variant
            .tasks
            .iter()
            .filter(|t| {
                let search_name = remove_gen_suffix_ref(&t.name);
                task_map.contains_key(search_name)
            })
            .map(|t| &t.name)
            .collect();

        generated_tasks.iter().for_each(|t| {
            let gen_suite = task_map.get(remove_gen_suffix_ref(&t.to_string())).unwrap();
            let mut execution_tasks = vec![];
            for sub_suite in &gen_suite.task_refs() {
                execution_tasks.push(sub_suite.name.clone());
                bv.tasks.push(sub_suite.clone());
            }

            bv.display_tasks
                .as_mut()
                .unwrap()
                .push(gen_suite.display_task());
        });
        shrub_project.buildvariants.push(bv);
    }

    let config = serde_yaml::to_string(&shrub_project).unwrap();
    println!("{}", config);
}
