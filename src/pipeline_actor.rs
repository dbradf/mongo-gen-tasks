use std::{collections::HashSet, path::PathBuf};

use shrub_rs::models::{task::EvgTask, variant::BuildVariant};
use tokio::sync::{mpsc, oneshot};

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
    ) -> Self {
        Self {
            receiver,
            generator_actor: GeneratorActorHandle::new(),
            write_config_actor: WriteConfigActorHandle::new(config_dir),
            task_splitter,
            build_variant: build_variant.clone(),
            config_location: config_dir.to_string(),
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
                let gen_suite = self.task_splitter.split_task(&task_history);
                let gen_params =
                    task_def_to_gen_params(&task_def, &self.build_variant, &self.config_location)
                        .await;
                self.write_config_actor.write_sub_suite(&gen_suite).await;
                self.generator_actor
                    .generate_resmoke(&gen_suite, gen_params)
                    .await;
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

#[derive(Clone)]
pub struct PipelineActorHandle {
    sender: mpsc::Sender<PipelineMessage>,
}

impl PipelineActorHandle {
    pub fn new(
        config_dir: &str,
        task_splitter: TaskSplitter,
        build_variant: &BuildVariant,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(64);
        let mut actor = PipelineActor::new(receiver, config_dir, task_splitter, build_variant);
        tokio::spawn(async move { actor.run().await });

        Self { sender }
    }

    pub async fn split_task(&self, task_history: TaskRuntimeHistory, task_def: &EvgTask) {
        let msg = PipelineMessage::SplitTask(task_history, task_def.clone());
        self.sender.send(msg).await.unwrap();
    }

    pub async fn gen_fuzzer(&self, params: FuzzerGenTaskParams) {
        let msg = PipelineMessage::GenFuzzer(params);
        self.sender.send(msg).await.unwrap();
    }

    pub async fn gen_resmoke(&self, gen_suite: GeneratedSuite, gen_params: ResmokeGenParams) {
        let msg = PipelineMessage::GenResmokeSuite(gen_suite, gen_params);
        self.sender.send(msg).await.unwrap();
    }

    pub async fn generator_tasks(&self, found_tasks: HashSet<String>) {
        let msg = PipelineMessage::GeneratorTasks(found_tasks);
        self.sender.send(msg).await.unwrap();
    }

    pub async fn write(&self, bv_name: &str, path: PathBuf) {
        let msg = PipelineMessage::Write(bv_name.to_string(), path);
        self.sender.send(msg).await.unwrap();
    }

    pub async fn flush(&self) {
        let (send, recv) = oneshot::channel();
        let msg = PipelineMessage::Flush(send);
        self.sender.send(msg).await.unwrap();
        recv.await.unwrap();
    }
}
