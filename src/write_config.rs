use std::path::PathBuf;

use rayon::prelude::*;
use tokio::sync::{mpsc, oneshot};

use crate::{resmoke::ResmokeSuiteConfig, split_tasks::GeneratedSuite};

#[derive(Debug)]
enum WriteConfigMessage {
    SuiteFiles(GeneratedSuite),
    Flush(oneshot::Sender<()>),
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
                let base_config = ResmokeSuiteConfig::read_suite_config(&gen_suite.suite_name);

                gen_suite.sub_suites.par_iter().for_each(|s| {
                    let config = base_config.update_config(&s.test_list, None);
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
                let misc_config = base_config.update_config(&vec![], Some(&all_tests));
                let mut path = PathBuf::from(&self.config_dir);
                path.push(format!("{}_misc.yml", gen_suite.task_name));
                std::fs::write(path, misc_config).unwrap();
            }
            WriteConfigMessage::Flush(sender) => sender.send(()).unwrap(),
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
        let count = 32;
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

    pub async fn flush(&mut self) {
        for sender in &self.senders {
            let (send, recv) = oneshot::channel();
            let msg = WriteConfigMessage::Flush(send);
            sender.send(msg).await.unwrap();
            recv.await.unwrap();
        }
    }
}
