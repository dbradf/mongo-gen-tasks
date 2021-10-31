use crate::resmoke::TestDiscovery;
use crate::task_history::{get_test_name, TaskRuntimeHistory, TestRuntimeHistory};
use std::cmp::min;

#[derive(Debug, Clone)]
pub struct SubSuite {
    test_list: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GeneratedSuite {
    task_name: String,
    sub_suites: Vec<SubSuite>,
}

pub struct SplitConfig {
    pub n_suites: usize,
}

pub struct TaskSplitter {
    pub test_discovery: Box<dyn TestDiscovery>,
    pub split_config: SplitConfig,
}

impl TaskSplitter {
    pub fn split_task(&self, task_stats: &TaskRuntimeHistory) -> GeneratedSuite {
        let test_list = self.test_discovery.discover_tests(&task_stats.suite_name);
        let total_runtime = task_stats
            .test_map
            .iter()
            .fold(0.0, |init, (_, item)| init + item.average_runtime);

        let max_tasks = min(self.split_config.n_suites, test_list.len());

        let runtime_per_subtask = total_runtime / max_tasks as f64;
        let mut sub_suites = vec![];
        let mut running_tests = vec![];
        let mut running_runtime = 0.0;
        for test in test_list {
            let test_name = get_test_name(&test);
            if let Some(test_stats) = task_stats.test_map.get(&test_name) {
                if (running_runtime + test_stats.average_runtime > runtime_per_subtask)
                    && !running_tests.is_empty()
                    && sub_suites.len() < max_tasks
                {
                    sub_suites.push(SubSuite {
                        test_list: running_tests.clone(),
                    });
                    running_tests = vec![];
                    running_runtime = 0.0;
                }
                running_runtime += test_stats.average_runtime;
            }
            running_tests.push(test.clone());
        }
        if !running_tests.is_empty() {
            sub_suites.push(SubSuite {
                test_list: running_tests.clone(),
            });
        }

        GeneratedSuite {
            task_name: task_stats.task_name.clone(),
            sub_suites,
        }
    }
}
