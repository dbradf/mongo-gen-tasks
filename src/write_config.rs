use std::{path::PathBuf, time::Instant};

use rayon::prelude::*;
use tokio::sync::mpsc;

use crate::{resmoke::generate_test_config, split_tasks::GeneratedSuite};

#[derive(Debug, Clone)]
enum WriteConfigMessage {
    SuiteFiles(GeneratedSuite),
}

#[derive(Debug)]
struct WriteConfigActor {
    receiver: mpsc::Receiver<WriteConfigMessage>,
    config_dir: String,
}

impl WriteConfigActor {
    fn new(receiver: mpsc::Receiver<WriteConfigMessage>, config_dir: String) -> Self {
        WriteConfigActor {
            config_dir,
            receiver,
        }
    }

    async fn run(&mut self) {
        while let Some(msg) = self.receiver.recv().await {
            self.handle_message(msg);
        }
    }

    fn handle_message(&mut self, msg: WriteConfigMessage) {
        match msg {
            WriteConfigMessage::SuiteFiles(gen_suite) => {
                let now = Instant::now();

                gen_suite.sub_suites.par_iter().for_each(|s| {
                    let config = generate_test_config(&gen_suite.suite_name, &s.test_list, None);
                    let mut path = PathBuf::from(&self.config_dir);
                    path.push(format!("{}.yml", s.name));

                    std::fs::write(path, config).unwrap();
                });
                let all_tests: Vec<String> = gen_suite
                    .sub_suites
                    .iter()
                    .map(|s| s.test_list.clone())
                    .flatten()
                    .collect();
                let misc_config =
                    generate_test_config(&gen_suite.suite_name, &vec![], Some(&all_tests));
                let mut path = PathBuf::from(&self.config_dir);
                path.push(format!("{}_misc.yml", gen_suite.task_name));
                std::fs::write(path, misc_config).unwrap();

                println!(
                    "Created Files: {}: {}ms",
                    &gen_suite.suite_name,
                    now.elapsed().as_millis()
                );
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct WriteConfigActorHandle {
    sender: mpsc::Sender<WriteConfigMessage>,
}

impl WriteConfigActorHandle {
    pub fn new(config_dir: &str) -> Self {
        let (sender, receiver) = mpsc::channel(32);
        let mut actor = WriteConfigActor::new(receiver, config_dir.to_string());
        tokio::spawn(async move { actor.run().await });

        Self { sender }
    }

    pub async fn write_sub_suite(&self, gen_suite: &GeneratedSuite) {
        let msg = WriteConfigMessage::SuiteFiles(gen_suite.clone());
        self.sender.send(msg).await.unwrap();
    }
}
