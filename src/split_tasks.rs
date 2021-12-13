use crate::resmoke::{ResmokeProxy, TestDiscovery};
use crate::task_history::{get_test_name, TaskRuntimeHistory};
use shrub_rs::models::task::TaskRef;
use shrub_rs::models::variant::DisplayTask;
use std::cmp::min;

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
}

pub struct SplitConfig {
    pub n_suites: usize,
}

pub struct TaskSplitter {
    pub test_discovery: ResmokeProxy,
    pub split_config: SplitConfig,
}

impl TaskSplitter {
    pub fn split_task(&self, task_stats: &TaskRuntimeHistory) -> GeneratedSuite {
        let suite_name = &task_stats.suite_name;
        let test_list = self.test_discovery.discover_tests(suite_name);
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
                        name: format!("{}_{}", &task_stats.task_name, i),
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
                name: format!("{}_{}", &task_stats.task_name, i),
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
