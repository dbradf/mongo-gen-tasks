use std::path::PathBuf;

use rayon::prelude::*;
use tokio::sync::mpsc;

use crate::{
    resmoke::{read_suite_config, update_config},
    split_tasks::GeneratedSuite,
};

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
                let base_config = read_suite_config(&gen_suite.suite_name);

                gen_suite.sub_suites.par_iter().for_each(|s| {
                    let config = update_config(&base_config, &s.test_list, None);
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
                let misc_config = update_config(&base_config, &vec![], Some(&all_tests));
                let mut path = PathBuf::from(&self.config_dir);
                path.push(format!("{}_misc.yml", gen_suite.task_name));
                std::fs::write(path, misc_config).unwrap();
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct WriteConfigActorHandle {
    senders: Vec<mpsc::Sender<WriteConfigMessage>>,
    index: usize,
}

impl WriteConfigActorHandle {
    pub fn new(config_dir: &str) -> Self {
        let count = 10;
        let senders_and_revievers: Vec<(
            mpsc::Sender<WriteConfigMessage>,
            mpsc::Receiver<WriteConfigMessage>,
        )> = (0..count).map(|_| mpsc::channel(32)).collect();
        let mut senders = vec![];
        senders_and_revievers
            .into_iter()
            .for_each(|(sender, receiver)| {
                senders.push(sender);
                let mut actor = WriteConfigActor::new(receiver, config_dir.to_string());
                tokio::spawn(async move { actor.run().await });
            });

        Self { senders, index: 0 }
    }

    async fn round_robbin(&mut self, msg: WriteConfigMessage) {
        let next = self.index;
        self.index = (next + 1) % self.senders.len();
        self.senders[next].send(msg).await.unwrap();
    }

    pub async fn write_sub_suite(&mut self, gen_suite: &GeneratedSuite) {
        let msg = WriteConfigMessage::SuiteFiles(gen_suite.clone());
        self.round_robbin(msg).await;
    }
}
