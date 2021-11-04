use chrono::{Duration, Utc};
use evg_api_rs::models::stats::EvgTestStatsRequest;
use evg_api_rs::EvgClient;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};

// const TASK_LEVEL_HOOKS: HashSet<&str> = vec!["CleanEveryN"].iter().collect();

#[derive(Debug, Clone)]
pub struct HookRuntimeHistory {
    pub test_name: String,
    pub hook_name: String,
    pub average_runtime: f64,
}

impl Display for HookRuntimeHistory {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{} : {}",
            self.test_name, self.hook_name, self.average_runtime
        )
    }
}

#[derive(Debug, Clone)]
pub struct TestRuntimeHistory {
    pub test_name: String,
    pub average_runtime: f64,
    pub hooks: Vec<HookRuntimeHistory>,
}

impl Display for TestRuntimeHistory {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}: {}", self.test_name, self.average_runtime)?;
        for hook in &self.hooks {
            writeln!(f, "- {}", hook)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct TaskRuntimeHistory {
    pub suite_name: String,
    pub task_name: String,
    pub test_map: HashMap<String, TestRuntimeHistory>,
}

pub async fn get_task_history(
    evg_client: &EvgClient,
    task: &str,
    variant: &str,
    suite: &str,
) -> TaskRuntimeHistory {
    let today = Utc::now();
    let lookback = Duration::days(14);
    let start_date = today - lookback;

    let request = EvgTestStatsRequest {
        after_date: start_date.format("%Y-%m-%d").to_string(),
        before_date: today.format("%Y-%m-%d").to_string(),
        group_num_days: 14,
        variants: variant.to_string(),
        tasks: task.to_string(),
        tests: None,
    };

    let stats = evg_client
        .get_test_stats("mongodb-mongo-master", &request)
        .await
        .unwrap();
    let mut hook_map: HashMap<String, Vec<HookRuntimeHistory>> = HashMap::new();
    for stat in &stats {
        if is_hook(&stat.test_file) {
            let test_name = hook_test_name(&stat.test_file);
            let hook_name = hook_hook_name(&stat.test_file);
            if let Some(v) = hook_map.get_mut(&test_name.to_string()) {
                v.push(HookRuntimeHistory {
                    test_name: test_name.to_string(),
                    hook_name: hook_name.to_string(),
                    average_runtime: stat.avg_duration_pass,
                });
            } else {
                hook_map.insert(
                    test_name.to_string(),
                    vec![HookRuntimeHistory {
                        test_name: test_name.to_string(),
                        hook_name: hook_name.to_string(),
                        average_runtime: stat.avg_duration_pass,
                    }],
                );
            }
        }
    }

    let mut test_map: HashMap<String, TestRuntimeHistory> = HashMap::new();
    for stat in &stats {
        if !is_hook(&stat.test_file) {
            let test_name = get_test_name(&stat.test_file);
            if let Some(v) = test_map.get_mut(&test_name) {
                v.test_name = stat.test_file.clone();
                v.average_runtime += stat.avg_duration_pass;
            } else {
                test_map.insert(
                    test_name.clone(),
                    TestRuntimeHistory {
                        test_name: stat.test_file.clone(),
                        average_runtime: stat.avg_duration_pass,
                        hooks: hook_map
                            .get(&test_name.to_string())
                            .unwrap_or(&vec![])
                            .clone(),
                    },
                );
            }
        }
    }

    // println!("{}: ", task);
    // for (task, test) in test_map {
    //     println!("{}", task);
    //     println!("{}", test);
    // }

    TaskRuntimeHistory {
        suite_name: suite.to_string(),
        task_name: task.to_string(),
        test_map,
    }
}

fn is_hook(test_file: &str) -> bool {
    test_file.contains(":")
}

fn hook_test_name(test_file: &str) -> &str {
    test_file.split(":").next().unwrap()
}

fn hook_hook_name(test_file: &str) -> &str {
    test_file.split(":").last().unwrap()
}

pub fn get_test_name(test_file: &str) -> String {
    let s = test_file.split("/");
    s.last().unwrap().trim_end_matches(".js").to_string()
}
