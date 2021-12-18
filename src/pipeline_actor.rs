use std::{collections::HashSet, path::PathBuf, time::Instant};

use shrub_rs::models::{task::EvgTask, variant::BuildVariant};
use tokio::sync::{mpsc, oneshot};
use tracing::{event, Level};

use crate::{
    gen_actor::GeneratorActorHandle,
    get_gen_task_var,
    resmoke_task_gen::ResmokeGenParams,
    split_tasks::{GeneratedSuite, TaskSplitter},
    task_history::TaskRuntimeHistory,
    task_types::fuzzer_tasks::FuzzerGenTaskParams,
    write_config::WriteConfigActorHandle,
};

async fn task_def_to_gen_params(
    task_def: &EvgTask,
    build_variant: &BuildVariant,
    config_location: &str,
) -> ResmokeGenParams {
    let resmoke_args = get_gen_task_var(&task_def, "resmoke_args").unwrap_or("");
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

#[derive(Debug)]
enum PipelineMessage {
    SplitTask(TaskRuntimeHistory, EvgTask),
    GenFuzzer(FuzzerGenTaskParams),
    GenResmokeSuite(GeneratedSuite, ResmokeGenParams),
    GeneratorTasks(HashSet<String>),
    Write(String, PathBuf),
    Flush(oneshot::Sender<()>),
}

struct PipelineActor {
    receiver: mpsc::Receiver<PipelineMessage>,
    generator_actor: GeneratorActorHandle,
    write_config_actor: WriteConfigActorHandle,
    task_splitter: TaskSplitter,
    build_variant: BuildVariant,
    config_location: String,
}

impl PipelineActor {
    fn new(
        receiver: mpsc::Receiver<PipelineMessage>,
        config_dir: &str,
        task_splitter: TaskSplitter,
        build_variant: &BuildVariant,
        config_location: &str,
    ) -> Self {
        Self {
            receiver,
            generator_actor: GeneratorActorHandle::new(),
            write_config_actor: WriteConfigActorHandle::new(config_dir),
            task_splitter,
            build_variant: build_variant.clone(),
            config_location: config_location.to_string(),
        }
    }

    async fn run(&mut self) {
        while let Some(msg) = self.receiver.recv().await {
            self.handle_message(msg).await;
        }
    }

    async fn handle_message(&mut self, msg: PipelineMessage) {
        match msg {
            PipelineMessage::SplitTask(task_history, task_def) => {
                let task_name = task_def.name.as_str();
                event!(Level::INFO, task_name, "Splitting Task");
                let start = Instant::now();
                let gen_suite = self.task_splitter.split_task(&task_history);
                event!(
                    Level::INFO,
                    task_name,
                    duration_ms = start.elapsed().as_millis() as u64,
                    "Split finished"
                );
                let gen_params =
                    task_def_to_gen_params(&task_def, &self.build_variant, &self.config_location)
                        .await;
                let start = Instant::now();
                self.write_config_actor.write_sub_suite(&gen_suite).await;
                event!(
                    Level::INFO,
                    task_name,
                    duration_ms = start.elapsed().as_millis() as u64,
                    "Write config finished"
                );
                let start = Instant::now();
                self.generator_actor
                    .generate_resmoke(&gen_suite, gen_params)
                    .await;
                event!(
                    Level::INFO,
                    task_name,
                    duration_ms = start.elapsed().as_millis() as u64,
                    "Gen config finished"
                );
            }
            PipelineMessage::GenFuzzer(fuzzer_params) => {
                self.generator_actor.generate_fuzzer(fuzzer_params).await;
            }
            PipelineMessage::GenResmokeSuite(gen_suite, gen_params) => {
                self.write_config_actor.write_sub_suite(&gen_suite).await;
                self.generator_actor
                    .generate_resmoke(&gen_suite, gen_params)
                    .await;
            }
            PipelineMessage::GeneratorTasks(found_tasks) => {
                self.generator_actor.add_generator_tasks(found_tasks).await;
            }
            PipelineMessage::Write(bv_name, path) => {
                self.generator_actor.write(&bv_name, path).await;
            }
            PipelineMessage::Flush(sender) => {
                sender.send(()).unwrap();
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct PipelineActorHandle {
    senders: Vec<mpsc::Sender<PipelineMessage>>,
    index: usize,
}

impl PipelineActorHandle {
    pub fn new(
        config_dir: &str,
        task_splitter: TaskSplitter,
        build_variant: &BuildVariant,
        config_location: &str,
    ) -> Self {
        let count = 32;

        let senders_and_receivers: Vec<(
            mpsc::Sender<PipelineMessage>,
            mpsc::Receiver<PipelineMessage>,
        )> = (0..count).map(|_| mpsc::channel(32)).collect();
        let mut senders = vec![];
        senders_and_receivers
            .into_iter()
            .for_each(|(sender, receiver)| {
                senders.push(sender);
                let mut actor = PipelineActor::new(
                    receiver,
                    config_dir,
                    task_splitter.clone(),
                    build_variant,
                    config_location,
                );
                tokio::spawn(async move { actor.run().await });
            });

        Self { senders, index: 0 }
    }

    async fn round_robbin(&mut self, msg: PipelineMessage) {
        let next = self.index;
        self.index = (next + 1) % self.senders.len();
        self.senders[next].send(msg).await.unwrap();
    }

    pub async fn split_task(&mut self, task_history: TaskRuntimeHistory, task_def: &EvgTask) {
        let msg = PipelineMessage::SplitTask(task_history, task_def.clone());
        self.round_robbin(msg).await;
    }

    pub async fn gen_fuzzer(&mut self, params: FuzzerGenTaskParams) {
        let msg = PipelineMessage::GenFuzzer(params);
        self.round_robbin(msg).await;
    }

    pub async fn gen_resmoke(&mut self, gen_suite: GeneratedSuite, gen_params: ResmokeGenParams) {
        let msg = PipelineMessage::GenResmokeSuite(gen_suite, gen_params);
        self.round_robbin(msg).await;
    }

    pub async fn generator_tasks(&mut self, found_tasks: HashSet<String>) {
        let msg = PipelineMessage::GeneratorTasks(found_tasks);
        self.round_robbin(msg).await;
    }

    pub async fn write(&mut self, bv_name: &str, path: PathBuf) {
        let msg = PipelineMessage::Write(bv_name.to_string(), path);
        self.round_robbin(msg).await;
    }

    pub async fn flush(&mut self) {
        for actor in &self.senders {
            let (send, recv) = oneshot::channel();
            let msg = PipelineMessage::Flush(send);
            actor.send(msg).await.unwrap();
            recv.await.unwrap();
        }
    }
}
